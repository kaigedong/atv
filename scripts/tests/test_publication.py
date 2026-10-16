import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[1] / "check-publication.py"
spec = importlib.util.spec_from_file_location("publication", MODULE)
publication = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publication)


class PublicationTests(unittest.TestCase):
    def test_secret_diagnostics_never_include_the_secret(self):
        token = "gh" + "p_" + "x" * 36
        findings = publication.inspect("source.rs", token.encode())
        self.assertTrue(findings)
        self.assertNotIn(token, str(findings))

    def test_credentials_and_signed_real_urls_are_rejected(self):
        for text in ['pass' + 'word = "sensitive-value"', 'gu' + 'id = "real-device-identity"',
                     'https://service.invalid/live?' + 'token=private-value']:
            self.assertTrue(publication.inspect("source.txt", text.encode()))

    def test_synthetic_auth_data_remains_publishable(self):
        text = 'guid = "fixture-guid-0"\nhttps://fixture.invalid/live?token=fixture'
        self.assertFalse(publication.inspect("tests/example.txt", text.encode()))

    def test_private_paths_and_commit_metadata_are_rejected(self):
        path = "/home/" + "private-user/tools"
        self.assertTrue(publication.inspect("source.txt", path.encode()))
        self.assertFalse(publication.github_email("private" + "@example.invalid"))
        self.assertTrue(publication.github_email("public@users.noreply.github.com"))

    def test_generated_files_and_changed_media_cannot_be_uploaded(self):
        for path in [".env", "nested/signing.jks", "artifacts/app.apk", "research/native-cmg/data.json"]:
            self.assertTrue(publication.inspect(path, b"data"))
        self.assertTrue(publication.inspect(publication.GENERATED_TS, b"changed media"))

    def test_history_catches_a_secret_deleted_from_current_tree(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            def git(*args):
                return subprocess.run(["git", "-C", name, *args], check=True, capture_output=True)
            git("init")
            git("config", "user.name", "fixture")
            git("config", "user.email", "fixture@users.noreply.github.com")
            (root / "notes.txt").write_text('pass' + 'word = "sensitive-value"')
            git("add", ".")
            git("commit", "-m", "fixture")
            (root / "notes.txt").write_text("safe")
            git("add", ".")
            git("commit", "-m", "remove")
            self.assertTrue(publication.audit(root)["passed"])
            self.assertFalse(publication.audit(root, history=True)["passed"])


if __name__ == "__main__":
    unittest.main()
