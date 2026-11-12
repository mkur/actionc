#!/usr/bin/env python3
"""Measure MIR6502 home-value reuse against a separately frozen compiler."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
BUILD = ROOT / "build/home-value-invalidation"
DOC = ROOT / "docs/benchmarks/6502-home-value-invalidation"
BASE = "95e334be"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def normalized(path):
    return path.read_bytes().replace(b"\r\n", b"\n")


def measure(compiler, phase, check):
    compiler = Path(compiler).resolve()
    directory = BUILD / phase
    directory.mkdir(parents=True, exist_ok=True)
    if phase == "baseline":
        names = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", BASE, "src"], cwd=ROOT, text=True).splitlines()
        compiler_sources = {n: sha(subprocess.check_output(["git", "show", f"{BASE}:{n}"], cwd=ROOT).replace(b"\r\n", b"\n")) for n in names if n.endswith(".rs")}
    else:
        compiler_sources = {p.relative_to(ROOT).as_posix(): sha(normalized(p)) for p in sorted((ROOT / "src").rglob("*.rs"))}
    report = dict(baseline_revision=BASE, compiler_sha256=sha(compiler.read_bytes()),
                  compiler_sources_sha256=compiler_sources,
                  source_sha256={n: sha(normalized(ROOT / n)) for n in
                      ["samples/tn/modern/TN.ACT", "samples/tn/modern/TNDBG.ACT", "samples/tn/modern/LIB.ACT"]},
                  configurations=[])
    for name in ["TN", "TNDBG"]:
        for runtime in ["cart", "standalone"]:
            stem = f"{name}-{runtime}"
            image, listing = directory / f"{stem}.xex", directory / f"{stem}.asm"
            started = time.monotonic()
            subprocess.run([str(compiler), "--mode", "mir6502", "--runtime", runtime,
                            "-o", str(image), "--listing", str(listing),
                            str(ROOT / f"samples/tn/modern/{name}.ACT")],
                           cwd=ROOT, check=True, capture_output=True)
            elapsed = (time.monotonic() - started) * 1000
            text = normalized(listing).decode()
            routines = [dict(name=n, start=int(a, 16), end=int(b, 16)) for n, a, b in
                        re.findall(r"^; ===== PROC (\S+) \$([0-9A-F]+)\.\.\$([0-9A-F]+)", text, re.M)]
            alloc = next(r for r in routines if r["name"] == "Alloc")
            body = text.split("; ===== PROC Alloc ", 1)[1].split("; ===== END PROC Alloc =====", 1)[0]
            report["configurations"].append(dict(name=name, runtime=runtime,
                xex_bytes=image.stat().st_size, image_sha256=sha(image.read_bytes()),
                listing_sha256=sha(text.encode()), routines=routines,
                alloc_bytes=alloc["end"]-alloc["start"], alloc_listing=body,
                placement_lines=[l for l in text.splitlines() if re.match(
                    r"(?:global_|local_|param_|spill_).* = |; ===== DATA |; .*reserved|; .*storage", l)],
                compile_ms=elapsed))
    DOC.mkdir(parents=True, exist_ok=True)
    path = DOC / f"{phase}.json"
    if check:
        expected = json.loads(path.read_text())
        report["compiler_sha256"] = expected["compiler_sha256"]
        for old, new in zip(expected["configurations"], report["configurations"]):
            new["compile_ms"] = old["compile_ms"]
        assert report == expected, f"{phase} bindings differ"
    else:
        path.write_text(json.dumps(report, indent=2) + "\n")
    for r in report["configurations"]:
        print(f"{r['name']}/{r['runtime']}: {r['xex_bytes']} XEX bytes, Alloc {r['alloc_bytes']} bytes")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--compiler", required=True)
    p.add_argument("--phase", choices=["baseline", "current"], required=True)
    p.add_argument("--check", action="store_true")
    a = p.parse_args()
    measure(a.compiler, a.phase, a.check)
