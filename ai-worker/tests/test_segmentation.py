import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from PIL import Image

from keepframe_worker import segmentation


class _Config:
    id2label = {"0": "wall", "1": "sky", "2": "person", "3": "car, automobile"}


class _Model:
    config = _Config()


class _Cuda:
    def __init__(self, available):
        self.available = available

    def is_available(self):
        return self.available


class _Torch:
    def __init__(self, available):
        self.cuda = _Cuda(available)


class SegmentationProviderTests(unittest.TestCase):
    def test_provider_reports_pinned_model_and_unavailable_storage(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(segmentation, "MODEL_PATH", Path(directory)):
            state = segmentation.status()
            self.assertFalse(state["installed"])
            self.assertEqual(state["modelRevision"], segmentation.MODEL_REVISION)
            self.assertEqual(state["licence"], "Apache-2.0")
            with self.assertRaisesRegex(RuntimeError, "not installed"):
                segmentation.load_model()

    def test_subject_people_and_sky_capabilities_come_from_model_labels(self):
        model = _Model()
        self.assertEqual(segmentation._target_ids(model, "sky"), [1])
        self.assertEqual(segmentation._target_ids(model, "people"), [2])
        self.assertEqual(segmentation._target_ids(model, "subject"), [2, 3])

    def test_mask_encoding_is_compressed_and_preserves_a_central_region(self):
        values = bytes(255 if 4 <= x < 12 and 4 <= y < 12 else 0 for y in range(16) for x in range(16))
        png, populated = segmentation._coverage_png(values, 16, 16)
        decoded = Image.open(__import__("io").BytesIO(png)).convert("L")
        self.assertEqual(decoded.size, (16, 16))
        self.assertGreater(decoded.getpixel((8, 8)), 200)
        self.assertLess(decoded.getpixel((0, 0)), 10)
        self.assertGreater(populated, 0.15)
        self.assertLess(populated, 0.40)

    def test_unknown_category_fails_before_model_loading(self):
        with self.assertRaisesRegex(ValueError, "Unsupported"):
            segmentation.predict(Image.new("RGB", (16, 16)), "faces")

    def test_provider_prefers_cuda_but_has_cpu_and_forced_cpu_fallbacks(self):
        self.assertEqual(segmentation._preferred_device(_Torch(True)), "cuda")
        self.assertEqual(segmentation._preferred_device(_Torch(False)), "cpu")
        self.assertEqual(segmentation._preferred_device(_Torch(True), "cpu"), "cpu")
        self.assertEqual(segmentation._preferred_device(_Torch(True), force_cpu=True), "cpu")


if __name__ == "__main__":
    unittest.main()
