#!/usr/bin/env python3
"""Check dependency direction and keep OS adapters out of the shared engine."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib
from toolchain import rust_environment

root = Path(__file__).resolve().parent.parent
crates = root / "native/crates"
shared = {"atv-core", "atv-auth", "atv-cmg", "atv-media", "atv-player"}
adapters = {"atv-android", "atv-desktop", "atv-apple"}
# The default Android project must be the native one, not a parallel legacy app.
for legacy in ["package.json", "package-lock.json", "web/index.html", "native-android/settings.gradle",
               "scripts/build-web.mjs", "scripts/prepare-player.mjs"]:
    assert not (root / legacy).exists(), f"legacy production entry point remains: {legacy}"
for path in (root / "app/src").rglob("*"):
    if not path.is_file():
        continue
    assert path.suffix.lower() not in {".js", ".cjs", ".mjs", ".wasm", ".html", ".htm"}, path
    if path.suffix in {".java", ".kt"}:
        text = path.read_text()
        for marker in ["android.webkit", "androidx.webkit", "evaluateJavascript", "javax.script", "org.mozilla.javascript"]:
            assert marker not in text, f"{path}: browser/script reference {marker}"
android_build = (root / "app/build.gradle").read_text()
for path in (root / "apple/Sources").rglob("*"):
    if not path.is_file():
        continue
    assert path.suffix.lower() not in {".js", ".cjs", ".mjs", ".wasm", ".html"}, path
    if path.suffix == ".swift":
        for marker in ["WKWebView", "import JavaScriptCore", "import WebKit", "evaluateJavaScript"]:
            assert marker not in path.read_text(), f"{path}: prohibited runtime {marker}"
for marker in ["androidx.webkit", "syncWebAssets", "generated/webAssets", "npm", "esbuild"]:
    assert marker not in android_build, f"default Android build still uses {marker}"
edges = {}
for name in sorted(shared | adapters):
    manifest = tomllib.loads((crates / name / "Cargo.toml").read_text())
    sections = [manifest.get("dependencies", {})]
    sections += [target.get("dependencies", {}) for target in manifest.get("target", {}).values()]
    dependencies = set().union(*(set(section) for section in sections))
    edges[name] = sorted(dependencies & (shared | adapters))
    if name in shared:
        assert not dependencies & (adapters | {"jni", "sdl2", "gstreamer", "gstreamer-app"}), name
        for file in (crates / name / "src").rglob("*.rs"):
            text = file.read_text()
            for symbol in ["ANativeWindow", "AMediaCodec", "AAudioStream", "JNIEnv", "jni::", "gstreamer::", "sdl2::"]:
                assert symbol not in text, f"{file}: platform symbol {symbol} in shared code"
assert not set(edges["atv-android"]) & adapters
assert not set(edges["atv-desktop"]) & adapters
assert not set(edges["atv-apple"]) & adapters
assert json.loads((crates / "atv-core/assets/channels.json").read_text()) == json.loads(
    (root / "app/src/main/assets/channels.json").read_text())

environment = rust_environment()
cargo = shutil.which("cargo", path=environment.get("PATH"))
if not cargo:
    raise SystemExit("Install Rust and put cargo on PATH, or set CARGO_HOME")
for package, target, forbidden in [
    ("atv-player", None, ["jni v", "sdl2 v", "gstreamer v", "atv-android v", "atv-desktop v"]),
    ("atv-android", "aarch64-linux-android", ["sdl2 v", "gstreamer v", "atv-desktop v"]),
    ("atv-apple", "aarch64-apple-tvos", ["jni v", "sdl2 v", "gstreamer v", "atv-android v", "atv-desktop v"]),
]:
    args = [cargo, "tree", "--locked", "--manifest-path", str(root / "native/Cargo.toml"), "-p", package, "--prefix", "none"]
    if target:
        args += ["--target", target]
    result = subprocess.run(args, capture_output=True, text=True, env=environment, check=True)
    for dependency in forbidden:
        assert dependency not in result.stdout, f"{package}: unexpected {dependency}"
print(json.dumps({"passed": True, "evidence": "dependency-direction-and-platform-boundary", "edges": edges,
                  "runtimePlaybackVerifiedByThisCheck": False}, indent=2))
