#!/usr/bin/env python3
"""Sign reviewed CI APKs with the persistent GitHub environment identity."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile

from toolchain import android_sdk

ROOT = Path(__file__).resolve().parent.parent
ABIS = ("universal", "armeabi-v7a", "arm64-v8a", "x86", "x86_64")


def signature_report(text, expected):
    fingerprints = re.findall(r"^Signer #\d+ certificate SHA-256 digest: ([0-9a-fA-F]{64})$", text, re.M)
    if len(fingerprints) != 1 or fingerprints[0].lower() != expected.lower():
        raise ValueError("APK signer differs from the pinned release certificate")
    schemes = {scheme: f"Verified using {scheme} scheme" in text and
               bool(re.search(rf"^Verified using {scheme} scheme .*: true$", text, re.M))
               for scheme in ("v1", "v2", "v3")}
    if not all(schemes.values()):
        raise ValueError("Release APK requires verified v1, v2 and v3 signatures")
    return {"certificateSha256": fingerprints[0].lower(), "schemes": schemes}


def checked(command, environment=None):
    result = subprocess.run(command, cwd=ROOT, env=environment, capture_output=True, text=True)
    if result.returncode:
        # Tool diagnostics are intentionally withheld from the secret-bearing step.
        raise RuntimeError(f"{Path(command[0]).name} failed; private signing diagnostics withheld")
    return result.stdout


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def stop_signing(*_):
    raise SystemExit(143)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=ROOT / "ci-dist/android")
    parser.add_argument("--output", type=Path, default=ROOT / "ci-dist/android-signed")
    args = parser.parse_args()
    if (os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("GITHUB_REPOSITORY") != "kaigedong/atv"
            or os.environ.get("GITHUB_REF") != "refs/heads/main"
            or os.environ.get("GITHUB_EVENT_NAME") not in {"push", "workflow_dispatch"}):
        raise ValueError("Release signing is restricted to the main-branch GitHub job")
    identity = json.loads((ROOT / "app/signing-certificate.json").read_text(encoding="utf-8"))
    manifest = json.loads((args.input / "manifest.json").read_text(encoding="utf-8"))
    commit = checked(["git", "rev-parse", "HEAD"]).strip()
    if manifest["sourceCommit"] != commit or commit != os.environ.get("GITHUB_SHA"):
        raise ValueError("Unsigned artifacts do not belong to this exact workflow commit")
    entries = {a["abi"]: a for a in manifest["artifacts"] if not a["debugSigned"]}
    if len(entries) != len(ABIS) or set(entries) != set(ABIS):
        raise ValueError("Expected all five unsigned release APK variants")
    tools = android_sdk() / "build-tools/35.0.0"
    apksigner, zipalign = str(tools / "apksigner"), str(tools / "zipalign")
    args.output.mkdir(parents=True, exist_ok=True)
    if any(args.output.iterdir()):
        raise ValueError("Signed output directory must be empty")
    environment = os.environ.copy()
    encoded = environment.pop("ATV_ANDROID_KEYSTORE_BASE64", "")
    if not encoded or not environment.get("ATV_ANDROID_KEYSTORE_PASSWORD"):
        raise ValueError("Persistent Android signing secrets are required; no fallback key is generated")
    # SIGTERM from job cancellation should also leave the temporary directory context.
    signal.signal(signal.SIGTERM, stop_signing)
    artifacts = []
    with tempfile.TemporaryDirectory(prefix="atv-android-sign-", dir=os.environ["RUNNER_TEMP"]) as directory:
        store = Path(directory) / "release.p12"
        with store.open("xb") as stream:
            os.chmod(store, 0o600)
            stream.write(base64.b64decode(encoded, validate=True))
        for abi in ABIS:
            entry = entries[abi]
            expected_name = f"atv-android-{abi}-release-unsigned.apk"
            if entry["file"] != expected_name or entry["minSdk"] != 19:
                raise ValueError("Unexpected release artifact identity")
            source = args.input / expected_name
            gate = json.loads(source.with_suffix(".json").read_text(encoding="utf-8"))
            if sha256(source) != entry["sha256"] or gate["apkSha256"] != entry["sha256"]:
                raise ValueError("Unsigned APK does not match its reviewed build manifest")
            checked([zipalign, "-c", "-P", "16", "4", str(source)])
            target = args.output / f"atv-android-{abi}-release.apk"
            checked([apksigner, "sign", "--ks", str(store), "--ks-type", identity["keystoreType"],
                     "--ks-key-alias", identity["alias"],
                     "--ks-pass", "env:ATV_ANDROID_KEYSTORE_PASSWORD",
                     "--key-pass", "env:ATV_ANDROID_KEYSTORE_PASSWORD",
                     "--min-sdk-version", "19", "--v1-signing-enabled", "true",
                     "--v2-signing-enabled", "true", "--v3-signing-enabled", "true",
                     "--v4-signing-enabled", "false", "--out", str(target), str(source)], environment)
            signatures = signature_report(checked([apksigner, "verify", "--verbose", "--print-certs",
                                          "--min-sdk-version", "19", str(target)], environment),
                                          identity["certificateSha256"])
            checked([zipalign, "-c", "-P", "16", "4", str(target)])
            checked([os.sys.executable, str(ROOT / "scripts/verify-native-apk.py"), str(target),
                     "--abis", ",".join(ABIS[1:]) if abi == "universal" else abi,
                     "--output", str(target.with_suffix(".json"))], environment)
            artifacts.append({"file": target.name, "abi": abi, "minSdk": 19, "sha256": sha256(target),
                              "signature": signatures, "runtimePlaybackVerifiedByCi": False})
    output = {"sourceCommit": commit, "versionCode": manifest["versionCode"],
              "versionName": manifest["versionName"], "tag": manifest["tag"], "artifacts": artifacts,
              "signing": "Persistent android-release GitHub environment key; no private key in artifacts."}
    (args.output / "manifest.json").write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    (args.output / "SHA256SUMS").write_text(
        "".join(f'{a["sha256"]}  {a["file"]}\n' for a in artifacts), encoding="utf-8")
    print(json.dumps({"signedApks": len(artifacts), "sourceCommit": commit,
                      "certificateSha256": identity["certificateSha256"], "v1Compatible": True}))


if __name__ == "__main__":
    main()
