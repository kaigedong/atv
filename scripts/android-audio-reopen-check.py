#!/usr/bin/env python3
"""Verify app reopen/focus recovery against the emulator's real output sink."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import threading
import time

PACKAGE = "cn.bobo.atv.nativeplayer"
ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--discovery-file", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--adb", default=str(Path(os.environ["ANDROID_HOME"]) / "platform-tools/adb")
                        if os.environ.get("ANDROID_HOME") else "adb")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"emulator-[0-9]+", args.serial):
        parser.error("this installation/output test requires an owned development emulator")
    discovery = {}
    for line in args.discovery_file.read_text().splitlines():
        key, separator, value = line.partition("=")
        if separator and key in ("port.serial", "grpc.port"):
            discovery[key] = value
    if (discovery.get("port.serial") != args.serial.removeprefix("emulator-")
            or args.endpoint != "127.0.0.1:" + discovery.get("grpc.port", "")):
        parser.error("selected audio sink and Android serial must belong to the same emulator")
    if args.output.exists():
        parser.error("select a fresh output directory; retained captures must not be reused")
    args.output.mkdir(parents=True)
    adb = [args.adb, "-s", args.serial]
    if not args.skip_build:
        subprocess.run([str(ROOT / "scripts/android-build.sh"), "assembleDebug", "assembleDebugAndroidTest", "lintDebug"],
                       cwd=ROOT, check=True)
    apk = ROOT / "app/build/outputs/apk/debug/app-debug.apk"
    for artifact in (apk, ROOT / "app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"):
        subprocess.run(adb + ["install", "-r", str(artifact)], check=True)
    process = subprocess.Popen(adb + ["shell", "am", "instrument", "-w", "-e", "mode", "audio-lifecycle",
                               "-e", "audioProbeSeconds", "10",
                               PACKAGE + ".test/" + PACKAGE + ".NativeRuntimeInstrumentation"],
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    lines = queue.Queue()
    def read_lines():
        try:
            for line in process.stdout:
                lines.put(line)
        finally:
            lines.put(None)
    reader = threading.Thread(target=read_lines, daemon=True)
    reader.start()
    captures = []
    status = {}
    report_path = None
    deadline = time.monotonic() + 480
    try:
        with (args.output / "instrumentation.txt").open("w") as log:
            while True:
                if time.monotonic() >= deadline:
                    raise RuntimeError("audio lifecycle instrumentation exceeded its deadline")
                try:
                    line = lines.get(timeout=1)
                except queue.Empty:
                    continue
                if line is None:
                    break
                log.write(line)
                log.flush()
                match = re.match(r"INSTRUMENTATION_STATUS: (audioProbeStage|audioProbeExpectation)=(.*)", line)
                if match:
                    status[match[1]] = match[2].strip()
                if line.startswith("INSTRUMENTATION_STATUS_CODE:"):
                    if "audioProbeStage" in status:
                        stage = status["audioProbeStage"]
                        if not re.fullmatch(r"initial|reopened-[1-3]|focus-lost|focus-regained", stage):
                            raise RuntimeError("unrecognized audio capture stage")
                        output = args.output / (stage + ".json")
                        if status.get("audioProbeExpectation") == "silent":
                            # Drain the emulator/mixer's finite tail in a separate observation.
                            # The following full window must still contain no nonzero samples.
                            drain = subprocess.run([sys.executable, str(ROOT / "scripts/capture-emulator-audio.py"),
                                "--endpoint", args.endpoint, "--seconds", "2", "--discovery-file", str(args.discovery_file),
                                "--output", str(args.output / (stage + "-drain.json"))],
                                capture_output=True, text=True, timeout=20)
                            drain_path = args.output / (stage + "-drain.json")
                            if drain.returncode not in (0, 1) or not drain_path.is_file():
                                raise RuntimeError("audio tail drain failed")
                            drained = json.loads(drain_path.read_text())
                            if drained.get("transportOutcome") != "DEADLINE_EXCEEDED" or drained.get("durationWallSeconds", 0) < 1.5:
                                raise RuntimeError("audio tail drain was unavailable or incomplete")
                        capture = subprocess.run([sys.executable, str(ROOT / "scripts/capture-emulator-audio.py"),
                            "--endpoint", args.endpoint, "--seconds", "4", "--discovery-file", str(args.discovery_file),
                            "--output", str(output)], capture_output=True, text=True, timeout=20)
                        if capture.returncode not in (0, 1) or not output.is_file():
                            raise RuntimeError("audio sink capture failed")
                        evidence = json.loads(output.read_text())
                        if evidence.get("transportOutcome") != "DEADLINE_EXCEEDED" or evidence.get("durationWallSeconds", 0) < 3.5:
                            raise RuntimeError("audio sink observation was unavailable or incomplete")
                        audible = status.get("audioProbeExpectation") == "audible"
                        passed = evidence.get("nonSilentOutputObserved") is audible
                        captures.append({"stage": stage, "expectedAudible": audible, "passed": passed,
                                         "nonzeroSamples": evidence.get("nonzeroSamples"), "capture": str(output)})
                        print(json.dumps(captures[-1]), flush=True)
                        if not passed:
                            raise RuntimeError("unexpected output silence/activity at " + stage)
                    status = {}
                match = re.match(r"INSTRUMENTATION_RESULT: reportPath=((?:/storage/emulated/0/Android/data/"
                    + re.escape(PACKAGE) + r"/files|/data/(?:data|user/0)/" + re.escape(PACKAGE)
                    + r"/files)/runtime-native/[0-9a-f-]{36}\.json)\s*$", line)
                if match:
                    report_path = match[1]
        process.wait(timeout=10)
        if not report_path:
            raise RuntimeError("instrumentation did not return a report")
        destination = args.output / "result.json"
        if report_path.startswith("/data/"):
            destination.write_bytes(subprocess.check_output(adb + ["shell", "run-as", PACKAGE, "cat", report_path]))
        else:
            subprocess.run(adb + ["pull", report_path, str(destination)], check=True)
        result = json.loads(destination.read_text())
        expected = {"initial", "reopened-1", "reopened-2", "reopened-3", "focus-lost", "focus-regained"}
        digest = hashlib.sha256(apk.read_bytes()).hexdigest()
        passed = (result.get("passed") is True and result.get("targetApkSha256") == digest
                  and {row["stage"] for row in captures} == expected and len(captures) == len(expected)
                  and all(row["passed"] for row in captures))
        summary = {"passed": passed, "targetApkSha256": digest, "captures": captures,
                   "physicalTvVerified": False, "audioRetained": False}
        (args.output / "audio-output.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps({"passed": passed, "output": str(args.output), "targetApkSha256": digest}, indent=2))
        return 0 if passed else 1
    finally:
        if process.poll() is None:
            subprocess.run(adb + ["shell", "am", "force-stop", PACKAGE], check=True)
            process.terminate()
            process.wait(timeout=10)
        process.stdout.close()
        reader.join(timeout=2)


if __name__ == "__main__":
    raise SystemExit(main())
