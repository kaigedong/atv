#!/usr/bin/env python3
"""Run checksum-pinned official audit binaries on the Linux x64 CI runner."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import urllib.request

TOOLS = {
    "gitleaks": ("gitleaks/gitleaks", "v8.30.1", "gitleaks_8.30.1_linux_x64.tar.gz",
                 "551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb"),
    "actionlint": ("rhysd/actionlint", "v1.7.12", "actionlint_1.7.12_linux_amd64.tar.gz",
                   "8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8"),
}
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("tool", choices=TOOLS)
args = parser.parse_args()
if platform.system() != "Linux" or platform.machine() != "x86_64":
    raise SystemExit("This helper is for the Linux x64 audit runner")
repo, version, filename, expected = TOOLS[args.tool]
output = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())) / "atv-ci-audit"
output.mkdir(exist_ok=True)
archive = output / filename
urllib.request.urlretrieve(f"https://github.com/{repo}/releases/download/{version}/{filename}", archive)
if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
    raise SystemExit("Audit tool checksum mismatch")
with tarfile.open(archive) as package:
    executable = output / args.tool
    executable.write_bytes(package.extractfile(args.tool).read())
executable.chmod(0o755)
root = Path(__file__).resolve().parent.parent
command = [str(executable)]
if args.tool == "gitleaks":
    command += ["git", ".", "--log-opts=HEAD", "--redact", "--no-banner"]
subprocess.run(command, cwd=root, check=True)
