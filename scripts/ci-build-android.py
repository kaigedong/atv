#!/usr/bin/env python3
"""Build and gate universal/single-ABI APKs in a clean CI checkout."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import re
from android_version import build_version
from toolchain import android_sdk

ROOT = Path(__file__).resolve().parent.parent
output = ROOT / "ci-dist/android"
output.mkdir(parents=True, exist_ok=True)
artifacts = []
version = build_version(ROOT)
for abi in ["universal", "armeabi-v7a", "arm64-v8a", "x86", "x86_64"]:
    command = [str(ROOT / "scripts/android-build.sh"), ":app:clean", "assembleDebug", "assembleRelease"]
    command += [f'-PatvVersionCode={version["versionCode"]}', f'-PatvVersionName={version["versionName"]}']
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
        badging = subprocess.check_output([str(android_sdk() / "build-tools/35.0.0/aapt2"),
                                           "dump", "badging", str(target)], text=True)
        package = re.search(r"^package: name='cn.bobo.atv.nativeplayer' versionCode='(\d+)' versionName='([^']+)'", badging, re.M)
        if not package or (int(package[1]), package[2]) != (version["versionCode"], version["versionName"]):
            raise SystemExit("Packaged APK version does not match its commit-derived release identity")
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
    **version,
    "artifacts": artifacts,
    "signing": "Debug uses an ephemeral runner key; release APKs are unsigned. No signing keys are uploaded."
}, indent=2) + "\n")
(output / "SHA256SUMS").write_text("".join(f'{a["sha256"]}  {a["file"]}\n' for a in artifacts))
