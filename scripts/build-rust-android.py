#!/usr/bin/env python3
"""Compile the real Rust Android adapter for Gradle; no browser build stage."""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess
from toolchain import rust_environment

ROOT = Path(__file__).resolve().parent.parent
NDK_VERSION = "29.0.14206865"
TARGETS = {"armeabi-v7a": "armv7-linux-androideabi", "arm64-v8a": "aarch64-linux-android",
           "x86": "i686-linux-android", "x86_64": "x86_64-linux-android"}
LEGACY_NDK_VERSION = "25.2.9519653"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--abis", default=",".join(TARGETS))
    parser.add_argument("--package", choices=["atv-android", "atv-output-probe"], default="atv-android")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    abis = args.abis.split(",")
    if not abis or len(set(abis)) != len(abis) or any(abi not in TARGETS for abi in abis):
        parser.error("ABIs must be distinct supported Android ABIs")
    env = rust_environment()
    cargo = shutil.which("cargo", path=env.get("PATH"))
    if not cargo:
        raise SystemExit("Rust 1.99.1/cargo is required. See docs/rust-android.md.")
    modern_ndk = Path(env.get("ANDROID_NDK_HOME", str(args.sdk / "ndk" / NDK_VERSION)))
    hosts = {"Linux": "linux-x86_64", "Darwin": "darwin-x86_64", "Windows": "windows-x86_64"}
    if platform.system() not in hosts:
        raise SystemExit("Unsupported Android NDK build host")
    windows = platform.system() == "Windows"
    env["CARGO_TARGET_DIR"] = str(ROOT / "native/target")
    env["RUSTFLAGS"] = env.get("RUSTFLAGS", "") + " -C link-arg=-Wl,-z,max-page-size=16384"
    args.output.mkdir(parents=True, exist_ok=True)
    for abi in TARGETS:
        if abi not in abis and (args.output / abi).exists():
            shutil.rmtree(args.output / abi)
    for abi in abis:
        target = TARGETS[abi]
        legacy = abi in {"armeabi-v7a", "x86"}
        api = 19 if legacy else 21
        ndk = Path(env.get("ATV_LEGACY_NDK_HOME", str(args.sdk / "ndk" / LEGACY_NDK_VERSION))) if legacy else modern_ndk
        tools = ndk / "toolchains/llvm/prebuilt" / hosts[platform.system()] / "bin"
        nm = tools / ("llvm-nm.exe" if windows else "llvm-nm")
        compiler_target = "armv7a-linux-androideabi" if abi == "armeabi-v7a" else target
        linker = tools / (compiler_target + str(api) + "-clang" + (".cmd" if windows else ""))
        cxx = tools / (compiler_target + str(api) + "-clang++" + (".cmd" if windows else ""))
        if not linker.is_file():
            raise SystemExit(f"Missing NDK compiler for {abi}")
        env[f"CARGO_TARGET_{target.upper().replace('-', '_')}_LINKER"] = str(linker)
        env[f"CC_{target.replace('-', '_')}"] = str(linker)
        env[f"CXX_{target.replace('-', '_')}"] = str(cxx)
        env[f"AR_{target.replace('-', '_')}"] = str(tools / ("llvm-ar.exe" if windows else "llvm-ar"))
        command = [cargo, "build", "--locked", "--manifest-path", str(ROOT / "native/Cargo.toml"),
                        "-p", args.package, "--release", "--target", target]
        if legacy:
            command += ["--features", "legacy-android"]
        # Resolve every strong dependency against the actual minimum API stubs.
        target_env = env.copy()
        target_env["RUSTFLAGS"] += " -C link-arg=-Wl,-z,defs"
        if args.package == "atv-output-probe":
            # A test library must not interpose the production JNI registry.
            target_env["RUSTFLAGS"] += " -C link-arg=-Wl,--exclude-libs,ALL"
        if legacy:
            target_env["RUSTFLAGS"] += ' --cfg getrandom_backend="custom"'
        subprocess.run(command, env=target_env, cwd=ROOT, check=True)
        library_name = "libatv_native.so" if args.package == "atv-android" else "libatv_output_probe.so"
        library = Path(env["CARGO_TARGET_DIR"]) / target / "release" / library_name
        symbols = subprocess.check_output([str(nm), "--defined-only", "--dynamic", str(library)], text=True)
        exported = {line.split()[-1] for line in symbols.splitlines() if line.split()}
        if args.package == "atv-output-probe" and any("NativeBridge_" in name for name in exported):
            raise SystemExit("Test library unexpectedly exports production JNI methods")
        bridge = "NativeBridge" if args.package == "atv-android" else "OutputProbe"
        methods = ["create", "select", "pause", "resume", "destroy", "status"] if args.package == "atv-android" else ["run", "close"]
        for method in methods:
            if f"Java_cn_bobo_atv_nativeplayer_{bridge}_{method}" not in exported:
                raise SystemExit(f"Rust JNI method {bridge}.{method} missing for {abi}")
        destination = args.output / abi / library_name
        destination.parent.mkdir(parents=True, exist_ok=True)
        temporary = destination.with_suffix(".so.tmp")
        shutil.copyfile(library, temporary)
        temporary.replace(destination)
        print(f"Rust native output ready: {abi}", flush=True)


if __name__ == "__main__":
    main()
