#!/usr/bin/env python3
"""Reproduce the historical fact audit using a temporary test-only include.

Run from any directory. Do not build or edit known_callees.rs concurrently.
Results go to build/call-facts-audit-20261116; archived evidence is untouched.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUTPUT = ROOT / "build/call-facts-audit-20261116"
HOOK = b'''\n#[cfg(test)]
mod call_facts_audit { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/build/call-facts-audit-20261116/probe.rs")); }
'''


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    probe = (HERE / "probe.rs").read_bytes()
    (OUTPUT / "probe.rs").write_bytes(probe)
    source = ROOT / "src/mir6502/analysis/known_callees.rs"
    original = source.read_bytes()
    assert b"mod call_facts_audit" not in original
    receipt = {
        "revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip(),
        "original_module_sha256": hashlib.sha256(original).hexdigest(),
        "probe_sha256": hashlib.sha256(probe).hexdigest(),
    }
    env = os.environ.copy()
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "build/checked-arena/shared-target"))
    env.setdefault("CARGO_BUILD_JOBS", "2")
    env.update(
        CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0",
        CARGO_PROFILE_DEV_OPT_LEVEL="1", CARGO_PROFILE_TEST_OPT_LEVEL="1",
    )
    command = [
        "cargo", "test", "--lib",
        "mir6502::analysis::known_callees::call_facts_audit::audit_call_facts",
        "--", "--exact", "--ignored", "--nocapture",
    ]
    receipt["command"] = command
    source.write_bytes(original + HOOK)
    try:
        receipt["return_code"] = subprocess.run(command, cwd=ROOT, env=env).returncode
    finally:
        current = source.read_bytes()
        assert current.count(HOOK) == 1
        source.write_bytes(current.replace(HOOK, b"", 1))
        receipt["hook_removed"] = source.read_bytes() == original
        (OUTPUT / "run-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if receipt["return_code"] == 0:
        image = (OUTPUT / "tn.xex").read_bytes()
        receipt["tn_bytes"] = len(image)
        receipt["tn_sha256"] = hashlib.sha256(image).hexdigest()
        expected = json.loads((HERE / "receipt.json").read_text())
        receipt["matches_audited_tn_image"] = receipt["tn_sha256"] == expected["tn_sha256"]
        (OUTPUT / "run-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps(receipt, indent=2))
    raise SystemExit(receipt["return_code"])


if __name__ == "__main__":
    main()
