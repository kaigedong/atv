#!/usr/bin/env python3
"""Select SDK Manager explicitly and install exact NDKs on the ephemeral runner."""
import os
from pathlib import Path
import subprocess
from toolchain import android_sdk

if os.environ.get("GITHUB_ACTIONS") != "true":
    raise SystemExit("This SDK cleanup/setup helper is only for disposable GitHub runners")
sdk = android_sdk().resolve()
manager = sdk / "cmdline-tools/latest/bin/sdkmanager"
if not manager.is_file():
    candidates = sorted(sdk.glob("cmdline-tools/*/bin/sdkmanager"))
    if not candidates:
        raise SystemExit("The runner needs Android Command Line Tools installed")
    manager = candidates[-1]
required_ndks = {"25.2.9519653", "29.0.14206865"}
# The hosted image also carries large unrelated NDKs. Remove only those SDK
# packages inside this disposable job, leaving room for the two required ones.
if (sdk / "ndk").is_dir():
    unused = ["ndk;" + p.name for p in (sdk / "ndk").iterdir()
              if p.is_dir() and p.name not in required_ndks]
    if unused:
        subprocess.run([str(manager), "--sdk_root=" + str(sdk), "--uninstall", *unused], check=True)
subprocess.run([str(manager), "--sdk_root=" + str(sdk), "platforms;android-36", "build-tools;35.0.0",
                *("ndk;" + version for version in sorted(required_ndks))],
               input="y\n" * 100, text=True, check=True)
# The image supplies ANDROID_NDK_HOME for a different NDK; override it explicitly.
with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as output:
    for key in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        output.write(f"{key}={sdk}\n")
    for key in ("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"):
        output.write(f"{key}={sdk / 'ndk/29.0.14206865'}\n")
