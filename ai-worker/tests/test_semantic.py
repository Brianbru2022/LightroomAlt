import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from keepframe_worker import semantic


class _Vector:
    def float(self): return self
    def norm(self, **_kwargs): return _Norm()
    def __truediv__(self, _other): return self
    def __getitem__(self, _index): return self
    def detach(self): return self
    def cpu(self): return self
    def tolist(self): return [1.0] + [0.0] * (semantic.DIMENSION - 1)


class _Norm:
    def clamp(self, **_kwargs): return 1.0


class SemanticProviderTests(unittest.TestCase):
    def test_provider_is_pinned_offline_and_unavailable_without_weights(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(semantic, "MODEL_PATH", Path(folder)):
            status = semantic.status()
            self.assertFalse(status["installed"])
            self.assertEqual(status["modelRevision"], "7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed")
            self.assertEqual(status["licence"], "Apache-2.0")
            self.assertEqual(status["embeddingDimensions"], 768)

    def test_corrupt_or_truncated_install_is_not_available(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(semantic, "MODEL_PATH", Path(folder)):
            model = Path(folder)
            for name in ("config.json", "preprocessor_config.json", "special_tokens_map.json",
                         "spiece.model", "tokenizer.json", "tokenizer_config.json"):
                (model / name).write_bytes(b"{}")
            (model / "model.safetensors").write_bytes(b"truncated")
            (model / "MODEL_SHA256.txt").write_text(semantic.MODEL_SHA256, encoding="ascii")
            self.assertFalse(semantic.installed())

    def test_load_failure_is_clear_and_does_not_claim_a_session(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(semantic, "MODEL_PATH", Path(folder)):
            with self.assertRaisesRegex(RuntimeError, "not installed"):
                semantic.load_model()
            self.assertFalse(semantic.status()["loaded"])

    def test_execution_provider_prefers_cuda_but_has_cpu_fallback(self):
        class Torch:
            class cuda:
                @staticmethod
                def is_available(): return True
        self.assertEqual(semantic._preferred_device(Torch), "cuda")
        self.assertEqual(semantic._preferred_device(Torch, requested="cpu"), "cpu")
        self.assertEqual(semantic._preferred_device(Torch, force_cpu=True), "cpu")

    def test_vector_normalisation_requires_exact_dimension(self):
        result = semantic._normalised(_Vector())
        self.assertEqual(len(result), 768)
        self.assertEqual(result[0], 1.0)

    def test_query_validation_fails_before_loading_model(self):
        with self.assertRaises(ValueError):
            semantic.embed_text("  ")


if __name__ == "__main__":
    unittest.main()
