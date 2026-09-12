from __future__ import annotations

import io
import base64
import json
import os
import re
from pathlib import Path

from fastapi import Depends, FastAPI, File, Form, Header, HTTPException, UploadFile
from PIL import Image

from .schemas import AnalysisResponse, normalise_payload
from . import segmentation

MODEL_ID = "Qwen/Qwen3-VL-8B-Instruct"
MODEL_ROOT = Path(os.environ.get("KEEPFRAME_MODEL_ROOT", r"D:\AI Models\Keepframe"))
TOKEN = os.environ.get("KEEPFRAME_WORKER_TOKEN", "")
MODEL_PATH = MODEL_ROOT / "Qwen3-VL-8B-Instruct"

app = FastAPI(title="Keepframe local analysis worker", docs_url=None, redoc_url=None)
_model = None
_processor = None


def authorised(x_keepframe_token: str = Header(default="")) -> None:
    if not TOKEN or x_keepframe_token != TOKEN:
        raise HTTPException(status_code=401, detail="Invalid Keepframe worker token")


def load_model():
    global _model, _processor
    if _model is not None:
        return _model, _processor
    if not MODEL_PATH.exists():
        raise HTTPException(status_code=503, detail=f"Analysis model is not installed at {MODEL_PATH}")
    os.environ.setdefault("HF_HOME", str(MODEL_ROOT / "huggingface"))
    os.environ.setdefault("HF_HUB_OFFLINE", "1")
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
    import torch
    from transformers import AutoModelForImageTextToText, AutoProcessor

    _processor = AutoProcessor.from_pretrained(MODEL_PATH, local_files_only=True)
    _model = AutoModelForImageTextToText.from_pretrained(
        MODEL_PATH,
        local_files_only=True,
        torch_dtype=torch.bfloat16,
        device_map="auto",
    )
    return _model, _processor


def extract_json(text: str) -> dict:
    match = re.search(r"\{.*\}", text, flags=re.DOTALL)
    if not match:
        raise ValueError("The analysis model returned no JSON object")
    return json.loads(match.group(0))


@app.get("/health", dependencies=[Depends(authorised)])
def health() -> dict:
    return {
        "ready": MODEL_PATH.exists(),
        "analysisReady": MODEL_PATH.exists(),
        "loaded": _model is not None,
        "model": MODEL_ID,
        "modelPath": str(MODEL_PATH),
        "offline": True,
        "segmentation": segmentation.status(),
    }


@app.post("/v1/analyse", response_model=AnalysisResponse, dependencies=[Depends(authorised)])
async def analyse(
    image: UploadFile = File(...),
    requested_intent: str = Form("restoration"),
    common_brief: str = Form(""),
) -> AnalysisResponse:
    data = await image.read()
    if not data:
        raise HTTPException(status_code=422, detail="No image was supplied")
    try:
        source = Image.open(io.BytesIO(data)).convert("RGB")
        model, processor = load_model()
        instruction = (
            "Analyse this photograph conservatively for a non-destructive photo editing workflow. "
            f"The requested intent is {requested_intent}. The user's brief is: {common_brief or 'none'}. "
            "Describe only visible, edit-relevant conditions. Do not identify people or infer sensitive facts. "
            "Return one JSON object with observations, suggested_intents, preserve and negative_constraints. "
            "Use only these intent values: restoration, scratch_repair, denoise, sharpen, upscale, "
            "lighting_correction, object_removal, sky_replacement, colourisation, custom. "
            "Use only these preserve values: identity_faces, composition, text, period_detail, skin_texture, "
            "grain, monochrome_tonality. Use British English."
        )
        messages = [{"role": "user", "content": [{"type": "image", "image": source}, {"type": "text", "text": instruction}]}]
        inputs = processor.apply_chat_template(messages, tokenize=True, add_generation_prompt=True, return_dict=True, return_tensors="pt").to(model.device)
        output_ids = model.generate(**inputs, max_new_tokens=520, do_sample=False)
        generated = output_ids[:, inputs.input_ids.shape[1]:]
        text = processor.batch_decode(generated, skip_special_tokens=True)[0]
        return normalise_payload(extract_json(text), requested_intent)
    except HTTPException:
        raise
    except Exception as exc:
        raise HTTPException(status_code=502, detail=f"Local analysis failed: {exc}") from exc


@app.post("/unload", dependencies=[Depends(authorised)])
def unload() -> dict:
    global _model, _processor
    _model = None
    _processor = None
    segmentation.unload()
    try:
        import torch
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:
        pass
    return {"unloaded": True}


@app.post("/v1/segment", dependencies=[Depends(authorised)])
def segment(image: UploadFile = File(...), category: str = Form(...)) -> dict:
    data = image.file.read()
    if not data:
        raise HTTPException(status_code=422, detail="No image was supplied")
    try:
        result = segmentation.predict(Image.open(io.BytesIO(data)), category)
        png = result.pop("png")
        return {
            **result,
            "coveragePng": base64.b64encode(png).decode("ascii"),
            "provider": segmentation.PROVIDER,
            "providerVersion": segmentation.PROVIDER_VERSION,
            "model": segmentation.MODEL_ID,
            "modelRevision": segmentation.MODEL_REVISION,
            "modelSha256": segmentation.MODEL_SHA256,
        }
    except segmentation.SegmentationNoResult as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    except ValueError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc
    except Exception as exc:
        raise HTTPException(status_code=502, detail=f"Local segmentation failed: {exc}") from exc
