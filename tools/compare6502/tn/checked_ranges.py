#!/usr/bin/env python3
"""Generate/check fresh checked-arena bindings without altering historical data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
BUILD = ROOT / "build/checked-arena"
DOC = ROOT / "docs/benchmarks/6502-checked-arena"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(command, *, cwd=ROOT, env=None):
    subprocess.run(command, cwd=cwd, env=env, check=True)


def normalized(path):
    return path.read_bytes().replace(b"\r\n", b"\n")


def cycle_rows(path):
    rows = []
    for line in path.read_text().splitlines():
        name, mode, runtime, operation, amount, cursor, cycles = line.split("\t")
        rows.append(dict(name=name.removesuffix(".ACT"), mode=mode.lower(),
                         runtime="cart" if runtime == "ActionCart" else "standalone",
                         operation=operation, amount=int(amount), cursor=int(cursor), cycles=int(cycles)))
    return rows


def stable(value):
    # Timings are observations; image/source identities and cycle counts bind
    # the experiment. Wall-clock scheduling is not a reproducible checksum.
    if isinstance(value, dict):
        return {k: stable(v) for k, v in value.items() if not k.endswith("_ms")}
    if isinstance(value, list):
        return [stable(v) for v in value]
    return value


def generate(workspace, check, vm):
    BUILD.mkdir(parents=True, exist_ok=True)
    workspace = Path(workspace).resolve()
    env = dict(os.environ, CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2",
               CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0",
               CARGO_PROFILE_DEV_OPT_LEVEL="1", CARGO_PROFILE_TEST_OPT_LEVEL="1",
               CARGO_TARGET_DIR=str(BUILD / "shared-target"),
               ACTIONC_CHECKED_RANGE_OUTPUT=str(BUILD / "measured"))
    compiler_sources = {p.relative_to(ROOT).as_posix(): sha(normalized(p))
                        for p in sorted((ROOT / "src").rglob("*.rs"))}
    for name, digest in compiler_sources.items():
        assert sha(normalized(workspace / name)) == digest, f"validation checkout differs: {name}"
    run(["cargo", "test", "--lib", "fresh_checked_range_measurements", "--", "--ignored"],
        cwd=workspace, env=env)
    measured = json.loads((BUILD / "measured/measurements.json").read_text())
    for row in measured["configurations"]:
        row["image_sha256"] = sha((BUILD / "measured" / f"{row['stem']}.xex").read_bytes())
        row["listing_sha256"] = sha(normalized(BUILD / "measured" / f"{row['stem']}.asm"))
    unchecked = json.loads((DOC / "unchecked.json").read_text())
    for row in unchecked["configurations"]:
        assert sha((BUILD / f"{row['name']}.xex").read_bytes()) == row["image_sha256"], row["name"]
    if vm:
        vm_env = dict(os.environ, CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2",
                      ACTIONC_ARENA_VM_RESULTS=str(BUILD / "checked-cycles.tsv"),
                      ACTIONC_UNCHECKED_ARENA_VM_RESULTS=str(BUILD / "unchecked-cycles.tsv"),
                      ACTIONC_CHECKED_RANGE_VM_RESULTS=str(BUILD / "neutral-cycles.tsv"))
        base = ["cargo", "test", "--release", "--manifest-path", "tools/vm-runtime-tests/Cargo.toml"]
        run(base + ["--test", "tn_checked_arena", "--test", "checked_pointer_ranges"], env=vm_env)
        run(base + ["--test", "tn_checked_arena", "unchecked_valid_allocation_cycle_controls", "--", "--ignored"], env=vm_env)
    checked_cycles = cycle_rows(BUILD / "checked-cycles.tsv")
    unchecked_cycles = cycle_rows(BUILD / "unchecked-cycles.tsv")
    for row in measured["configurations"]:
        if row["range_enabled"]:
            continue  # Already asserted byte-for-byte equal to the off image.
        mode = "Mir6502" if row["mode"] == "mir6502" else "Optimized"
        runtime = "ActionCart" if row["runtime"] == "cart" else "Standalone"
        path = BUILD / f"vm-{row['name']}.ACT-{mode}-{runtime}.xex"
        assert sha(path.read_bytes()) == row["image_sha256"], f"rerun --vm: {path}"
    neutral_cycles = []
    for line in (BUILD / "neutral-cycles.tsv").read_text().splitlines():
        opt, runtime, origin, enabled, size, cycles = line.split("\t")
        neutral_cycles.append(dict(optimized_nir=opt == "true", runtime=runtime,
                                   origin=int(origin, 16), enabled=enabled == "true",
                                   emitted_bytes=int(size), cycles=int(cycles)))
    report = dict(
        compiler_sources_sha256=compiler_sources,
        source_sha256={p.relative_to(ROOT).as_posix(): sha(normalized(p)) for p in
                       [ROOT / "samples/tn/modern" / n for n in ["TN.ACT", "TNDBG.ACT", "LIB.ACT"]]},
        measurement_sources_sha256={n: sha(normalized(ROOT / n)) for n in
            ["tools/compare6502/tn/checked_ranges.py", "tools/vm-runtime-tests/tests/tn_checked_arena.rs",
             "tools/vm-runtime-tests/tests/checked_pointer_ranges.rs", "fixtures/runtime/checked_pointer_ranges.act"]},
        unchecked_binding=dict(path="unchecked.json", sha256=sha((DOC / "unchecked.json").read_bytes()),
                               source_revision=unchecked["source_revision"],
                               compiler_sha256=unchecked["compiler_sha256"]),
        profile="Rust dev/test opt-level=1, debug=0, incremental=0; compile_ms measures generation, not rustc build",
        default_range_retention=dict(conservative=False, optimized=True),
        configurations=measured["configurations"], analyses=measured["analyses"],
        checked_cycles=checked_cycles, unchecked_cycles=unchecked_cycles, neutral_cycles=neutral_cycles)
    path = DOC / "checked.json"
    if check:
        assert stable(json.loads(path.read_text())) == stable(report), "fresh checked bindings differ"
    else:
        path.write_text(json.dumps(report, indent=2) + "\n")
    print(f"{'Verified' if check else 'Recorded'} 12 fresh TN configurations, historical bindings and image-matched VM cycles")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", default=ROOT, help="optional clean checkout with identical compiler sources")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--vm", action="store_true", help="regenerate execution measurements as well as images")
    args = parser.parse_args()
    generate(args.workspace, args.check, args.vm)
