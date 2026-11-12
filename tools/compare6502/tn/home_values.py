#!/usr/bin/env python3
"""Measure MIR6502 home-value reuse against a separately frozen compiler."""
import argparse
import hashlib
import json
import os
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


def details(compiler, phase, check):
    """Bind frame placement, accepted rule counts, bounded facts and VM results."""
    bindings = json.loads((DOC / f"{phase}.json").read_text())
    directory = BUILD / phase
    result = dict(compiler_sources_sha256=bindings["compiler_sources_sha256"], configurations=[])
    for config in bindings["configurations"]:
        name, runtime = config["name"], config["runtime"]
        stem = f"{name}-{runtime}"
        image = directory / f"{stem}.xex"
        assert sha(image.read_bytes()) == config["image_sha256"], f"stale {image}"
        run = subprocess.run([str(Path(compiler).resolve().with_name("actionc-emit")), "--profile", "modern", "--backend", "mir6502", "--runtime", runtime,
                              "--emit-materialized-mir6502", str(ROOT / f"samples/tn/modern/{name}.ACT")],
                             cwd=ROOT, check=True, capture_output=True,
                             env={**os.environ, "ACTIONC_MIR6502_PEEPHOLES": "sites"})
        mir = run.stdout.replace(b"\r\n", b"\n").decode()
        stats = run.stderr.replace(b"\r\n", b"\n").decode()
        (directory / f"{stem}.mir").write_text(mir)
        (directory / f"{stem}.peepholes").write_text(stats)
        assembly = normalized(directory / f"{stem}.asm").decode()
        # DATA labels are emission's actual scalar/spill reservations, rather
        # than the materializer's tentative addresses.
        slots = []
        for label, body in re.findall(r"^((?:local_|param_|spill_)\w+):\n(.*?)(?=^; =====|\Z)", assembly, re.M | re.S):
            size = sum(len(re.findall(r"\$[0-9A-F]{2}", line.split(";", 1)[0]))
                       for line in body.splitlines() if ".BYTE" in line)
            slots.append(dict(symbol=label, bytes=size))
        counts = {}
        # Selection and materialization print separate reports.
        for aggregate in re.findall(r"^  aggregate:\n(.*?)(?=^  per-routine:|\Z)", stats, re.M | re.S):
            for rule, count in re.findall(r"^    ([\w-]+): (\d+)$", aggregate, re.M):
                counts[rule] = counts.get(rule, 0) + int(count)
        peaks = {}
        for routine, peak in re.findall(r"^    (\S+): ssa-lite-home-values: maximum retained byte facts: (\d+);", stats, re.M):
            peaks[routine] = max(peaks.get(routine, 0), int(peak))
        if phase == "current":
            assert peaks, "current actionc-emit must be rebuilt alongside actionc"
        source_zp = set()
        for width, address in re.findall(r"^global g\d+ \S+: (byte|word) absolute \$(00[0-9A-F]{2})", mir, re.M):
            base = int(address, 16)
            source_zp.update(range(base, base + (1 if width == "byte" else 2)))
        result["configurations"].append(dict(name=name, runtime=runtime,
            image_sha256=config["image_sha256"], mir_sha256=sha(mir.encode()),
            frame_slots=slots, spill_slots=sum(s["symbol"].startswith("spill_") for s in slots),
            allocated_zp_lines=[line for line in mir.splitlines() if re.match(r"  zp\d+ ->", line)],
            source_zp_bytes=sorted(source_zp),
            accepted_ssa_lite_rewrites={r: n for r, n in counts.items() if r in {
                "ssa-lite-consumer-forwards", "ssa-lite-cross-block-forwards", "ssa-lite-redundant-reloads"}},
            max_retained_byte_facts=peaks))
    neutral = directory if phase == "baseline" else BUILD / "current-neutral"
    result["neutral_images"] = []
    for path in sorted(neutral.glob("neutral-*.json")):
        result["neutral_images"].append(dict(stem=path.stem,
            image_sha256=sha(path.with_suffix(".xex").read_bytes()), metadata=json.loads(path.read_text())))
    assert len(result["neutral_images"]) == 8
    result["neutral_cycles"] = (directory / "neutral-cycles.tsv").read_text().splitlines()
    result["alloc_cycles"] = (directory / "alloc-cycles.tsv").read_text().splitlines()
    path = DOC / f"{phase}-details.json"
    if check:
        assert json.loads(path.read_text()) == result, f"{phase} detailed bindings differ"
    else:
        path.write_text(json.dumps(result, indent=2) + "\n")
    print(f"{phase}: four MIR/frame/rule profiles, eight neutral images and VM cycles match")


def mads(compiler, phase, check):
    """Reassemble and relocate every real TN image with actual LF/CRLF listings."""
    directory = BUILD / phase
    result = []
    for name in ["TN", "TNDBG"]:
        for runtime in ["cart", "standalone"]:
            stem = f"{name}-{runtime}"
            original = normalized(directory / f"{stem}.asm").decode()
            for origin in [0x2C00, 0x3100]:
                expected = directory / f"{stem}.xex"
                if origin != 0x2C00:
                    expected = directory / f"{stem}-{origin:04X}.xex"
                    subprocess.run([str(Path(compiler).resolve()), "--mode", "mir6502", "--runtime", runtime,
                        "--origin", hex(origin), "-o", str(expected), str(ROOT / f"samples/tn/modern/{name}.ACT")],
                        check=True, cwd=ROOT, capture_output=True)
                listing = original.replace("ACTIONC_ORIGIN = $2C00", f"ACTIONC_ORIGIN = ${origin:04X}")
                for crlf in [False, True]:
                    asm = directory / f"{stem}-{origin:04X}-{crlf}.mads.asm"
                    image = asm.with_suffix(".xex")
                    asm.write_bytes((listing.replace("\n", "\r\n") if crlf else listing).encode())
                    subprocess.run([os.environ.get("ACTIONC_MADS", "mads"), str(asm), f"-o:{image}", "-s"],
                        check=True, capture_output=True)
                    assert image.read_bytes() == expected.read_bytes(), str(asm)
                    result.append(dict(name=name, runtime=runtime, origin=origin, crlf=crlf,
                                       image_sha256=sha(image.read_bytes())))
    path = DOC / f"{phase}-mads.json"
    if check:
        assert json.loads(path.read_text()) == result
    else:
        path.write_text(json.dumps(result, indent=2) + "\n")
    print(f"{phase}: 16 real TN/TNDBG assemblies and relocations match direct emission")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--compiler", required=True)
    p.add_argument("--phase", choices=["baseline", "current"], required=True)
    p.add_argument("--check", action="store_true")
    p.add_argument("--details", action="store_true", help="bind/check existing measured images, MIR profiles and VM cycle files")
    p.add_argument("--mads", action="store_true", help="bind/check TN reassembly and relocation in MADS")
    a = p.parse_args()
    (mads if a.mads else details if a.details else measure)(a.compiler, a.phase, a.check)
