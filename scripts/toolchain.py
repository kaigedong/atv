"""Portable, optional discovery of user-local build tools. No machine identity."""
import os
from pathlib import Path


def data_home(environment):
    return Path(environment.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))


def rust_environment():
    environment = os.environ.copy()
    local_rust = data_home(environment) / "atv-rust"
    if "CARGO_HOME" not in environment and (local_rust / "cargo/bin/cargo").is_file():
        environment["CARGO_HOME"] = str(local_rust / "cargo")
        environment.setdefault("RUSTUP_HOME", str(local_rust / "rustup"))
    if "CARGO_HOME" in environment:
        environment["PATH"] = str(Path(environment["CARGO_HOME"]) / "bin") + os.pathsep + environment.get("PATH", "")
    environment.setdefault("RUSTUP_TOOLCHAIN", environment.get("ATV_RUST_TOOLCHAIN", "1.99.1"))
    return environment


def android_sdk():
    for key in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        if os.environ.get(key):
            return Path(os.environ[key])
    candidates = [Path.home() / "Android/Sdk", Path.home() / "Library/Android/sdk",
                  data_home(os.environ) / "android-sdk"]
    if os.environ.get("LOCALAPPDATA"):
        candidates.append(Path(os.environ["LOCALAPPDATA"]) / "Android/Sdk")
    for candidate in candidates:
        if (candidate / "platform-tools").is_dir():
            return candidate
    raise RuntimeError("Set ANDROID_HOME to your installed Android SDK")
