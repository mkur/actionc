#!/usr/bin/env python3
"""Bind the targeted word-move store fix to real TN images and Alloc VM cycles."""
import argparse
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile

import home_values as home

ROOT = home.ROOT
BASE = "71325d37"
BUILD = ROOT / "build/word-move-store"
DOC = ROOT / "docs/benchmarks/6502-word-move-store"


def source_digest(sources):
    return home.sha(json.dumps(sources, sort_keys=True, separators=(",", ":")).encode())


def measure(compiler, vm, check):
    compiler = Path(compiler).resolve()
    baseline = json.loads((ROOT / "docs/benchmarks/6502-home-value-invalidation/current.json").read_text())
    # Reuse the frozen, already qualified image receipts without modifying them.
    archive = subprocess.check_output(["git", "archive", BASE, "src"], cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        old_sources = {p.name: home.sha(tree.extractfile(p).read().replace(b"\r\n", b"\n"))
                       for p in tree if p.isfile() and p.name.endswith(".rs")}
    assert old_sources == baseline["compiler_sources_sha256"], "baseline is not the pinned compiler"
    for name, digest in baseline["source_sha256"].items():
        assert home.sha(home.normalized(ROOT / name)) == digest, f"library/source changed: {name}"
    sources = {p.relative_to(ROOT).as_posix(): home.sha(home.normalized(p))
               for p in sorted((ROOT / "src").rglob("*.rs"))}
    result = dict(baseline_revision=BASE, baseline_compiler_sources_sha256=source_digest(old_sources),
                  compiler_sources_sha256=source_digest(sources), compiler_sha256=home.sha(compiler.read_bytes()),
                  source_sha256=baseline["source_sha256"],
                  fixture_sha256=home.sha(home.normalized(ROOT / "fixtures/runtime/word_move_store.act")),
                  configurations=[])
    BUILD.mkdir(parents=True, exist_ok=True)
    for before in baseline["configurations"]:
        name, runtime = before["name"], before["runtime"]
        stem = f"{name}-{runtime}"
        image, listing = BUILD / f"{stem}.xex", BUILD / f"{stem}.asm"
        subprocess.run([str(compiler), "--mode", "mir6502", "--runtime", runtime,
                        "-o", str(image), "--listing", str(listing), str(ROOT / f"samples/tn/modern/{name}.ACT")],
                       cwd=ROOT, check=True, capture_output=True)
        text = home.normalized(listing).decode()
        routines = {n: int(b, 16) - int(a, 16) for n, a, b in re.findall(
            r"^; ===== PROC (\S+) \$([0-9A-F]+)\.\.\$([0-9A-F]+)", text, re.M)}
        old_routines = {r["name"]: r["end"] - r["start"] for r in before["routines"]}
        assert routines["Alloc"] == 79, "targeted fix must eliminate Alloc result staging"
        body = text.split("; ===== PROC Alloc ", 1)[1].split("; ===== END PROC Alloc =====", 1)[0]
        result["configurations"].append(dict(name=name, runtime=runtime,
            baseline_image_sha256=before["image_sha256"], image_sha256=home.sha(image.read_bytes()),
            listing_sha256=home.sha(text.encode()), before_xex_bytes=before["xex_bytes"], xex_bytes=image.stat().st_size,
            before_alloc_bytes=before["alloc_bytes"], alloc_bytes=routines["Alloc"], alloc_listing=body,
            routine_changes={n: [old_routines[n], size] for n, size in routines.items() if old_routines[n] != size}))
    path = DOC / "current.json"
    expected = json.loads(path.read_text()) if check else None
    if vm:
        subprocess.run(["cargo", "test", "--release", "--manifest-path", str(ROOT / "tools/vm-runtime-tests/Cargo.toml"),
                        "--test", "home_value_invalidation", "real_alloc_image_controls", "--", "--nocapture"],
                       cwd=ROOT, check=True, env={**os.environ, "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2",
                           "ACTIONC_HOME_VALUE_TN_IMAGES": str(BUILD),
                           "ACTIONC_HOME_VALUE_ALLOC_CYCLES": str(BUILD / "alloc-cycles.tsv")})
        result["alloc_cycles"] = (BUILD / "alloc-cycles.tsv").read_text().splitlines()
    else:
        assert check, "new cycle receipts require --vm"
        # Reuse cycles only when the complete fresh image/source receipt matches.
        result["alloc_cycles"] = expected["alloc_cycles"]
    if check:
        result["compiler_sha256"] = expected["compiler_sha256"]
        assert result == expected, "word-move store bindings differ"
    else:
        DOC.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(result, indent=2) + "\n")
    for r in result["configurations"]:
        print(f"{r['name']}/{r['runtime']}: Alloc {r['before_alloc_bytes']} -> {r['alloc_bytes']}, XEX {r['before_xex_bytes']} -> {r['xex_bytes']}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--vm", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    measure(args.compiler, args.vm, args.check)
