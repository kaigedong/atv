import importlib.util
from pathlib import Path
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location("signing", SCRIPTS / "ci-sign-android.py")
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class AndroidSigningTests(unittest.TestCase):
    def report(self, fingerprint="a" * 64, v1=True):
        return (f"Signer #1 certificate SHA-256 digest: {fingerprint}\n"
                f"Verified using v1 scheme (JAR signing): {str(v1).lower()}\n"
                "Verified using v2 scheme (APK Signature Scheme v2): true\n"
                "Verified using v3 scheme (APK Signature Scheme v3): true\n")

    def test_accepts_expected_signer_and_kitkat_signature(self):
        self.assertTrue(signing.signature_report(self.report(), "a" * 64)["schemes"]["v1"])

    def test_foreign_signer_is_rejected_even_with_valid_signatures(self):
        with self.assertRaisesRegex(ValueError, "pinned"):
            signing.signature_report(self.report(), "b" * 64)

    def test_missing_kitkat_signature_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "v1"):
            signing.signature_report(self.report(v1=False), "a" * 64)

    def test_multiple_signers_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "pinned"):
            signing.signature_report(self.report() + self.report(), "a" * 64)


if __name__ == "__main__":
    unittest.main()
