import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from android_version import build_version

spec = importlib.util.spec_from_file_location("release", SCRIPTS / "ci-publish-release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def fixtures(self, root):
        version = {"sourceCommit": "a" * 40, "versionCode": 1006, "versionName": "0.1.0-r1006", "tag": "v0.1.0-r1006"}
        fingerprint = "b" * 64
        android = root / "atv-android-signed"
        android.mkdir()
        artifacts = []
        for abi in release.ABIS:
            file = android / f"atv-android-{abi}-release.apk"
            file.write_bytes(b"fixture-verified-package")
            digest = release.digest(file)
            entry = {"abi": abi, "file": file.name, "minSdk": 19, "sha256": digest,
                     "signature": {"certificateSha256": fingerprint, "schemes": {v: True for v in ("v1", "v2", "v3")}}}
            artifacts.append(entry)
            file.with_suffix(".json").write_text(json.dumps({"apkSha256": digest, "forbiddenAssetsPresent": False,
                                                              "prohibitedDexReferencesPresent": False}))
        (android / "manifest.json").write_text(json.dumps({**version, "artifacts": artifacts}))
        for artifact, platform in release.NATIVE.items():
            directory = root / artifact
            directory.mkdir()
            file = directory / f"atv-{platform}.tar.gz"
            file.write_bytes(b"fixture-verified-native-archive")
            (directory / "manifest.json").write_text(json.dumps({
                "sourceCommit": version["sourceCommit"], "archive": file.name, "archiveSha256": release.digest(file),
                "buildVerified": True, "signingRequested": False}))
        return version, fingerprint

    def test_complete_release_contains_only_twelve_products_and_two_manifests(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            version, fingerprint = self.fixtures(root)
            # An unrelated private file must not enter the allowlisted release.
            (root / "atv-android-signed/fixture-private.p12").write_bytes(b"fixture-private-key")
            output = root / "output"
            manifest = release.prepare(root, output, version, fingerprint)
            self.assertEqual(len(manifest["assets"]), 12)
            self.assertEqual(len(list(output.iterdir())), 14)
            self.assertFalse((output / "fixture-private.p12").exists())

    def test_tampered_apk_is_rejected_before_staging(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            version, fingerprint = self.fixtures(root)
            next((root / "atv-android-signed").glob("*.apk")).write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "manifest"):
                release.prepare(root, root / "output", version, fingerprint)
            self.assertFalse((root / "output").exists())

    def test_other_commit_or_certificate_is_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            version, fingerprint = self.fixtures(root)
            with self.assertRaisesRegex(ValueError, "commit"):
                release.prepare(root, root / "output", {**version, "sourceCommit": "c" * 40}, fingerprint)
            with self.assertRaisesRegex(ValueError, "signature"):
                release.prepare(root, root / "output", version, "c" * 64)

    def test_missing_platform_is_rejected_before_staging(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            version, fingerprint = self.fixtures(root)
            next(root.glob("atv-linux-*/*.tar.gz")).unlink()
            with self.assertRaisesRegex(ValueError, "manifest"):
                release.prepare(root, root / "output", version, fingerprint)
            self.assertFalse((root / "output").exists())

    def test_remote_checksum_or_extra_asset_is_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            file = Path(name) / "app.apk"
            file.write_bytes(b"fixture-apk")
            metadata = {"assets": [{"name": file.name, "state": "uploaded", "digest": "sha256:" + release.digest(file)}]}
            self.assertFalse(release.check_remote_assets(metadata, [file], True))
            metadata["assets"][0]["digest"] = "sha256:" + "0" * 64
            with self.assertRaisesRegex(ValueError, "checksum"):
                release.check_remote_assets(metadata, [file], True)
            metadata["assets"][0]["name"] = "extra.apk"
            with self.assertRaisesRegex(ValueError, "asset set"):
                release.check_remote_assets(metadata, [file], True)

    def test_release_order_does_not_compare_hashes_or_strings(self):
        self.assertLess(release.release_code("v0.1.0-native.1"), release.release_code("v0.1.0-r1006"))
        self.assertLess(release.release_code("v0.1.0-r1009"), release.release_code("v0.1.0-r1010"))
        with self.assertRaises(ValueError):
            release.release_code("v9.0.0")

    def test_draft_without_public_tag_is_resumed_by_id(self):
        draft = {"id": 42, "tag_name": "v0.1.0-r1006", "draft": True}
        with patch.object(release, "api", side_effect=[None, [draft]]) as api:
            self.assertEqual(release.find_release(draft["tag_name"]), draft)
            self.assertEqual(api.call_args_list[1].args, ("releases?per_page=100",))

    def test_next_commit_increases_apk_version_and_rerun_keeps_version(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            def git(*args):
                return subprocess.run(["git", *args], cwd=root, capture_output=True, check=True)
            git("init")
            git("config", "user.name", "fixture")
            git("config", "user.email", "fixture@users.noreply.github.com")
            git("commit", "--allow-empty", "-m", "one")
            first = build_version(root)
            self.assertEqual(first, build_version(root))
            git("commit", "--allow-empty", "-m", "two")
            second = build_version(root)
            self.assertEqual(second["versionCode"], first["versionCode"] + 1)
            self.assertNotEqual(first["sourceCommit"], second["sourceCommit"])


if __name__ == "__main__":
    unittest.main()
