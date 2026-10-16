#!/usr/bin/env python3
"""Build and gate universal/single-ABI APKs in a clean CI checkout."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
output = ROOT / "ci-dist/android"
output.mkdir(parents=True, exist_ok=True)
artifacts = []
for abi in ["universal", "armeabi-v7a", "arm64-v8a", "x86", "x86_64"]:
    command = [str(ROOT / "scripts/android-build.sh"), ":app:clean", "assembleDebug", "assembleRelease"]
    if abi == "universal":
        command.append("lintDebug")
    else:
        command.append("-PatvAbis=" + abi)
    subprocess.run(command, cwd=ROOT, check=True)
    for variant, filename in [("debug", "app-debug.apk"), ("release", "app-release-unsigned.apk")]:
        source = ROOT / "app/build/outputs/apk" / variant / filename
        suffix = "debug" if variant == "debug" else "release-unsigned"
        target = output / f"atv-android-{abi}-{suffix}.apk"
        shutil.copyfile(source, target)
        gate = ROOT / "app/build/reports/native" / (variant + ".json")
        report = json.loads(gate.read_text())
        digest = hashlib.sha256(target.read_bytes()).hexdigest()
        if digest != report["apkSha256"]:
            raise SystemExit("APK does not match its mandatory dependency gate")
        shutil.copyfile(gate, target.with_suffix(".json"))
        artifacts.append({"file": target.name, "sha256": digest, "abi": abi,
                          "minSdk": 19, "debugSigned": variant == "debug",
                          "runtimePlaybackVerifiedByCi": False})
(output / "manifest.json").write_text(json.dumps({
    "sourceCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
    "artifacts": artifacts,
    "signing": "Debug uses an ephemeral runner key; release APKs are unsigned. No signing keys are uploaded."
}, indent=2) + "\n")
(output / "SHA256SUMS").write_text("".join(f'{a["sha256"]}  {a["file"]}\n' for a in artifacts))
