#!/usr/bin/env python3
"""Package only intended native products, preserving app permissions/symlinks."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import platform
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--platform", required=True, choices=["linux", "macos", "ios", "ios-simulator", "tvos", "tvos-simulator"])
parser.add_argument("--architecture", choices=["x86_64", "aarch64"])
args = parser.parse_args()
name = args.platform + ("-" + args.architecture if args.architecture else "")
output = ROOT / "ci-dist" / name
output.mkdir(parents=True, exist_ok=True)
epoch = int(subprocess.check_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT))


def normalize(info):
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = epoch
    return info


archive = output / f"atv-{name}.tar.gz"
if args.platform == "linux":
    if args.architecture != platform.machine():
        raise SystemExit("Linux package architecture does not match the native runner")
    binary = ROOT / "native/target/release/atv-desktop"
    if not binary.is_file():
        raise SystemExit("Build the release Linux host first")
    readme = ("ATV native Linux desktop. Requires SDL2, GStreamer >=1.20 and AVC/AAC plugins.\n"
              "Ubuntu: sudo apt-get install libsdl2-2.0-0 libgstreamer1.0-0 "
              "gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav\n"
              "Run ./atv-desktop; arrows change channels, Enter full screen, Escape back, Q quit.\n"
              "Build-only CI artifact; playback is not exercised against the live service in CI.\n").encode()
    with tarfile.open(archive, "w:gz") as package:
        package.add(binary, arcname=f"atv-{name}/atv-desktop", filter=normalize)
        info = normalize(tarfile.TarInfo(f"atv-{name}/README.txt"))
        info.size = len(readme)
        info.mode = 0o644
        package.addfile(info, io.BytesIO(readme))
    metadata = {"platform": "linux", "architecture": args.architecture,
                "libc": list(platform.libc_ver()), "runtimePlaybackVerifiedByCi": False}
else:
    report = ROOT / "apple/build" / (args.platform + "-build.json")
    metadata = json.loads(report.read_text())
    if metadata.get("buildVerified") is not True or metadata.get("signingRequested") is not False:
        raise SystemExit("Expected a successfully built unsigned Apple app")
    app = ROOT / metadata["appBundle"]
    if not app.is_dir() or app.suffix != ".app":
        raise SystemExit("Apple app bundle missing")
    with tarfile.open(archive, "w:gz") as package:
        package.add(app, arcname=app.name, filter=normalize)
    dependencies = ROOT / "apple/build" / (args.platform + "-dependencies.json")
    (output / "dependencies.json").write_bytes(dependencies.read_bytes())
metadata["sourceCommit"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
metadata["archive"] = archive.name
metadata["archiveSha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
(output / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n")
(output / "SHA256SUMS").write_text(metadata["archiveSha256"] + "  " + archive.name + "\n")
print(json.dumps({"platform": name, "archive": archive.name, "sha256": metadata["archiveSha256"]}))
