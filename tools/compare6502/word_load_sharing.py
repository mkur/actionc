#!/usr/bin/env python3
"""Export/check the direct-word-sharing corpus; bulk artifacts stay in build/."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
ARTIFACTS = ROOT / "build/word-load-sharing/measured"
SCORECARD = ROOT / "docs/benchmarks/6502-word-load-sharing/current.json"


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def export():
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0",
               CARGO_PROFILE_TEST_DEBUG="0", CARGO_PROFILE_DEV_OPT_LEVEL="1",
               CARGO_PROFILE_TEST_OPT_LEVEL="1", CARGO_BUILD_JOBS="2",
               CARGO_TARGET_DIR=str(ROOT / "build/checked-arena/shared-target"),
               ACTIONC_WORD_LOAD_RESULTS=str(ARTIFACTS / "current.json"))
    # The scorecard uses the feature's own aggregate counters, not candidate
    # estimates inferred from source names or disassembly.
    env.pop("ACTIONC_MIR6502_PEEPHOLES", None)
    log = ARTIFACTS / "export.log"
    with log.open("w") as output:
        result = subprocess.run(["cargo", "test", "--lib",
                                 "word_load_sharing_measure_corpus", "--",
                                 "--ignored", "--nocapture"], cwd=ROOT, env=env,
                                stdout=output, stderr=subprocess.STDOUT)
    if result.returncode:
        sys.exit(f"Corpus export failed; see {log}")


def compact():
    raw = json.loads((ARTIFACTS / "current.json").read_text())
    reports = {}
    key = None
    for line in (ARTIFACTS / "export.log").read_text().splitlines():
        if line.startswith("MEASURE word-load-sharing "):
            key = line.split()[-1]
            reports[key] = {}
        elif key and line.startswith("    word-load-sharing-"):
            name, count = line.strip().split(": ")
            reports[key][name.removeprefix("word-load-sharing-")] = int(count)
    rows = []
    for row in raw["measurements"]:
        old, new = row["before"], row["after"]
        assert old["frames"] == new["frames"]
        assert [r[1] for r in old["reserved"]] == [r[1] for r in new["reserved"]]
        facts = {"frames": old["frames"], "reservation_capacities": [r[1] for r in old["reserved"]]}
        relocations = [{"before": a[0], "after": b[0], "bytes": a[1]}
                       for a, b in zip(old["reserved"], new["reserved"]) if a[0] != b[0]]
        changed = {name: {"before": size, "after": new["routine_bytes"][name],
                          "sites": new["sites"].get(name, 0)}
                   for name, size in old["routine_bytes"].items()
                   if size != new["routine_bytes"][name]}
        stats = reports[f'{row["name"]}/{row["runtime"]}/true']
        sites = sum(new["sites"].values())
        assert stats.get("applied", 0) == sites
        images = [hashlib.sha256((ARTIFACTS / f'{row["name"]}-{row["runtime"]}-{enabled}.xex').read_bytes()).hexdigest()
                  for enabled in ["false", "true"]]
        rows.append({**{k: row[k] for k in ["name", "source", "origin", "runtime"]},
                     "xex_bytes": [old["xex_bytes"], new["xex_bytes"]],
                     "xex_sha256": images,
                     "routine_instruction_bytes": [sum(old["routine_bytes"].values()),
                                                   sum(new["routine_bytes"].values())],
                     "direct_byte_loads": [old["direct_byte_loads"], new["direct_byte_loads"]],
                     "storage_facts_sha256": digest(facts),
                     "reservation_relocations": relocations,
                     "changed_routines": changed, "selection": stats,
                     "selector_emission_batches": new["selector_emission_batches"]})
    # Record text inputs independently of git revision and host newline format.
    paths = subprocess.check_output(["git", "ls-files", "src", "include",
                                     "samples/tn/modern", "samples/toolkit/modern"],
                                    cwd=ROOT, text=True).splitlines()
    paths += [r["source"] for r in rows]
    paths += ["Cargo.lock", "src/mir6502/materialize/word_load_sharing_measurements.rs",
              "tools/compare6502/word_load_sharing.py"]
    inputs = {p: hashlib.sha256((ROOT / p).read_bytes().replace(b"\r\n", b"\n")).hexdigest()
              for p in sorted(set(paths)) if (ROOT / p).is_file()}
    return {"schema": 1, "baseline_revision": "7f7778f6",
            "configuration": "Mir6502Config::optimized(); toggle only enable_word_load_sharing",
            "compiler_and_source_inputs_sha256": digest(inputs),
            "storage_facts_equal": True, "outer_materializations_per_variant": 1,
            "measurements": rows}


def mads():
    count = 0
    for path in sorted(ARTIFACTS.glob("*-true.asm")) + sorted(ARTIFACTS.glob("*-false.asm")):
        expected = path.with_suffix(".xex").read_bytes()
        lf = path.read_text().replace("\r\n", "\n")
        for crlf in [False, True]:
            asm = path.with_suffix(".roundtrip.asm")
            image = path.with_suffix(".roundtrip.xex")
            asm.write_bytes((lf.replace("\n", "\r\n") if crlf else lf).encode())
            result = subprocess.run([os.environ.get("ACTIONC_MADS", "mads"), str(asm),
                                     f"-o:{image}", "-s"], capture_output=True, text=True)
            if result.returncode:
                sys.exit(result.stdout + result.stderr)
            assert image.read_bytes() == expected, (path.name, crlf)
            asm.unlink()
            image.unlink()
            count += 1
    assert count == 112, count
    print(f"MADS: {count} identical LF/CRLF roundtrips")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare with the committed scorecard")
    parser.add_argument("--reuse", action="store_true", help="use an existing export for this compiler")
    parser.add_argument("--mads", action="store_true", help="reassemble both variants with LF and CRLF")
    args = parser.parse_args()
    if not args.reuse:
        export()
    result = compact()
    if args.check:
        if result != json.loads(SCORECARD.read_text()):
            sys.exit("Word load sharing measurements changed; inspect build/word-load-sharing/measured/")
        print("Word load sharing: 28 corpus comparisons match")
    else:
        SCORECARD.write_text(json.dumps(result, indent=2) + "\n")
        print(f"Wrote {SCORECARD.relative_to(ROOT)}")
    if args.mads:
        mads()


if __name__ == "__main__":
    main()
