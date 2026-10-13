#!/usr/bin/env python3
"""Compile isolated bitmap traversal alternatives with one pinned compiler."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
VARIANTS = ["baseline", "cached", "byte_shift", "byte_nibbles", "cached_bound", "byte_nibbles_bound"]
spec = importlib.util.spec_from_file_location("tn_audit", ROOT / "surveys/tn/audit-current-output.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def alternative(original, fixture, variant):
    source = original.replace("\r\n", "\n")
    fixture = fixture.replace("\r\n", "\n")
    if variant in ["cached", "byte_shift", "byte_nibbles", "byte_nibbles_bound"]:
        name = {"cached": "CachedBits", "byte_shift": "ByteShift",
                "byte_nibbles": "ByteNibbles", "byte_nibbles_bound": "ByteNibbles"}[variant]
        body = fixture.split(f"PROC {name}(BitSpan POINTER span)\n", 1)[1].split("\nRETURN\n", 1)[0]
        body = body.replace("span", "tags").replace(".data", ".bits").replace(".limit", ".extent").replace(".total", ".tagged").replace("masks(", "tagMasks(")
        start, end = source.index("PROC TagsRecount("), source.index("\n; extent is")
        source = source[:start] + "PROC TagsRecount(Selection POINTER tags)\n" + body + "\nRETURN\n" + source[end:]
        if "byte_nibbles" in variant:
            source = source.replace("TYPE DirEntry=[", "BYTE ARRAY nibbleCounts(16)=[0 1 1 2 1 2 2 3 1 2 2 3 2 3 3 4]\n\nTYPE DirEntry=[", 1)
    if variant in ["cached_bound", "byte_nibbles_bound"]:
        start, end = source.index("PROC TagsExtent("), source.index("PROC ToggleAllTags(")
        body = source[start:end]
        if "byteLimit=capacity RSH 3\n" in body and "WHILE ordinal<byteLimit DO" in body:
            return source
        assert body.count("CARD ordinal\n") == 1
        assert body.count("WHILE ordinal<(capacity RSH 3) DO") == 1
        body = body.replace("CARD ordinal\n", "CARD ordinal,byteLimit\n", 1)
        body = body.replace("IF tags.complete AND currentExtent<capacity THEN\n", "IF tags.complete AND currentExtent<capacity THEN\n    byteLimit=capacity RSH 3\n", 1)
        body = body.replace("WHILE ordinal<(capacity RSH 3) DO", "WHILE ordinal<byteLimit DO", 1)
        source = source[:start] + body + source[end:]
    return source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--source-dir", type=Path, default=ROOT / "samples/tn/modern")
    parser.add_argument("--variants", nargs="+", choices=VARIANTS, default=VARIANTS)
    parser.add_argument("--roots", nargs="+", choices=["TN", "TNDBG"], default=["TN"])
    parser.add_argument("--crlf", action="store_true")
    args = parser.parse_args()
    compiler, output = args.compiler.resolve(), args.out_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    sources = args.source_dir.resolve()
    fixture = ROOT / "surveys/tn/reproducers/bitmap-scans.act"
    original, text = (sources / "DIR.ACT").read_text(), fixture.read_text()
    report = {"compiler_sha256": hashlib.sha256(compiler.read_bytes()).hexdigest(),
              "fixture_sha256_lf": hashlib.sha256(text.replace("\r\n", "\n").encode()).hexdigest(),
              "directory_sha256_lf": hashlib.sha256(original.replace("\r\n", "\n").encode()).hexdigest(),
              "generic": {}, "roots": {}}

    def measure(source, stem, mode, runtime):
        listing, image = output / f"{stem}.asm", output / f"{stem}.xex"
        subprocess.run([str(compiler), "--mode", mode, "--runtime", runtime, "--listing", str(listing), "-o", str(image), str(source)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        metrics = audit.census(listing.read_text().replace("\r\n", "\n"), image.read_bytes())
        return {"load_file_bytes": len(image.read_bytes()), "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
                "routines": metrics["routines"]}

    for variant in args.variants:
        directory = output / variant
        directory.mkdir(exist_ok=True)
        transformed = alternative(original, text, variant)
        assert transformed == alternative(original.replace("\n", "\r\n"), text.replace("\n", "\r\n"), variant)
        for source in sources.glob("*.ACT"):
            content = transformed if source.name == "DIR.ACT" else source.read_text().replace("\r\n", "\n")
            (directory / source.name).write_text(content)
        report["roots"][variant] = {}
        for name in args.roots:
            for mode in ["optimized", "mir6502"]:
                stem = f"{variant}/{name}-{mode}"
                result = measure(directory / f"{name}.ACT", stem, mode, "cart")
                if args.crlf:
                    converted = directory / "crlf"
                    converted.mkdir(exist_ok=True)
                    for source in directory.glob("*.ACT"):
                        (converted / source.name).write_bytes(source.read_text().replace("\n", "\r\n").encode())
                    other = measure(converted / f"{name}.ACT", stem + "-crlf", mode, "cart")
                    assert result["sha256"] == other["sha256"], stem
                report["roots"][variant][f"{name}-{mode}"] = {"load_file_bytes": result["load_file_bytes"], "sha256": result["sha256"], "lf_crlf_identical": args.crlf,
                    "routines": {n: result["routines"][n] for n in ["TagBitInRange", "TagsRecount", "TagsExtent"]}}
                print(stem, result["load_file_bytes"], flush=True)
    for mode in ["optimized", "mir6502"]:
        for runtime in ["cart", "standalone"]:
            stem = f"generic-{mode}-{runtime}"
            report["generic"][stem] = measure(fixture, stem, mode, runtime)
    (output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
