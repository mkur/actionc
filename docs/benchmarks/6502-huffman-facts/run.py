#!/usr/bin/env python3
"""Audit Huffman facts without retaining a production-source change.

Run from any directory. Do not build or edit known_callees.rs concurrently.
Large artifacts are written under build/huffman-facts.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUTPUT = ROOT / "build/huffman-facts"
HOOK = b'''\n#[cfg(test)]
mod huffman_facts_audit { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/benchmarks/6502-huffman-facts/probe.rs")); }
'''


def main():
    (OUTPUT / "driver").mkdir(parents=True, exist_ok=True)
    for source, name in [
        ("tools/vm-runtime-tests/fixtures/huff_dec_driver.act", "huff_dec.act"),
        ("fixtures/runtime/tacle/huff_dec/types.inc", "types.inc"),
        ("fixtures/runtime/tacle/huff_dec/kernel.inc", "kernel.inc"),
    ]:
        text = (ROOT / source).read_text().replace("\r\n", "\n")
        (OUTPUT / "driver" / name).write_text(text)
    source = ROOT / "src/mir6502/analysis/known_callees.rs"
    original = source.read_bytes()
    assert b"mod huffman_facts_audit" not in original
    receipt = {
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "original_module_sha256": hashlib.sha256(original).hexdigest(),
        "probe_sha256": hashlib.sha256((HERE / "probe.rs").read_bytes()).hexdigest(),
        "shared_probe_sha256": hashlib.sha256((HERE.parent / "6502-call-facts-audit/probe.rs").read_bytes()).hexdigest(),
    }
    env = os.environ.copy()
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "build/checked-arena/shared-target"))
    env.setdefault("CARGO_BUILD_JOBS", "2")
    env.update(CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0",
               CARGO_PROFILE_DEV_OPT_LEVEL="1", CARGO_PROFILE_TEST_OPT_LEVEL="1")
    command = ["cargo", "test", "--lib",
               "mir6502::analysis::known_callees::huffman_facts_audit::audit_huffman",
               "--", "--exact", "--ignored", "--nocapture"]
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
    raise SystemExit(receipt["return_code"])


if __name__ == "__main__":
    main()
