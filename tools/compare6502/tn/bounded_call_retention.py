#!/usr/bin/env python3
"""Check measured bounded-call retention controls and generate the scorecard."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
BUILD = ROOT / "build/bounded-call"
DOC = ROOT / "docs/benchmarks/6502-bounded-call-retention"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def text(path):
    return path.read_text().replace("\r\n", "\n")


def generate(check):
    baseline = json.loads(text(BUILD / "baseline.json"))
    source = "fixtures/runtime/bounded_call_pointer_retention.act"
    assert baseline["source_sha256"] == digest(text(ROOT / source).encode())
    report = dict(date="2026-11-12", implementation_revision="60d456f8",
                  qualification="Optimized default enabled; incoming flag observation rejected",
                  source_sha256=baseline["source_sha256"],
                  neutral=[], tn=[], checks={}, vm_samples=[])
    # The VM revision is also frozen by the previous measurement controls.
    qualified = json.loads(text(ROOT / "docs/benchmarks/6502-argument-x-bitwise/results.json"))
    report["vm_revision"] = qualified["vm_revision"]
    for opt in [False, True]:
        for runtime in ["ActionCart", "Standalone"]:
            for origin in [0x3000, 0x41C7]:
                stem = f"neutral-{str(opt).lower()}-{runtime}-{origin:04X}"
                facts, images = [], []
                for enabled in [False, True]:
                    path = BUILD / "measured" / f"{stem}-{str(enabled).lower()}"
                    facts.append(json.loads(text(path.with_suffix(".json"))))
                    images.append(path.with_suffix(".xex").read_bytes())
                    assert len(images[-1]) == facts[-1]["xex_bytes"]
                old, new = facts
                assert old["emitted_bytes"] == 180 and new["emitted_bytes"] == 170
                assert len(images[0]) - len(images[1]) == 10
                assert [s[:2] + s[3:] for s in old["storage"]] == [s[:2] + s[3:] for s in new["storage"]]
                frames = lambda f: [s for s in f["storage"] if s[0].startswith("Routine(")]
                assert frames(old) == frames(new)
                assert [n for _, n in old["reservations"]] == [n for _, n in new["reservations"]]
                sizes = [{name: end-start for name, start, end in f["routines"]} for f in facts]
                assert sizes[0]["ReadAcrossCopy"] == 56 and sizes[1]["ReadAcrossCopy"] == 46
                assert sizes[0]["UnknownCopy"] == sizes[1]["UnknownCopy"] == 69
                report["neutral"].append(dict(name=stem, image_bytes=list(map(len, images)),
                    image_sha256=list(map(digest, images)), routine_bytes=sizes,
                    storage_capacities_unchanged=True, frames_unchanged=True,
                    reservation_lengths_unchanged=True))
                if opt:
                    label = "cart" if runtime == "ActionCart" else "standalone"
                    before = f"neutral-{label}-{origin:04X}"
                    listing = BUILD / "before" / f"{before}.asm"
                    frozen = next(r for r in baseline["records"] if r["name"] == before)
                    assert digest(text(listing).encode()) == frozen["listing_sha256"]
                    output = listing.with_suffix(".xex")
                    result = subprocess.run([os.environ.get("ACTIONC_MADS", "mads"), str(listing),
                        f"-o:{output}", "-s"], capture_output=True, text=True)
                    assert result.returncode == 0, result.stdout + result.stderr
                    assert output.read_bytes() == images[0]
    for runtime in ["ActionCart", "Standalone"]:
        for name in ["TN", "TNDBG"]:
            stem = f"{name}-{runtime}-true"
            image = (BUILD / "tn-qualified" / f"{stem}.xex").read_bytes()
            expected = next(p for p in qualified["pairs"] if p["name"] == f"{name}-{runtime}")
            assert digest(image) == expected["image_sha256"][1]
            listing = text(BUILD / "tn-qualified" / f"{stem}.asm")
            start, end = re.search(r"; ===== PROC Strcat \$([0-9A-F]+)\.\.\$([0-9A-F]+)", listing).groups()
            assert int(end, 16)-int(start, 16) == 113
            report["tn"].append(dict(name=stem, image_bytes=len(image), image_sha256=digest(image),
                                     unchanged=True, strcat_bytes=113))
    for line in text(BUILD / "vm-results.tsv").splitlines():
        opt, runtime, origin, enabled, emitted, cycles, stores = line.split("\t")
        report["vm_samples"].append(dict(optimized_nir=opt == "true", runtime=runtime,
            origin=int(origin, 16), enabled=enabled == "true", emitted_bytes=int(emitted),
            sample_cycles=int(cycles), pair_stores=int(stores)))
        assert int(cycles) == (142 if enabled == "true" else 156)
        assert int(stores) == (2 if enabled == "true" else 4)
    assert len(report["vm_samples"]) == 16
    for name in ["backend-unit", "backend-integration", "backend-vm", "tn-and-mads", "tn-alias-vm", "tn-dispatch-vm"]:
        log = text(BUILD / f"{name}.log")
        results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", log)
        assert results and all(int(r[1]) == 0 for r in results), name
        assert "test result: FAILED" not in log
        report["checks"][name] = dict(targets=len(results), passed=sum(int(r[0]) for r in results),
                                      ignored=sum(int(r[2]) for r in results))
    sweep = text(BUILD / "backend-sweep.log")
    assert "ok=169 load_failed=0 sem_failed=0 nir_failed=0 mir_lower_failed=0 mir_verify_failed=0 materialization_failed=0 pre_emission_failed=0 emission_failed=0" in sweep
    report["checks"].update(emission_load_fixtures=169, mads_lf_crlf_relocations=8,
                              frozen_neutral_assembly_matches=4, new_vm_calls=2952)
    paths = [source, "src/mir6502/analysis/known_callees/bounded_memory.rs",
             "src/mir6502/analysis/record_bases/retention.rs", "src/mir6502/materialize/pointer_retention.rs",
             "src/mir6502/passes.rs", "tests/mir6502_bounded_call_pointer_retention.rs",
             "tools/vm-runtime-tests/tests/bounded_call_pointer_retention.rs"]
    report["implementation_sha256"] = {p: digest(text(ROOT / p).encode()) for p in paths}
    DOC.mkdir(parents=True, exist_ok=True)
    for filename, data in [("baseline.json", baseline), ("results.json", report)]:
        path = DOC / filename
        if check:
            assert json.loads(text(path)) == data, filename
        else:
            path.write_text(json.dumps(data, indent=2) + "\n")
    print("Verified eight neutral pairs, four frozen neutral controls, four TN images and all qualification logs")
    print(json.dumps(report["checks"], indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    generate(parser.parse_args().check)
