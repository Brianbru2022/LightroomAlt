from __future__ import annotations

import hashlib
import io
import os
import threading
import time
from pathlib import Path

from PIL import Image, ImageFilter

MODEL_ID = "microsoft/beit-base-finetuned-ade-640-640"
MODEL_REVISION = "a8b6f5ef4acb2ea55d882989deaa02d39401e2b2"
MODEL_SHA256 = "e0747360d190bd7c0f53d2fe3b2ed560c304d3eefef94574af9c7e93aaf8e7a9"
MODEL_BYTES = 899_902_905
PROVIDER = "Keepframe BEiT semantic segmentation"
PROVIDER_VERSION = "1.0.0"
LICENCE = "Apache-2.0"
SOURCE = f"https://huggingface.co/{MODEL_ID}/tree/{MODEL_REVISION}"
MODEL_ROOT = Path(os.environ.get("KEEPFRAME_MODEL_ROOT", r"D:\AI Models\Keepframe"))
MODEL_PATH = MODEL_ROOT / "segmentation" / "beit-base-ade20k-640"

_model = None
_processor = None
_device = "unloaded"
_loaded_ms = 0
_busy = False
_inference_lock = threading.RLock()
_verified = False

# ADE20K object-like labels from the model's own id2label metadata. Scene
# surfaces are deliberately excluded so Subject does not become wall/floor/sky.
SUBJECT_LABELS = {
    "person", "car", "boat", "bus", "truck", "airplane", "animal", "bicycle",
    "motorbike", "chair", "armchair", "sofa", "table", "desk", "bed", "bench",
    "cabinet", "wardrobe", "lamp", "television", "computer", "screen", "book",
    "bottle", "box", "bag", "basket", "flower", "plant", "sculpture", "vase",
    "toy", "fan", "clock", "flag", "food", "plate", "cup", "pot", "stove",
    "refrigerator", "microwave", "oven", "sink", "bathtub", "toilet", "signboard",
}


class SegmentationNoResult(RuntimeError):
    pass


def installed() -> bool:
    weight = MODEL_PATH / "pytorch_model.bin"
    marker = MODEL_PATH / "MODEL_SHA256.txt"
    return (
        weight.is_file()
        and weight.stat().st_size == MODEL_BYTES
        and marker.is_file()
        and marker.read_text(encoding="ascii").strip().lower() == MODEL_SHA256
        and (MODEL_PATH / "config.json").is_file()
        and (MODEL_PATH / "preprocessor_config.json").is_file()
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
        "source": SOURCE,
        "approximateBytes": MODEL_BYTES,
        "storagePath": str(MODEL_PATH),
        "executionProvider": _device,
        "loadedMs": _loaded_ms,
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


def load_model(force_cpu: bool = False):
    global _model, _processor, _device, _loaded_ms, _verified
    if _model is not None:
        return _model, _processor, _device
    if not installed():
        raise RuntimeError(f"The segmentation model is not installed at {MODEL_PATH}")
    if not _verified:
        digest = hashlib.sha256()
        with (MODEL_PATH / "pytorch_model.bin").open("rb") as weights:
            for chunk in iter(lambda: weights.read(8 * 1024 * 1024), b""):
                digest.update(chunk)
        if digest.hexdigest() != MODEL_SHA256:
            raise RuntimeError("The segmentation model failed SHA-256 verification")
        _verified = True
    os.environ.setdefault("HF_HOME", str(MODEL_ROOT / "huggingface"))
    os.environ.setdefault("HF_HUB_OFFLINE", "1")
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
    import torch
    from transformers import AutoImageProcessor, BeitForSemanticSegmentation

    started = time.perf_counter()
    _processor = AutoImageProcessor.from_pretrained(MODEL_PATH, local_files_only=True)
    _model = BeitForSemanticSegmentation.from_pretrained(MODEL_PATH, local_files_only=True)
    requested = os.environ.get("KEEPFRAME_SEGMENTATION_DEVICE", "auto").lower()
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


def _preferred_device(torch_module, requested: str = "auto", force_cpu: bool = False) -> str:
    """Choose acceleration without making CUDA a requirement."""
    return "cuda" if not force_cpu and requested != "cpu" and torch_module.cuda.is_available() else "cpu"


def _normalise_label(value: str) -> str:
    return value.lower().split(",", 1)[0].strip().replace("-", " ")


def _target_ids(model, category: str) -> list[int]:
    labels = {int(key): _normalise_label(value) for key, value in model.config.id2label.items()}
    if category == "sky":
        return [key for key, value in labels.items() if value == "sky"]
    if category == "people":
        return [key for key, value in labels.items() if value in {"person", "people"}]
    return [key for key, value in labels.items() if value in SUBJECT_LABELS]


def _coverage_png(values: bytes, width: int, height: int) -> tuple[bytes, float]:
    coverage = Image.frombytes("L", (width, height), values)
    # Median cleanup removes isolated class noise. A sub-pixel Gaussian keeps
    # the model's soft boundary without globally softening the accepted mask.
    coverage = coverage.filter(ImageFilter.MedianFilter(3)).filter(ImageFilter.GaussianBlur(0.7))
    populated = sum(value > 24 for value in coverage.getdata()) / max(1, width * height)
    output = io.BytesIO()
    coverage.save(output, format="PNG", optimize=True)
    return output.getvalue(), populated


def _predict_locked(source: Image.Image, category: str) -> dict:
    if category not in {"subject", "people", "sky"}:
        raise ValueError("Unsupported intelligent mask category")
    import torch
    import torch.nn.functional as functional

    started = time.perf_counter()
    model, processor, device = load_model()
    load_complete = time.perf_counter()
    image = source.convert("RGB")
    inputs = processor(images=image, return_tensors="pt")
    preprocessing_complete = time.perf_counter()
    inputs = {key: value.to(next(model.parameters()).device) for key, value in inputs.items()}
    try:
        with torch.inference_mode():
            logits = model(**inputs).logits
    except RuntimeError as error:
        if next(model.parameters()).device.type != "cuda" or "out of memory" not in str(error).lower():
            raise
        unload()
        model, processor, _ = load_model(force_cpu=True)
        globals()["_device"] = "CPU fallback"
        inputs = processor(images=image, return_tensors="pt")
        with torch.inference_mode():
            logits = model(**inputs).logits
        device = "CPU fallback"
    inference_complete = time.perf_counter()
    logits = functional.interpolate(logits, size=(image.height, image.width), mode="bilinear", align_corners=False)
    probabilities = logits.softmax(dim=1)[0]
    labels = probabilities.argmax(dim=0)
    candidates = _target_ids(model, category)
    if not candidates:
        raise SegmentationNoResult(f"The model exposes no {category} class")
    if category == "subject":
        scored = []
        for identifier in candidates:
            selected = labels == identifier
            area = selected.float().mean().item()
            if area < 0.002 or area > 0.80:
                continue
            confidence = probabilities[identifier][selected].mean().item()
            ys, xs = selected.nonzero(as_tuple=True)
            centrality = 1.0 - min(1.0, abs(xs.float().mean().item() / image.width - .5) + abs(ys.float().mean().item() / image.height - .5))
            scored.append((area * (0.65 + 0.35 * centrality) * confidence, identifier))
        if not scored:
            raise SegmentationNoResult("No credible foreground subject was found")
        candidates = [max(scored)[1]]
    selected = torch.zeros_like(labels, dtype=torch.bool)
    coverage = torch.zeros_like(probabilities[0])
    for identifier in candidates:
        class_pixels = labels == identifier
        selected |= class_pixels
        coverage = torch.maximum(coverage, probabilities[identifier] * class_pixels)
    fraction = selected.float().mean().item()
    confidence = probabilities[candidates].amax(dim=0)[selected].mean().item() if selected.any() else 0.0
    # A scene model can label a featureless bright surface as sky. Reject an
    # effectively whole-frame sky prediction: for local adjustment it is less
    # useful than a global edit and is a high-value false-positive guard.
    if fraction < 0.002 or confidence < 0.30 or (category == "sky" and fraction > 0.95):
        raise SegmentationNoResult(f"No credible {category} region was found")
    coverage = ((coverage - 0.20) / 0.65).clamp(0, 1)
    png, populated = _coverage_png((coverage.mul(255).byte().cpu().numpy()).tobytes(), image.width, image.height)
    post_complete = time.perf_counter()
    return {
        "png": png,
        "width": image.width,
        "height": image.height,
        "confidence": confidence,
        "coverageFraction": populated,
        "executionProvider": device,
        "timings": {
            "loadMs": round((load_complete - started) * 1000),
            "preprocessMs": round((preprocessing_complete - load_complete) * 1000),
            "inferenceMs": round((inference_complete - preprocessing_complete) * 1000),
            "postprocessMs": round((post_complete - inference_complete) * 1000),
        },
        "checksum": hashlib.sha256(png).hexdigest(),
    }


def predict(source: Image.Image, category: str) -> dict:
    global _busy
    if category not in {"subject", "people", "sky"}:
        raise ValueError("Unsupported intelligent mask category")
    with _inference_lock:
        _busy = True
        try:
            return _predict_locked(source, category)
        finally:
            _busy = False
