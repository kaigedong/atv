#!/usr/bin/env python3
"""Summarize measured in-process Android soak samples; never infer hardware performance."""
import argparse
import json
from pathlib import Path
import statistics


def distribution(values):
    ordered = sorted(values)
    def percentile(fraction):
        at = (len(ordered) - 1) * fraction
        low = int(at)
        high = min(low + 1, len(ordered) - 1)
        return ordered[low] + (ordered[high] - ordered[low]) * (at - low)
    return {"min": min(ordered), "median": statistics.median(ordered),
            "p95": percentile(.95), "max": max(ordered)}


def summarize(path, warmup):
    report = json.loads(path.read_text())
    if report.get("passed") is not True:
        raise ValueError(f"{path}: acceptance failed; no performance success claim")
    soak = next(s for s in report["steps"] if s["name"] == "continuous-native-live")
    all_samples = soak["snapshots"]
    start = all_samples[0]["elapsedMs"]
    samples = [s for s in all_samples if s["elapsedMs"] >= start + warmup * 1000
               and s.get("state") == "playing" and "totalPssKiB" in s]
    if len(samples) < 3:
        raise ValueError(f"{path}: insufficient steady-state measured samples")
    first, last = samples[0], samples[-1]
    wall_ms = last["elapsedMs"] - first["elapsedMs"]
    intervals = [100 * (b["processCpuMs"] - a["processCpuMs"]) / (b["elapsedMs"] - a["elapsedMs"])
                 for a, b in zip(samples, samples[1:]) if b["elapsedMs"] - a["elapsedMs"] >= 1000]
    edge = min(30000, wall_ms // 3)
    early = [s["totalPssKiB"] / 1024 for s in samples if s["elapsedMs"] <= first["elapsedMs"] + edge]
    late = [s["totalPssKiB"] / 1024 for s in samples if s["elapsedMs"] >= last["elapsedMs"] - edge]
    result = {
        "sourceReport": str(path), "apkSha256": report["targetApkSha256"],
        "deviceModel": report["deviceModel"], "sdk": report["sdk"],
        "requestedSoakSeconds": soak["requestedSeconds"], "warmupExcludedSeconds": warmup,
        "measuredSeconds": wall_ms / 1000, "samples": len(samples),
        "resolution": [last.get("videoWidth"), last.get("videoHeight")],
        "processPssMiB": distribution([s["totalPssKiB"] / 1024 for s in samples]),
        "nativeHeapAllocatedMiB": distribution([s["nativeHeapAllocatedBytes"] / 1048576 for s in samples]),
        "javaHeapUsedMiB": distribution([s["javaHeapUsedBytes"] / 1048576 for s in samples]),
        "processCpuPercentOfOneCore": {
            "mean": 100 * (last["processCpuMs"] - first["processCpuMs"]) / wall_ms,
            "intervals": distribution(intervals)},
        "videoOutputFramesPerWallSecond": 1000 * (last["videoFrames"] - first["videoFrames"]) / wall_ms,
        "audioOutputFramesPerWallSecond": 1000 * (last["audioFrames"] - first["audioFrames"]) / wall_ms,
        "pssLastVsFirstWindowMedianMiB": statistics.median(late) - statistics.median(early),
        "nativeWorkerCounts": [s["nativeThreads"] for s in (first, last)],
        "limitations": [
            "App process with instrumentation monitoring overhead; 100% CPU means one logical core.",
            "PSS includes proportional shared pages; codec service/GPU allocations and their CPU are not fully charged to the app.",
            "Emulator results do not predict the MagicBox ARM device or prove hardware decoding.",
            "Finite-run PSS differences neither prove nor disprove a memory leak.",
            "Video output counters are not an independent physical display measurement."]}
    buffers = [s["buffers"] for s in samples if "buffers" in s]
    if len(buffers) == len(samples):
        result["renderPayloadBufferCapacityMiB"] = {
            key: distribution([b[key] / 1048576 for b in buffers])
            for key in ("fragmentCapacityBytes", "demuxCapacityBytes", "pendingPacketCapacityBytes",
                        "outputCapacityBytes", "renderCapacityBytes")}
        result["renderSampledPeakCapacityMiB"] = max(b["renderPeakCapacityBytes"] for b in buffers) / 1048576
        result["limitations"].append(
            "Buffer capacities cover selected renderer payloads only; exclude queued/in-flight downloads, TLS, metadata and OS codecs. Peak is sampled at batch boundaries, not a total allocation peak.")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path)
    parser.add_argument("--warmup-seconds", type=int, default=60)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.warmup_seconds < 0:
        parser.error("warmup must be nonnegative")
    result = {"evidence": "measured-Android-in-process-soak", "runs":
              [summarize(path, args.warmup_seconds) for path in args.reports]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
