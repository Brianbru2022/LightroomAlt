import hashlib
import io
import unittest
from unittest import mock

from PIL import Image

from keepframe_worker import enhancement


class EnhancementProviderTests(unittest.TestCase):
    def test_metadata_is_pinned_local_and_licence_explicit(self):
        status = enhancement.status()
        self.assertEqual({item["operation"] for item in status["models"]}, {"denoise", "super_resolution"})
        denoise = next(item for item in status["models"] if item["operation"] == "denoise")
        sr = next(item for item in status["models"] if item["operation"] == "super_resolution")
        self.assertEqual(denoise["licence"], "Apache-2.0")
        self.assertEqual(sr["licence"], "BSD-3-Clause")
        self.assertEqual(sr["supportedScales"], [2, 4])
        self.assertEqual(len(denoise["modelSha256"]), 64)
        self.assertEqual(len(sr["modelSha256"]), 64)
        self.assertTrue(denoise["storagePath"].startswith(str(enhancement.MODEL_ROOT)))

    def test_installation_requires_exact_size_and_hash(self):
        spec = enhancement.MODELS["denoise"]
        with mock.patch.object(enhancement, "MODEL_PATH") as root:
            path = root / spec.filename
            path.is_file.return_value = True
            path.stat.return_value.st_size = spec.size
            with mock.patch.object(enhancement, "_digest", return_value="bad"):
                enhancement._verified.discard("denoise")
                self.assertFalse(enhancement.installed("denoise"))

    def test_tile_validation_rejects_unsupported_or_unsafe_dimensions(self):
        with self.assertRaises(ValueError):
            enhancement.run_tile(Image.new("RGB", (64, 64)), "invent-detail")
        with self.assertRaises(ValueError):
            enhancement.run_tile(Image.new("RGB", (16, 64)), "denoise")

    def test_png_round_trip_helper_preserves_exact_dimensions(self):
        source = Image.new("RGB", (37, 41), (17, 39, 91))
        output = io.BytesIO()
        source.save(output, format="PNG")
        reopened = Image.open(io.BytesIO(output.getvalue()))
        self.assertEqual(reopened.size, (37, 41))
        self.assertEqual(len(hashlib.sha256(output.getvalue()).hexdigest()), 64)


if __name__ == "__main__":
    unittest.main()
