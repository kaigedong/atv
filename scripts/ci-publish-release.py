#!/usr/bin/env python3
"""Promote a complete, hash-verified native build to a public GitHub Release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

from android_version import build_version

ROOT = Path(__file__).resolve().parent.parent
REPO = "kaigedong/atv"
ABIS = ("universal", "armeabi-v7a", "arm64-v8a", "x86", "x86_64")
NATIVE = {"atv-linux-x86_64": "linux-x86_64", "atv-linux-aarch64": "linux-aarch64",
          **{"atv-apple-" + p: p for p in ("macos", "ios", "ios-simulator", "tvos", "tvos-simulator")}}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def prepare(source, output, version, fingerprint):
    """Only copy allowlisted products from this exact commit; never whole artifacts."""
    android = load(source / "atv-android-signed/manifest.json")
    if any(android.get(key) != value for key, value in version.items()):
        raise ValueError("Signed APK release identity differs from the checked-out commit")
    if len(android["artifacts"]) != len(ABIS) or {a["abi"] for a in android["artifacts"]} != set(ABIS):
        raise ValueError("Release requires all five signed Android variants")
    products, native = [], []

    def product(directory, name, expected_hash, platform):
        path = directory / name
        if path.is_symlink() or not path.is_file() or digest(path) != expected_hash:
            raise ValueError("Release product does not match its build manifest: " + name)
        products.append((path, {"file": name, "sha256": expected_hash,
                                "bytes": path.stat().st_size, "platform": platform}))

    for entry in android["artifacts"]:
        name = f'atv-android-{entry["abi"]}-release.apk'
        if entry["file"] != name or entry["minSdk"] != 19:
            raise ValueError("Unexpected Android release filename or minimum SDK")
        signature = entry["signature"]
        if signature["certificateSha256"] != fingerprint or not all(
                signature["schemes"].get(v) is True for v in ("v1", "v2", "v3")):
            raise ValueError("Release APK lacks the pinned signature or KitKat signature")
        directory = source / "atv-android-signed"
        gate = load((directory / name).with_suffix(".json"))
        if (gate["apkSha256"] != entry["sha256"] or gate["forbiddenAssetsPresent"] is not False
                or gate["prohibitedDexReferencesPresent"] is not False):
            raise ValueError("APK dependency gate does not match this release")
        product(directory, name, entry["sha256"], "android-" + entry["abi"])
    for artifact, platform in NATIVE.items():
        directory = source / artifact
        metadata = load(directory / "manifest.json")
        name = f"atv-{platform}.tar.gz"
        if metadata.get("sourceCommit") != version["sourceCommit"] or metadata["archive"] != name:
            raise ValueError("Native archive does not belong to this release commit")
        if platform not in ("linux-x86_64", "linux-aarch64") and (
                metadata.get("buildVerified") is not True or metadata.get("signingRequested") is not False):
            raise ValueError("Expected verified unsigned Apple application")
        product(directory, name, metadata["archiveSha256"], platform)
        native.append(metadata)
    # Do not create partial output until every platform has passed validation.
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ValueError("Release staging directory must be empty")
    for path, metadata in products:
        shutil.copyfile(path, output / metadata["file"])
    manifest = {"schemaVersion": 1, **version, "assets": [p[1] for p in products],
                "android": android, "native": native}
    manifest_path = output / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (output / "SHA256SUMS").write_text(
        "".join(f'{metadata["sha256"]}  {metadata["file"]}\n' for _, metadata in products)
        + digest(manifest_path) + "  manifest.json\n", encoding="utf-8")
    return manifest


def release_code(tag):
    if tag == "v0.1.0-native.1":
        return 1
    match = re.fullmatch(r"v0\.1\.0-r(\d+)", tag)
    if not match:
        raise ValueError("Unknown latest release version; refusing to change its ordering")
    return int(match[1])


def command(arguments):
    result = subprocess.run(["gh", *arguments], cwd=ROOT, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError("GitHub release operation failed: " + " ".join(arguments[:3]))
    return result.stdout.strip()


def api(path, optional=False):
    result = subprocess.run(["gh", "api", f"repos/{REPO}/" + path], capture_output=True, text=True)
    if optional and result.returncode and "HTTP 404" in result.stderr:
        return None
    if result.returncode:
        raise RuntimeError("GitHub release metadata request failed")
    return json.loads(result.stdout)


def check_remote_assets(release, files, complete):
    expected = {p.name: p for p in files}
    remote = {a["name"]: a for a in release["assets"]}
    if not set(remote) <= set(expected) or (complete and set(remote) != set(expected)):
        raise ValueError("Release asset set differs from the validated products")
    for name, asset in remote.items():
        if asset["state"] != "uploaded" or asset.get("digest") != "sha256:" + digest(expected[name]):
            raise ValueError("Uploaded release asset checksum differs: " + name)
    return set(expected) - set(remote)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=ROOT / "ci-dist/downloaded")
    parser.add_argument("--output", type=Path, default=ROOT / "ci-dist/release")
    args = parser.parse_args()
    if (os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("GITHUB_REPOSITORY") != REPO
            or os.environ.get("GITHUB_REF") != "refs/heads/main"
            or os.environ.get("GITHUB_EVENT_NAME") not in {"push", "workflow_dispatch"}):
        raise ValueError("Only the main-branch CI job may publish releases")
    version = build_version(ROOT)
    if version["sourceCommit"] != os.environ.get("GITHUB_SHA"):
        raise ValueError("Publishing checkout differs from workflow commit")
    if api("commits/main")["sha"] != version["sourceCommit"]:
        print("Skipping superseded commit; the newer main build will publish its own release")
        return
    identity = load(ROOT / "app/signing-certificate.json")
    prepare(args.input, args.output, version, identity["certificateSha256"])
    files = sorted(args.output.iterdir())
    tag = version["tag"]
    release = api("releases/tags/" + tag, optional=True)
    latest = api("releases/latest", optional=True)
    if latest and latest["tag_name"] != tag and release_code(latest["tag_name"]) >= version["versionCode"]:
        raise ValueError("Refusing to replace a newer release with an older build")
    if release and release["target_commitish"] != version["sourceCommit"]:
        raise ValueError("Existing release tag targets a different source commit")
    if release and not release["draft"]:
        check_remote_assets(release, files, complete=True)
        print("Already published and verified: " + release["html_url"])
        return
    if not release:
        notes = Path(os.environ["RUNNER_TEMP"]) / "atv-release-notes.txt"
        notes.write_text(
            f'ATV {version["versionName"]} · commit {version["sourceCommit"]}\n\n'
            "Android：下载对应 ABI 的 release.apk；旧 ARMv7 / Android 4.4 选择 armeabi-v7a，"
            "不确定架构时选择 universal。五个 APK 使用固定正式签名，支持覆盖同签名旧版本。"
            "从临时 Debug 签名包迁移需卸载重装，卸载会清除本地数据。\n\n"
            "Linux：提供 x86_64 与 ARM64 归档，需要安装 SDL2、GStreamer 及 AVC/AAC 插件。\n\n"
            "macOS / iOS / tvOS：提供未签名应用归档；Apple 真机安装需要自己的签名配置。\n\n"
            "本次 CI 验证编译、签名与包检查；未执行真机直播或性能验收。应用内更新尚未实现。\n\n"
            f'Android versionCode：{version["versionCode"]}\n'
            f'构建记录：https://github.com/{REPO}/actions/runs/{os.environ["GITHUB_RUN_ID"]}\n',
            encoding="utf-8")
        command(["release", "create", tag, "--repo", REPO, "--target", version["sourceCommit"],
                 "--title", "ATV " + version["versionName"], "--draft", "--notes-file", str(notes)])
        release = api("releases/tags/" + tag)
    missing = check_remote_assets(release, files, complete=False)
    if missing:
        command(["release", "upload", tag, "--repo", REPO, *[str(p) for p in files if p.name in missing]])
    release = api("releases/tags/" + tag)
    check_remote_assets(release, files, complete=True)
    if api("commits/main")["sha"] != version["sourceCommit"]:
        print("Newer main commit arrived; leaving this complete draft unpublished")
        return
    command(["release", "edit", tag, "--repo", REPO, "--draft=false", "--latest"])
    published = api("releases/tags/" + tag)
    if published["draft"] or published["prerelease"]:
        raise ValueError("Release was not published to the public update channel")
    check_remote_assets(published, files, complete=True)
    print("Published and verified: " + published["html_url"])


if __name__ == "__main__":
    main()
