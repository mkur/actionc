#!/usr/bin/env python3
"""Freeze reproducible unchecked TN controls before introducing checked cursors."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
DOC = ROOT / "docs/benchmarks/6502-checked-arena"
BUILD = ROOT / "build/checked-arena"
BASE = "dedd35ab"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def generate(compiler, check):
    compiler = Path(compiler).resolve()
    sources = BUILD / "unchecked-sources"
    sources.mkdir(parents=True, exist_ok=True)
    source_hashes = {}
    for name in ["TN.ACT", "TNDBG.ACT", "LIB.ACT"]:
        path = f"samples/tn/modern/{name}"
        data = subprocess.check_output(["git", "show", f"{BASE}:{path}"], cwd=ROOT).replace(b"\r\n", b"\n")
        (sources / name).write_bytes(data)
        source_hashes[path] = sha(data)
    report = dict(source_revision=BASE, source_sha256=source_hashes,
                  compiler_sha256=sha(compiler.read_bytes()), configurations=[])
    for mode in ["optimized", "mir6502"]:
        for runtime in ["cart", "standalone"]:
            for name in ["TN", "TNDBG"]:
                stem = f"{name}-{mode}-{runtime}"
                image, listing = BUILD / f"{stem}.xex", BUILD / f"{stem}.asm"
                subprocess.run([str(compiler), "--mode", mode, "--runtime", runtime,
                    "-o", str(image), "--listing", str(listing), str(sources / f"{name}.ACT")],
                    cwd=ROOT, check=True, capture_output=True)
                text = listing.read_text().replace("\r\n", "\n")
                routines = re.findall(r"^; ===== PROC (\S+) \$([0-9A-F]+)\.\.\$([0-9A-F]+)(.*)$", text, re.M)
                rows = [dict(name=n, start=int(a, 16), end=int(b, 16), extra=extra.strip())
                        for n, a, b, extra in routines]
                storage, active = [], None
                for line in text.splitlines():
                    label = re.fullmatch(r"((?:global|param|local)_[^:]+):", line)
                    if label:
                        active = dict(name=label[1], start=None, bytes=0)
                        storage.append(active)
                    elif re.match(r"[A-Za-z_].*:", line) or line.startswith("; ===== PROC "):
                        active = None
                    elif active is not None:
                        raw = re.search(r"; \$([0-9A-F]+): ((?:[0-9A-F]{2} ?)+)$", line)
                        if raw:
                            if active["start"] is None:
                                active["start"] = int(raw[1], 16)
                            active["bytes"] += len(raw[2].split())
                report["configurations"].append(dict(name=stem, xex_bytes=len(image.read_bytes()),
                    image_sha256=sha(image.read_bytes()), listing_sha256=sha(text.encode()), routines=rows, storage=storage,
                    placement_lines=[line for line in text.splitlines() if re.match(
                        r"(?:global_|local_|param_).* = |; ===== DATA |; .*reserved|; .*storage", line)]))
    DOC.mkdir(parents=True, exist_ok=True)
    path = DOC / "unchecked.json"
    if check:
        expected = json.loads(path.read_text())
        # Binary identity is provenance, not a reproducible Rust build promise.
        report["compiler_sha256"] = expected["compiler_sha256"]
        assert report == expected, "unchecked controls changed"
    else:
        path.write_text(json.dumps(report, indent=2) + "\n")
    print("Verified eight unchecked TN controls" if check else "Froze eight unchecked TN controls")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="target/release/actionc")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generate(args.compiler, args.check)
