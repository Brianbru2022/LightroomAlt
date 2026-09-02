import unittest

from keepframe_worker.schemas import normalise_payload


class SchemaTests(unittest.TestCase):
    def test_defaults_preserve_identity_and_requested_intent(self):
        result = normalise_payload({"observations": ["Fine scratches are visible."]}, "scratch_repair")
        self.assertEqual(result.suggested_intents, ["scratch_repair"])
        self.assertIn("identity_faces", result.preserve)

    def test_unknown_intent_is_rejected(self):
        with self.assertRaises(ValueError):
            normalise_payload({"observations": ["Soft"], "suggested_intents": ["replace_person"]}, "restoration")


if __name__ == "__main__":
    unittest.main()
