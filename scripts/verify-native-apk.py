#!/usr/bin/env python3
"""Inspect the packaged native artifact; this is a dependency gate, not a playback test."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import zipfile


def elf_summary(data, expected_machine):
    if len(data) < 64 or data[:4] != b"\x7fELF" or data[4] not in (1, 2) or data[5] != 1:
        raise ValueError("native library is not a little-endian ELF32/ELF64 file")
    if data[4] != (2 if expected_machine in (62, 183) else 1):
        raise ValueError("native library bitness does not match APK directory")
    machine = struct.unpack_from("<H", data, 18)[0]
    if machine != expected_machine:
        raise ValueError("native library architecture does not match APK directory")
    elf64 = data[4] == 2
    word = "Q" if elf64 else "I"
    dynamic_size = 16 if elf64 else 8
    offset = struct.unpack_from("<" + word, data, 32 if elf64 else 28)[0]
    entry_size, entries = struct.unpack_from("<HH", data, 54 if elf64 else 42)
    if entry_size < (56 if elf64 else 32) or entries > 4096 or offset + entry_size * entries > len(data):
        raise ValueError("invalid ELF program headers")
    alignments, loads, dynamic = [], [], None
    for index in range(entries):
        entry = offset + index * entry_size
        kind = struct.unpack_from("<I", data, entry)[0]
        if elf64:
            file_offset, vaddr, _, size, _, alignment = struct.unpack_from("<QQQQQQ", data, entry + 8)
        else:
            file_offset, vaddr, _, size, _, _, alignment = struct.unpack_from("<IIIIIII", data, entry + 4)
        if kind == 2:
            dynamic = (file_offset, size)
        if kind != 1:
            continue
        if file_offset + size > len(data) or alignment < 16384 or (vaddr - file_offset) % 16384:
            raise ValueError("native LOAD segment is not valid for 16 KiB pages")
        alignments.append(alignment)
        loads.append((vaddr, file_offset, size))
    if not alignments:
        raise ValueError("native library has no LOAD segments")
    if dynamic is None or dynamic[0] + dynamic[1] > len(data) or dynamic[1] % dynamic_size:
        raise ValueError("native library has invalid dynamic metadata")
    string_address, string_size, needed = None, None, []
    for cursor in range(dynamic[0], dynamic[0] + dynamic[1], dynamic_size):
        tag, value = struct.unpack_from("<" + word * 2, data, cursor)
        if tag == 0:
            break
        if tag == 1:
            needed.append(value)
        elif tag == 5:
            string_address = value
        elif tag == 10:
            string_size = value
    if string_address is None or string_size is None:
        raise ValueError("native dynamic string table is missing")
    string_offset = next((file + string_address - address for address, file, size in loads
                          if address <= string_address and string_address + string_size <= address + size), None)
    if string_offset is None:
        raise ValueError("native dynamic string table is out of bounds")
    strings = data[string_offset:string_offset + string_size]
    dependencies = []
    for index in needed:
        end = strings.find(b"\x00", index)
        if index >= len(strings) or end < 0:
            raise ValueError("invalid native dependency name")
        dependencies.append(strings[index:end].decode("ascii"))
    system_libraries = {"libandroid.so", "libmediandk.so", "libaaudio.so", "libdl.so", "libc.so", "libm.so", "liblog.so"}
    if not dependencies or not set(dependencies) <= system_libraries:
        raise ValueError("native library has an unexpected runtime dependency")
    if expected_machine in (3, 40) and set(dependencies) & {"libmediandk.so", "libaaudio.so"}:
        raise ValueError("API 19 library requires newer system media libraries")
    for name in (b"Java_cn_bobo_atv_nativeplayer_NativeBridge_" + method
                 for method in (b"create", b"select", b"pause", b"resume", b"destroy", b"status", b"quality",
                                b"updateCreate", b"updateCommand", b"updateStatus", b"updateVerify", b"updateDestroy")):
        if name + b"\x00" not in data:
            raise ValueError("native JNI symbol is missing")
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
            "machine": machine, "loadAlignments": alignments, "dynamicDependencies": dependencies}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk")
    parser.add_argument("--abis", default="armeabi-v7a,arm64-v8a,x86,x86_64")
    parser.add_argument("--output", type=Path, help="write the dependency report as JSON")
    args = parser.parse_args()
    machines = {"armeabi-v7a": 40, "arm64-v8a": 183, "x86": 3, "x86_64": 62}
    wanted = set(args.abis.split(","))
    if not wanted or not wanted <= machines.keys():
        parser.error("unsupported ABI selection")
    with zipfile.ZipFile(args.apk) as apk:
        names = apk.namelist()
        forbidden = (".wasm", ".js", ".mjs", ".cjs", ".html", ".htm")
        if any(name.lower().endswith(forbidden) for name in names):
            raise ValueError("APK contains a browser/JavaScript/WASM asset")
        assets = {name for name in names if name.startswith("assets/") and not name.endswith("/")}
        if assets != {"assets/channels.json"}:
            raise ValueError("APK contains an unexpected production asset")
        dex_files = [name for name in names if name.endswith(".dex")]
        if not dex_files:
            raise ValueError("APK contains no Android entry point")
        for name in dex_files:
            dex = apk.read(name)
            for marker in (b"Landroid/webkit/", b"Ljavax/script/", b"Lorg/mozilla/javascript/", b"Lcom/eclipsesource/v8/",
                           b"Lcom/quickjs/", b"Lcom/dokar/quickjs/", b"Lio/github/kawamuray/wasmtime/"):
                if marker in dex:
                    raise ValueError("APK DEX references a prohibited browser or script runtime")
        libraries = {}
        for abi in sorted(wanted):
            name = f"lib/{abi}/libatv_native.so"
            libraries[abi] = elf_summary(apk.read(name), machines[abi])
        expected = {f"lib/{abi}/libatv_native.so" for abi in wanted}
        if {name for name in names if name.endswith(".so")} != expected:
            raise ValueError("APK contains an unexpected native library")
    with open(args.apk, "rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    report = {"evidence": "packaged-dependency-and-ELF-inspection", "apkSha256": digest,
                      "forbiddenAssetsPresent": False, "prohibitedDexReferencesPresent": False,
                      "nativeLibraries": libraries, "playbackVerifiedByThisCheck": False}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
