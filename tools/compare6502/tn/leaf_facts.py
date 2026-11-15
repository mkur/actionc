#!/usr/bin/env python3
"""Dump private MIR6502 callee facts without changing the production compiler.

Installs a temporary test-only include, runs exactly one ignored diagnostic test,
and removes its include even if compilation fails. Do not run concurrent builds
or edits of known_callees.rs while this probe is installed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
HOOK = b'''\n// BEGIN temporary TN leaf facts probe\n#[cfg(test)]\nmod tn_leaf_facts_audit {\n    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/compare6502/tn/leaf_facts_probe.rs"));\n}\n// END temporary TN leaf facts probe\n'''


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("build/tn-leaf-facts"))
    parser.add_argument("--expect-image", type=Path)
    args = parser.parse_args()
    destination = args.output.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    source = ROOT / "src/mir6502/analysis/known_callees.rs"
    original = source.read_bytes()
    if b"temporary TN leaf facts probe" in original:
        raise SystemExit("A diagnostic hook is already installed; inspect it before proceeding.")
    paths = ["samples/tn/modern/TN.ACT", "samples/tn/modern/LIB.ACT",
             "tools/compare6502/tn/leaf_facts.py", "tools/compare6502/tn/leaf_facts_probe.rs",
             "src/mir6502/analysis/known_callees.rs", "src/mir6502/analysis/effects.rs"]
    receipt = {
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "inputs_sha256": {p: digest(ROOT / p) for p in paths},
        "compiler_diff_sha256": hashlib.sha256(subprocess.check_output(
            ["git", "diff", "HEAD", "--", "src", "Cargo.toml", "Cargo.lock"], cwd=ROOT)).hexdigest(),
    }
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(ROOT / "build/checked-arena/shared-target"),
               CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0",
               CARGO_PROFILE_DEV_OPT_LEVEL="1", CARGO_PROFILE_TEST_OPT_LEVEL="1",
               CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2",
               ACTIONC_LEAF_FACTS_OUTPUT=str(destination))
    if args.expect_image:
        env["ACTIONC_LEAF_FACTS_EXPECT_IMAGE"] = str(args.expect_image.resolve())
        receipt["expected_image_sha256"] = digest(args.expect_image)
    else:
        env.pop("ACTIONC_LEAF_FACTS_EXPECT_IMAGE", None)
    command = ["cargo", "test", "--lib",
               "mir6502::analysis::known_callees::tn_leaf_facts_audit::dump_tn_leaf_facts",
               "--", "--exact", "--ignored", "--nocapture"]
    receipt["command"] = command
    source.write_bytes(original + HOOK)
    try:
        subprocess.run(command, cwd=ROOT, env=env, check=True)
    finally:
        current = source.read_bytes()
        if current.count(HOOK) != 1:
            raise RuntimeError("Temporary hook changed; remove it manually without discarding other edits.")
        source.write_bytes(current.replace(HOOK, b"", 1))
    receipt["outputs_sha256"] = {p: digest(destination / p) for p in ["facts.json", "leaves.asm", "tn.xex"]}
    receipt["hook_removed"] = HOOK not in source.read_bytes()
    (destination / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
