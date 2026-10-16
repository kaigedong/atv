#!/usr/bin/env python3
"""Inspect a built Apple app's resources and direct Mach-O dependencies on macOS."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("app", type=Path)
parser.add_argument("--output", type=Path)
args = parser.parse_args()
macos = (args.app / "Contents/Info.plist").is_file()
contents = args.app / "Contents" if macos else args.app
info = plistlib.loads((contents / "Info.plist").read_bytes())
executable = contents / "MacOS" / info["CFBundleExecutable"] if macos else contents / info["CFBundleExecutable"]
for path in args.app.rglob("*"):
    if path.is_file() and path.suffix.lower() in {".js", ".cjs", ".mjs", ".html", ".wasm"}:
        raise ValueError("Apple app contains a browser/script asset")
dependencies = subprocess.check_output(["xcrun", "otool", "-L", str(executable)], text=True)
if any(marker in dependencies for marker in ["WebKit.framework", "JavaScriptCore.framework", "libquickjs", "libwasmtime"]):
    raise ValueError("Apple executable directly links a browser/script runtime")
with executable.open("rb") as stream:
    digest = hashlib.file_digest(stream, "sha256").hexdigest()
report = {"evidence": "Apple-bundle-and-direct-Mach-O-dependency-inspection",
          "executableSha256": digest, "dependencies": "\n".join(dependencies.splitlines()[1:]),
          "browserAssetsPresent": False, "directScriptRuntimePresent": False,
          "runtimePlaybackVerifiedByThisCheck": False}
if args.output:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
