#!/usr/bin/env python3
"""Build Rust + Swift native macOS/iOS/tvOS apps on a Mac with Xcode and XcodeGen."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
PLATFORMS = {
    "macos": ("macosx", "ATV-macOS", "macOS", ["aarch64-apple-darwin", "x86_64-apple-darwin"]),
    "ios": ("iphoneos", "ATV-iOS", "iOS", ["aarch64-apple-ios"]),
    "ios-simulator": ("iphonesimulator", "ATV-iOS", "iOS Simulator", ["aarch64-apple-ios-sim", "x86_64-apple-ios"]),
    "tvos": ("appletvos", "ATV-tvOS", "tvOS", ["aarch64-apple-tvos"]),
    "tvos-simulator": ("appletvsimulator", "ATV-tvOS", "tvOS Simulator", ["aarch64-apple-tvos-sim"]),
}


def run(command, env, **kwargs):
    return subprocess.run(command, env=env, cwd=ROOT, check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=PLATFORMS, default="macos")
    parser.add_argument("--configuration", choices=["Debug", "Release"], default="Release")
    parser.add_argument("--team", help="Apple development team ID for device signing")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Apple apps require macOS + Xcode SDKs. Linux can test the shared core and C ABI only.")
    for tool in ("xcrun", "xcodebuild", "xcodegen", "cargo", "rustup"):
        if shutil.which(tool) is None:
            parser.error(f"Missing {tool}; see docs/apple-native.md")
    sdk, scheme, destination, targets = PLATFORMS[args.platform]
    env = os.environ.copy()
    env["RUSTUP_TOOLCHAIN"] = "1.99.1"
    env["MACOSX_DEPLOYMENT_TARGET"] = "13.0"
    env["IPHONEOS_DEPLOYMENT_TARGET"] = "16.0"
    env["TVOS_DEPLOYMENT_TARGET"] = "16.0"
    env["SDKROOT"] = subprocess.check_output(["xcrun", "--sdk", sdk, "--show-sdk-path"], text=True).strip()
    # Pin Cargo output to the workspace rather than inheriting a caller's cache path.
    env["CARGO_TARGET_DIR"] = str(ROOT / "native/target")
    libraries = []
    for target in targets:
        run(["rustup", "target", "add", target, "--toolchain", "1.99.1"], env)
        run(["cargo", "build", "--locked", "--manifest-path", str(ROOT / "native/Cargo.toml"),
             "-p", "atv-apple", "--release", "--target", target], env)
        libraries.append(ROOT / "native/target" / target / "release/libatv_apple.a")
    output = ROOT / "apple/build/libs" / sdk / "libatv_apple.a"
    output.parent.mkdir(parents=True, exist_ok=True)
    if len(libraries) > 1:
        run(["xcrun", "lipo", "-create", *map(str, libraries), "-output", str(output)], env)
    else:
        shutil.copyfile(libraries[0], output)
    run(["xcodegen", "generate", "--spec", str(ROOT / "apple/project.yml"),
         "--project", str(ROOT / "apple")], env)
    command = ["xcodebuild", "-project", str(ROOT / "apple/ATV.xcodeproj"), "-scheme", scheme,
               "-configuration", args.configuration, "-sdk", sdk,
               "-destination", f"generic/platform={destination}",
               "-derivedDataPath", str(ROOT / "apple/build/DerivedData"), "ONLY_ACTIVE_ARCH=NO",
               "ARCHS=" + " ".join("arm64" if t.startswith("aarch64") else "x86_64" for t in targets)]
    if args.team:
        command += ["DEVELOPMENT_TEAM=" + args.team, "CODE_SIGN_STYLE=Automatic"]
    else:
        command += ["CODE_SIGNING_ALLOWED=NO"]
    command.append("build")
    run(command, env)
    product_dir = ROOT / "apple/build/DerivedData/Build/Products" / (
        args.configuration if sdk == "macosx" else args.configuration + "-" + sdk)
    product = product_dir / (scheme + ".app")
    if not product.is_dir():
        raise SystemExit("Xcode succeeded but the expected app bundle is missing")
    run(["python3", str(ROOT / "scripts/verify-apple-app.py"), str(product), "--output",
         str(ROOT / "apple/build" / (args.platform + "-dependencies.json"))], env)
    with output.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    report = {"platform": args.platform, "sdk": sdk, "rustTargets": targets,
              "rustStaticLibrarySha256": digest, "appBundle": str(product.relative_to(ROOT)),
              "buildVerified": True, "runtimePlaybackVerified": False,
              "signingRequested": bool(args.team), "deviceInstallationVerified": False}
    (ROOT / "apple/build" / (args.platform + "-build.json")).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
