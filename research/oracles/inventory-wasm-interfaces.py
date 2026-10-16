#!/usr/bin/env python3
"""Extract complete typed interfaces from retained REA evidence; no WASM execution."""
from collections import Counter
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
SELECTED = [
    ("cmg", "research/native-cmg/rea-wasm.json", "research/native-cmg/cmg-0.wasm", "research/upstream/cmg.worker.js"),
    ("openapi", "research/native-auth/rea-keygen-evidence.json", "research/native-auth/keygen_bg.wasm", "research/native-auth/cctvh5-openapicore.min.js"),
    ("ticket", "research/native-auth/rea-ticket-evidence.json", "research/native-auth/RJq7sO71JF.wasm", "research/upstream/RJq7sO71JF.js"),
]
modules = []
for name, evidence_path, wasm_path, glue_path in SELECTED:
    envelope = json.loads((ROOT / evidence_path).read_text())
    evidence = envelope.get("data", envelope)
    result = evidence["normalized_result"]
    interface = result["interface"]
    wasm = (ROOT / wasm_path).read_bytes()
    digest = hashlib.sha256(wasm).hexdigest()
    if digest != result["artifact"]["sha256"] or len(wasm) != result["artifact"]["bytes"]:
        raise ValueError(f"{name}: original artifact identity changed")
    if interface["state"] != "complete" or interface["issues"]:
        raise ValueError(f"{name}: retained typed interface is not complete")
    if hashlib.sha256(result["wat"]["text"].encode()).hexdigest() != result["wat"]["sha256"]:
        raise ValueError(f"{name}: decoded WAT identity changed")
    aliases = {}
    glue = (ROOT / glue_path).read_text()
    if name == "cmg":
        pairs = re.findall(r'Module\.([\w$]+)=function\([^)]*\)\{return Module\.asm\.([\w$]+)\.apply', glue)
    elif name == "ticket":
        pairs = re.findall(r'Module\["([^"\n]+)"\]=function\(\)\{return\([^;{}]{0,200}?Module\["asm"\]\["([^"\n]+)"\]', glue)
    else:
        pairs = []
    for alias, export in pairs:
        aliases.setdefault(export, []).append(alias)
    entries = interface["entries"]
    for entry in entries:
        if entry["direction"] == "export":
            entry["jsAliases"] = sorted(set(aliases.get(entry["name"], [])))
    modules.append({
        "module": name,
        "artifactSha256": digest,
        "artifactBytes": len(wasm),
        "reaEvidenceId": evidence["evidence_id"],
        "watSha256": result["wat"]["sha256"],
        "typedInterfaceState": interface["state"],
        "counts": dict(Counter(entry["direction"] for entry in entries)),
        "glueSha256": hashlib.sha256((ROOT / glue_path).read_bytes()).hexdigest(),
        "aliasEvidence": "literal wrapper forwarding only; not a function-semantics proof",
        "entries": entries,
    })
output = ROOT / "research/oracles/wasm-interfaces.json"
output.write_text(json.dumps({"scope": "selected pinned original core-WASM modules",
    "runtimeExecutedByThisInventory": False, "networkFetchedByThisInventory": False,
    "allFunctionSemanticsProven": False, "modules": modules}, ensure_ascii=False, indent=2) + "\n")
for module in modules:
    print(module["module"], module["counts"], module["reaEvidenceId"])
print(output)
