#!/usr/bin/env python3
"""Reject private publication inputs. Findings contain locations, never values.

This is a targeted repository guard, not a guarantee that every possible secret
format is recognisable. Review new data sources before adding exceptions.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

GENERATED_TS = "app/src/androidTest/assets/baseline-avc-aac.ts"
GENERATED_TS_SHA256 = "48c190a34500d07161f1301fc88acb711b327a120e2b6d725e4f2a32a75be4a6"
REVIEWED_BINARIES = {
    GENERATED_TS: GENERATED_TS_SHA256,
    "gradle/wrapper/gradle-wrapper.jar": "81a82aaea5abcc8ff68b3dfcb58b3c3c429378efd98e7433460610fecd7ae45f",
}
REVIEWED_TEXT_FIXTURES = {
    "native/crates/atv-auth/tests/oracle-fixtures.json": "ef9d2f26a3449ccf4f465b909401fa260f94498a7b15de4c24a620b412b7aa6b",
}
# Fixed application metadata shipped by the public browser client, not a user token.
PUBLIC_PROTOCOL_DIGESTS = {"818510ad207eee818c17c0e36fb6108e4e781ea2401b6433334716fe9e05ba10"}
BLOCKED_PREFIXES = ("research/upstream/", "research/native-", "research/runtime/",
                    "research/minimal-experiment/", "research/legacy-webview-cache/",
                    "test-artifacts/", "artifacts/", "ci-dist/", "native/target/", ".secrets/")
BLOCKED_SUFFIXES = {".har", ".pcap", ".pcapng", ".apk", ".ipa", ".wasm", ".h264", ".aac",
                    ".mp4", ".wav", ".jks", ".keystore", ".p12", ".pfx", ".p8", ".pem",
                    ".mobileprovision", ".bundle"}
PATTERNS = {
    "personal-home-path": re.compile(r"/(?:home|Users)/[A-Za-z0-9_.-]+|[A-Za-z]:\\Users\\[A-Za-z0-9_.-]+"),
    "private-network-address": re.compile(r"(?<![\d.])(?:192\.168\.(?:\d{1,3}\.)\d{1,3}|10\.(?:\d{1,3}\.){2}\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.(?:\d{1,3}\.)\d{1,3})(?![\d.])"),
    "credential-token": re.compile(r"(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[A-Z0-9]{16}|AIza[A-Za-z0-9_-]{30,})"),
    "private-key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "personal-session-reference": re.compile(r"\.(?:dsh|codex)/sessions/|DeepSeek\s+harness"),
}
EMAIL = re.compile(r"\b[A-Za-z0-9._%+\[\]-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
LITERAL = re.compile(r'''(?i)["']?(?:cookie|authorization|password|access_token|refresh_token|device_guid|deviceGuid|guid|token|secret|vsecret)["']?\s*[:=]\s*["']([^"'\n]{8,})["']''')
SIGNED_URL = re.compile(r'''https?://[^\s"'`<>]+[?&](?:token|signature|auth|wsSecret|txSecret|access_token)=[^\s"'`<>]+''', re.I)


def github_email(value):
    return value.endswith("@users.noreply.github.com") or value == "noreply@github.com"


def inspect(path, data):
    findings = []
    if path in REVIEWED_TEXT_FIXTURES:
        # Git checkout may translate text LF to CRLF on Windows.
        digest = hashlib.sha256(data.replace(b"\r\n", b"\n")).hexdigest()
        if digest != REVIEWED_TEXT_FIXTURES[path]:
            findings.append({"path": path, "rule": "unreviewed-synthetic-fixture-change"})
    name = Path(path).name
    if (path.startswith(BLOCKED_PREFIXES) or Path(path).suffix.lower() in BLOCKED_SUFFIXES
            or name in {"local.properties", "atv-device-guid", "id_rsa", "id_ed25519"}
            or (name.startswith(".env") and name != ".env.example")
            or "build" in Path(path).parts):
        findings.append({"path": path, "rule": "private-or-generated-file"})
    if Path(path).suffix == ".ts":
        if path != GENERATED_TS or hashlib.sha256(data).hexdigest() != GENERATED_TS_SHA256:
            findings.append({"path": path, "rule": "unreviewed-media-sample"})
    if b"\0" in data:
        if REVIEWED_BINARIES.get(path) != hashlib.sha256(data).hexdigest():
            findings.append({"path": path, "rule": "unreviewed-binary-file"})
        return findings
    for number, line in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
        for rule, pattern in PATTERNS.items():
            if pattern.search(line):
                findings.append({"path": path, "line": number, "rule": rule})
        for match in EMAIL.finditer(line):
            if not github_email(match[0]) and not match[0].endswith("@fixture.invalid"):
                findings.append({"path": path, "line": number, "rule": "personal-email"})
        for match in LITERAL.finditer(line):
            value = match[1]
            if (value.startswith("fixture") or value in {"never-log-me", "invalid; cookie=injection"}
                    or "fixture.invalid" in value or "fixture-token" in value
                    or hashlib.sha256(value.encode()).hexdigest() in PUBLIC_PROTOCOL_DIGESTS):
                continue
            findings.append({"path": path, "line": number, "rule": "unreviewed-credential-literal"})
        if any("fixture.invalid" not in match[0]
               and not (".invalid/" in match[0] and match[0].endswith("token=never-log-me"))
               for match in SIGNED_URL.finditer(line)):
            findings.append({"path": path, "line": number, "rule": "signed-network-url"})
    return findings


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def audit(root, history=False, ref="HEAD"):
    findings = []
    scanned = 0
    if history:
        records = git(root, "rev-list", "--objects", ref).decode().splitlines()
        for record in records:
            oid, _, path = record.partition(" ")
            if git(root, "cat-file", "-t", oid).strip() != b"blob":
                continue
            scanned += 1
            findings.extend(inspect(path, git(root, "cat-file", "blob", oid)))
        metadata = git(root, "log", ref, "--format=%H%x00%ae%x00%ce").decode().splitlines()
        for record in metadata:
            oid, author, committer = record.split("\0")
            if not github_email(author) or not github_email(committer):
                findings.append({"commit": oid[:12], "rule": "personal-commit-email"})
    else:
        paths = set(git(root, "ls-files", "-co", "--exclude-standard", "-z").decode().split("\0"))
        for path in sorted(paths - {""}):
            file = root / path
            if file.is_file():
                scanned += 1
                findings.extend(inspect(path, file.read_bytes()))
    return {"passed": not findings, "historyChecked": history, "scannedFilesOrBlobs": scanned,
            "findings": findings, "valuesRedacted": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--history", action="store_true", help="scan all blobs/metadata reachable from --ref")
    parser.add_argument("--ref", default="HEAD")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = audit(args.root.resolve(), args.history, args.ref)
    payload = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(payload)
    print(payload, end="")
    raise SystemExit(0 if result["passed"] else 1)


if __name__ == "__main__":
    main()
