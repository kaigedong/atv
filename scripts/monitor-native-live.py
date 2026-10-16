#!/usr/bin/env python3
"""Observe actual native status and screenshots for a sustained live-playback gate."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import time
from toolchain import android_sdk


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--seconds", type=int, default=600)
    parser.add_argument("--apk", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 10 <= args.seconds <= 3600:
        parser.error("--seconds must be between 10 and 3600")
    sdk = android_sdk()
    adb = [str(sdk / "platform-tools/adb"), "-s", args.serial]
    package = "cn.bobo.atv.nativeplayer"

    def command(*items):
        return subprocess.run(adb + list(items), capture_output=True, check=True, timeout=15).stdout

    pid = command("shell", "pidof", package).decode().strip()
    if not pid.isdigit():
        raise ValueError("native application must already be running")
    apk_hash = hashlib.sha256(args.apk.read_bytes()).hexdigest()
    installed_path = command("shell", "pm", "path", package).decode().strip().removeprefix("package:")
    installed_hash = command("shell", "sha256sum", installed_path).decode().split()[0]
    if installed_hash != apk_hash:
        raise ValueError("installed APK does not match the supplied artifact")
    args.output.mkdir(parents=True, exist_ok=True)
    with (args.output / "meminfo-start.txt").open("wb") as target:
        target.write(command("shell", "dumpsys", "meminfo", pid))
    log = subprocess.Popen(adb + ["logcat", "--pid=" + pid, "-T", "1", "-v", "brief", "-s", "ATVNative:I", "*:S"],
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
    selector = selectors.DefaultSelector()
    selector.register(log.stdout, selectors.EVENT_READ)
    started = time.monotonic()
    first = previous = final = None
    playing_began = last_progress = None
    samples, shots = [], []
    next_shot = 0
    failure = None
    keep = ("state", "pid", "positionMs", "videoWidth", "videoHeight", "videoFrames", "audioFrames",
            "audioSampleRate", "audioChannels", "audioNonzeroSamples", "audioPeak", "audioRms", "errorCode")
    print(json.dumps({"monitor": "started", "processPid": pid, "apkSha256": apk_hash}), flush=True)
    try:
        while time.monotonic() - started < args.seconds + 90:
            if log.poll() is not None:
                failure = "logcat process terminated"
                break
            if not selector.select(timeout=1):
                if playing_began and time.monotonic() - last_progress > 20:
                    failure = "no advancing native status for 20 seconds"
                    break
                continue
            line = log.stdout.readline()
            begin = line.find("{")
            if begin < 0:
                continue
            try:
                raw = json.loads(line[begin:])
            except json.JSONDecodeError:
                continue
            state = {key: raw[key] for key in keep if key in raw}
            now = time.monotonic()
            state["observedWallSeconds"] = round(now - started, 3)
            samples.append(state)
            if state.get("state") == "error":
                failure = "native player reported " + state.get("errorCode", "error")
                break
            if first is None:
                if state.get("state") != "playing" or not state.get("videoFrames") or not state.get("audioFrames"):
                    continue
                first = state
                playing_began = last_progress = now
            elif state.get("pid") != first.get("pid"):
                failure = "channel changed during sustained playback"
                break
            elif previous is not None:
                if state.get("videoFrames", 0) < previous.get("videoFrames", 0) or state.get("audioFrames", 0) < previous.get("audioFrames", 0):
                    failure = "media output counters reset"
                    break
                if state.get("videoFrames", 0) > previous.get("videoFrames", 0) and state.get("audioFrames", 0) > previous.get("audioFrames", 0):
                    last_progress = now
                elif now - last_progress > 15:
                    failure = "one or both native output tracks stopped"
                    break
            previous = final = state
            elapsed = now - playing_began
            if elapsed >= next_shot:
                path = args.output / f"screen-{int(elapsed):04d}.png"
                path.write_bytes(command("exec-out", "screencap", "-p"))
                shots.append({"wallSeconds": round(elapsed, 3), "path": path.name,
                              "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
                next_shot += args.seconds / 2
            print(json.dumps({"monitor": "sample", "wallSeconds": round(elapsed, 3),
                              "positionMs": state.get("positionMs"), "videoFrames": state.get("videoFrames"),
                              "audioFrames": state.get("audioFrames")}), flush=True)
            if elapsed >= args.seconds and state.get("positionMs", 0) - first.get("positionMs", 0) >= args.seconds * 1000:
                break
        else:
            failure = "sustained playback deadline exceeded"
    finally:
        log.terminate()
        try:
            log.wait(timeout=5)
        except subprocess.TimeoutExpired:
            log.kill()
            log.wait(timeout=5)
        selector.close()
    with (args.output / "meminfo-end.txt").open("wb") as target:
        target.write(command("shell", "dumpsys", "meminfo", pid))
    passed = failure is None and first is not None and final is not None and final["positionMs"] - first["positionMs"] >= args.seconds * 1000
    result = {"evidence": "actual-native-sustained-live-playback", "passed": passed, "failure": failure,
              "deviceSerial": args.serial, "isEmulator": args.serial.startswith("emulator-"), "processPid": pid,
              "apkSha256": apk_hash, "requiredSeconds": args.seconds, "first": first, "last": final,
              "samples": samples, "screenshots": shots, "externalAudioSinkEvidenceRequiredSeparately": True}
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"monitor": "finished", "passed": passed, "failure": failure,
                      "result": str(args.output / "result.json")}), flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
