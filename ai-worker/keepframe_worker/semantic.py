from __future__ import annotations

import hashlib
import os
import threading
import time
from pathlib import Path

from PIL import Image

MODEL_ID = "google/siglip-base-patch16-224"
MODEL_REVISION = "7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed"
MODEL_SHA256 = "421489ed67220ff3cf58fefe883271f5dd6fd1bca1f2d24157453b5cc8f82f88"
MODEL_BYTES = 812_672_320
DIMENSION = 768
INPUT_RESOLUTION = 224
PREPROCESSING_VERSION = 1
PROVIDER = "Keepframe local SigLIP semantic provider"
PROVIDER_VERSION = "1.0.0"
LICENCE = "Apache-2.0"
MODEL_ROOT = Path(os.environ.get("KEEPFRAME_MODEL_ROOT", r"D:\AI Models\Keepframe"))
MODEL_PATH = MODEL_ROOT / "semantic" / "siglip-base-patch16-224"

_model = None
_processor = None
_device = "unloaded"
_loaded_ms = 0
_busy = False
_verified = False
_lock = threading.RLock()


def installed() -> bool:
    required = (
        "config.json", "model.safetensors", "preprocessor_config.json",
        "special_tokens_map.json", "spiece.model", "tokenizer.json",
        "tokenizer_config.json", "MODEL_SHA256.txt",
    )
    return (
        all((MODEL_PATH / name).is_file() for name in required)
        and (MODEL_PATH / "model.safetensors").stat().st_size == MODEL_BYTES
        and (MODEL_PATH / "MODEL_SHA256.txt").read_text(encoding="ascii").strip().lower() == MODEL_SHA256
    )


def status() -> dict:
    return {
        "installed": installed(),
        "loaded": _model is not None,
        "busy": _busy,
        "provider": PROVIDER,
        "providerVersion": PROVIDER_VERSION,
        "model": MODEL_ID,
        "modelRevision": MODEL_REVISION,
        "modelSha256": MODEL_SHA256,
        "licence": LICENCE,
        "source": f"https://huggingface.co/{MODEL_ID}/tree/{MODEL_REVISION}",
        "approximateBytes": MODEL_BYTES,
        "storagePath": str(MODEL_PATH),
        "executionProvider": _device,
        "loadedMs": _loaded_ms,
        "inputResolution": INPUT_RESOLUTION,
        "embeddingDimensions": DIMENSION,
        "preprocessingVersion": PREPROCESSING_VERSION,
    }


def unload() -> None:
    global _model, _processor, _device, _loaded_ms, _verified
    _model = None
    _processor = None
    _device = "unloaded"
    _loaded_ms = 0
    _verified = False
    try:
        import torch
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:
        pass


def _preferred_device(torch_module, requested: str = "auto", force_cpu: bool = False) -> str:
    return "cuda" if not force_cpu and requested != "cpu" and torch_module.cuda.is_available() else "cpu"


def load_model(force_cpu: bool = False):
    global _model, _processor, _device, _loaded_ms, _verified
    if _model is not None:
        return _model, _processor, _device
    if not installed():
        raise RuntimeError(f"The semantic model is not installed at {MODEL_PATH}")
    if not _verified:
        digest = hashlib.sha256()
        with (MODEL_PATH / "model.safetensors").open("rb") as weights:
            for chunk in iter(lambda: weights.read(8 * 1024 * 1024), b""):
                digest.update(chunk)
        if digest.hexdigest() != MODEL_SHA256:
            raise RuntimeError("The semantic model failed SHA-256 verification")
        _verified = True
    os.environ.setdefault("HF_HOME", str(MODEL_ROOT / "huggingface"))
    os.environ.setdefault("HF_HUB_OFFLINE", "1")
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
    import torch
    from transformers import AutoProcessor, SiglipModel

    started = time.perf_counter()
    _processor = AutoProcessor.from_pretrained(MODEL_PATH, local_files_only=True)
    _model = SiglipModel.from_pretrained(MODEL_PATH, local_files_only=True)
    requested = os.environ.get("KEEPFRAME_SEMANTIC_DEVICE", "auto").lower()
    target = _preferred_device(torch, requested, force_cpu)
    try:
        _model.to(target)
        _device = "CUDA" if target == "cuda" else "CPU"
    except Exception:
        _model.to("cpu")
        _device = "CPU fallback"
    _model.eval()
    _loaded_ms = round((time.perf_counter() - started) * 1000)
    return _model, _processor, _device


def _normalised(values) -> list[float]:
    values = values.float()
    values = values / values.norm(p=2, dim=-1, keepdim=True).clamp(min=1e-12)
    result = values[0].detach().cpu().tolist()
    if len(result) != DIMENSION or not all(isinstance(value, float) for value in result):
        raise RuntimeError("The semantic provider returned an incompatible vector")
    return result


def embed_image(image: Image.Image) -> dict:
    global _busy, _device
    with _lock:
        _busy = True
        started = time.perf_counter()
        try:
            import torch
            model, processor, device = load_model()
            loaded = time.perf_counter()
            inputs = processor(images=image.convert("RGB"), return_tensors="pt")
            inputs = {key: value.to(next(model.parameters()).device) for key, value in inputs.items()}
            prepared = time.perf_counter()
            try:
                with torch.inference_mode():
                    vector = model.get_image_features(**inputs)
            except RuntimeError as error:
                if next(model.parameters()).device.type != "cuda" or "out of memory" not in str(error).lower():
                    raise
                unload()
                model, processor, _ = load_model(force_cpu=True)
                _device = "CPU fallback"
                inputs = processor(images=image.convert("RGB"), return_tensors="pt")
                inputs = {key: value.to(next(model.parameters()).device) for key, value in inputs.items()}
                with torch.inference_mode():
                    vector = model.get_image_features(**inputs)
                device = "CPU fallback"
            finished = time.perf_counter()
            return {
                "vector": _normalised(vector),
                "executionProvider": device,
                "timings": {
                    "loadMs": round((loaded - started) * 1000),
                    "preprocessMs": round((prepared - loaded) * 1000),
                    "inferenceMs": round((finished - prepared) * 1000),
                },
                **_provenance(),
            }
        finally:
            _busy = False


def embed_text(text: str) -> dict:
    global _busy
    query = text.strip()
    if not query or len(query) > 240:
        raise ValueError("Semantic query must contain between 1 and 240 characters")
    with _lock:
        _busy = True
        started = time.perf_counter()
        try:
            import torch
            model, processor, device = load_model()
            loaded = time.perf_counter()
            inputs = processor(text=[query], padding="max_length", truncation=True, return_tensors="pt")
            inputs = {key: value.to(next(model.parameters()).device) for key, value in inputs.items()}
            prepared = time.perf_counter()
            with torch.inference_mode():
                vector = model.get_text_features(**inputs)
            finished = time.perf_counter()
            return {
                "vector": _normalised(vector),
                "executionProvider": device,
                "timings": {
                    "loadMs": round((loaded - started) * 1000),
                    "preprocessMs": round((prepared - loaded) * 1000),
                    "inferenceMs": round((finished - prepared) * 1000),
                },
                **_provenance(),
            }
        finally:
            _busy = False


def _provenance() -> dict:
    return {
        "provider": PROVIDER,
        "providerVersion": PROVIDER_VERSION,
        "model": MODEL_ID,
        "modelRevision": MODEL_REVISION,
        "modelSha256": MODEL_SHA256,
        "dimensions": DIMENSION,
        "preprocessingVersion": PREPROCESSING_VERSION,
    }
