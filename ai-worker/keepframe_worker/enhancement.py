from __future__ import annotations

import hashlib
import io
import os
import threading
import time
from dataclasses import dataclass
from pathlib import Path

from PIL import Image


MODEL_ROOT = Path(os.environ.get("KEEPFRAME_MODEL_ROOT", r"D:\AI Models\Keepframe"))
MODEL_PATH = MODEL_ROOT / "enhancement"
PROVIDER = "Keepframe local restoration provider"
PROVIDER_VERSION = "1.0.0"


@dataclass(frozen=True)
class ModelSpec:
    operation: str
    model: str
    revision: str
    sha256: str
    size: int
    licence: str
    source: str
    filename: str
    scale: int


MODELS = {
    "denoise": ModelSpec(
        operation="denoise",
        model="SCUNet colour real PSNR",
        revision="SCUNet@52e440a80a655b01e0b41e9dd9bfe599bc11625e+KAIR-v1.0",
        sha256="fa78899ba2caec9d235a900e91d96c689da71c42029230c2028b00f09f809c2e",
        size=71_982_841,
        licence="Apache-2.0",
        source="https://github.com/cszn/KAIR/releases/download/v1.0/scunet_color_real_psnr.pth",
        filename="scunet_color_real_psnr.pth",
        scale=1,
    ),
    "super_resolution": ModelSpec(
        operation="super_resolution",
        model="Real-ESRGAN x4plus",
        revision="Real-ESRGAN@a4abfb2979a7bbff3f69f58f58ae324608821e27+v0.1.0",
        sha256="4fa0d38905f75ac06eb49a7951b426670021be3018265fd191d2125df9d682f1",
        size=67_040_989,
        licence="BSD-3-Clause",
        source="https://github.com/xinntao/Real-ESRGAN/releases/download/v0.1.0/RealESRGAN_x4plus.pth",
        filename="RealESRGAN_x4plus.pth",
        scale=4,
    ),
}

_models: dict[str, object] = {}
_devices: dict[str, str] = {}
_loaded_ms: dict[str, int] = {}
_verified: set[str] = set()
_lock = threading.RLock()
_busy = False


class EnhancementOutOfMemory(RuntimeError):
    pass


def _file(spec: ModelSpec) -> Path:
    return MODEL_PATH / spec.filename


def _digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as weights:
        for chunk in iter(lambda: weights.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def installed(operation: str) -> bool:
    spec = MODELS.get(operation)
    if spec is None:
        return False
    path = _file(spec)
    if not path.is_file() or path.stat().st_size != spec.size:
        return False
    if operation not in _verified:
        if _digest(path) != spec.sha256:
            return False
        _verified.add(operation)
    return True


def _cuda_memory(torch_module) -> tuple[int, int]:
    if not torch_module.cuda.is_available():
        return 0, 0
    free, total = torch_module.cuda.mem_get_info()
    return int(free), int(total)


def status() -> dict:
    try:
        import torch

        cuda = torch.cuda.is_available()
        free, total = _cuda_memory(torch)
        runtime = True
    except Exception:
        cuda = False
        free = total = 0
        runtime = False
    return {
        "runtimeAvailable": runtime,
        "busy": _busy,
        "cudaAvailable": cuda,
        "cudaFreeBytes": free,
        "cudaTotalBytes": total,
        "models": [
            {
                "operation": spec.operation,
                "installed": installed(spec.operation),
                "loaded": spec.operation in _models,
                "provider": PROVIDER,
                "providerVersion": PROVIDER_VERSION,
                "model": spec.model,
                "modelRevision": spec.revision,
                "modelSha256": spec.sha256,
                "licence": spec.licence,
                "source": spec.source,
                "approximateBytes": spec.size,
                "storagePath": str(_file(spec)),
                "executionProvider": _devices.get(spec.operation, "not loaded"),
                "loadedMs": _loaded_ms.get(spec.operation, 0),
                "supportedScales": [1] if spec.scale == 1 else [2, 4],
            }
            for spec in MODELS.values()
        ],
    }


def unload() -> None:
    _models.clear()
    _devices.clear()
    _loaded_ms.clear()
    try:
        import torch

        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:
        pass


def load_model(operation: str, force_cpu: bool = False):
    if operation not in MODELS:
        raise ValueError("Unsupported enhancement operation")
    if not installed(operation):
        raise RuntimeError(f"The {operation} model is not installed at {_file(MODELS[operation])}")
    import torch
    from spandrel import ModelLoader

    requested = os.environ.get("KEEPFRAME_ENHANCEMENT_DEVICE", "auto").lower()
    target = "cuda" if not force_cpu and requested != "cpu" and torch.cuda.is_available() else "cpu"
    cached = _models.get(operation)
    if cached is not None and next(cached.model.parameters()).device.type == target:
        return cached, _devices[operation]
    started = time.perf_counter()
    descriptor = ModelLoader().load_from_file(_file(MODELS[operation]))
    descriptor.model.eval().to(target)
    if target == "cuda" and descriptor.supports_half:
        descriptor.model.half()
        device = "CUDA FP16"
    else:
        descriptor.model.float()
        device = "CPU FP32"
    _models[operation] = descriptor
    _devices[operation] = device
    _loaded_ms[operation] = round((time.perf_counter() - started) * 1000)
    return descriptor, device


def _tensor_from_image(torch_module, image: Image.Image, device: str, half: bool):
    rgb = image.convert("RGB")
    # bytearray owns writable memory, avoiding PyTorch's warning for immutable bytes.
    values = torch_module.frombuffer(bytearray(rgb.tobytes()), dtype=torch_module.uint8)
    tensor = values.reshape(rgb.height, rgb.width, 3).permute(2, 0, 1).unsqueeze(0)
    tensor = tensor.to(device=device, dtype=torch_module.float16 if half else torch_module.float32)
    return tensor.div_(255.0)


def _reflect_to_multiple(torch_module, tensor, multiple: int = 8):
    import torch.nn.functional as functional

    height, width = tensor.shape[-2:]
    right = (-width) % multiple
    bottom = (-height) % multiple
    if not right and not bottom:
        return tensor, width, height
    # Reflect needs each pad to be smaller than its source dimension. Preview and
    # full tiles are validated at >= 32 px, so this remains well-defined.
    return functional.pad(tensor, (0, right, 0, bottom), mode="reflect"), width, height


def _image_from_tensor(torch_module, tensor) -> Image.Image:
    tensor = tensor.detach().float().clamp_(0, 1).mul_(255).round_().byte().cpu()[0]
    height, width = tensor.shape[-2:]
    pixels = tensor.permute(1, 2, 0).contiguous().numpy().tobytes()
    return Image.frombytes("RGB", (width, height), pixels)


def run_tile(image: Image.Image, operation: str, force_cpu: bool = False) -> dict:
    global _busy
    if operation not in MODELS:
        raise ValueError("Unsupported enhancement operation")
    if image.width < 32 or image.height < 32 or image.width > 2048 or image.height > 2048:
        raise ValueError("Enhancement tiles must be between 32 and 2048 pixels per side")
    with _lock:
        _busy = True
        started = time.perf_counter()
        try:
            import torch

            descriptor, provider = load_model(operation, force_cpu=force_cpu)
            loaded = time.perf_counter()
            target = next(descriptor.model.parameters()).device.type
            half = target == "cuda" and descriptor.supports_half
            tensor = _tensor_from_image(torch, image, target, half)
            tensor, source_width, source_height = _reflect_to_multiple(torch, tensor)
            prepared = time.perf_counter()
            if target == "cuda":
                torch.cuda.reset_peak_memory_stats()
            try:
                with torch.inference_mode():
                    output = descriptor(tensor)
            except RuntimeError as error:
                if "out of memory" not in str(error).lower():
                    raise
                if target == "cuda":
                    torch.cuda.empty_cache()
                raise EnhancementOutOfMemory("CUDA ran out of memory for this tile") from error
            inferred = time.perf_counter()
            scale = MODELS[operation].scale
            output = output[..., : source_height * scale, : source_width * scale]
            result = _image_from_tensor(torch, output)
            encoded = io.BytesIO()
            result.save(encoded, format="PNG", compress_level=4)
            completed = time.perf_counter()
            peak = int(torch.cuda.max_memory_allocated()) if target == "cuda" else 0
            return {
                "png": encoded.getvalue(),
                "width": result.width,
                "height": result.height,
                "scale": scale,
                "executionProvider": provider,
                "tilePeakBytes": peak,
                "timings": {
                    "loadMs": round((loaded - started) * 1000),
                    "preprocessMs": round((prepared - loaded) * 1000),
                    "inferenceMs": round((inferred - prepared) * 1000),
                    "postprocessMs": round((completed - inferred) * 1000),
                },
                "model": MODELS[operation].model,
                "modelRevision": MODELS[operation].revision,
                "modelSha256": MODELS[operation].sha256,
                "provider": PROVIDER,
                "providerVersion": PROVIDER_VERSION,
            }
        finally:
            _busy = False
