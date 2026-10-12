#!/usr/bin/env python3
"""Measure comparison fixtures and intact TN roots, including CRLF compilation."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tn_audit", ROOT / "surveys/tn/audit-current-output.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    compiler, output = args.compiler.resolve(), args.out_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"compiler_sha256": hashlib.sha256(compiler.read_bytes()).hexdigest(), "fixtures": {}, "roots": {}}

    def measure(source, stem, mode, runtime):
        image, listing = output / f"{stem}.xex", output / f"{stem}.asm"
        subprocess.run([str(compiler), "--mode", mode, "--runtime", runtime,
                        "--listing", str(listing), "-o", str(image), str(source)],
                       cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        blob = image.read_bytes()
        metrics = audit.census(listing.read_text().replace("\r\n", "\n"), blob)
        return {"load_file_bytes": len(blob), "sha256": hashlib.sha256(blob).hexdigest(),
                "routines": metrics["routines"]}, blob

    source = ROOT / "surveys/tn/reproducers/comparison-loops.act"
    crlf = output / "comparison-loops-crlf.act"
    crlf.write_bytes(source.read_text().replace("\r\n", "\n").replace("\n", "\r\n").encode())
    report["source_sha256_lf"] = hashlib.sha256(source.read_text().replace("\r\n", "\n").encode()).hexdigest()
    for runtime in ["cart", "standalone"]:
        stem = f"comparison-loops-{runtime}"
        metrics, blob = measure(source, stem, "mir6502", runtime)
        _, converted = measure(crlf, stem + "-crlf", "mir6502", runtime)
        assert blob == converted, stem
        report["fixtures"][stem] = {**metrics, "lf_crlf_identical": True}
        print(stem, metrics["load_file_bytes"], flush=True)

    sources = ROOT / "samples/tn/modern"
    crlf_sources = output / "crlf-roots"
    crlf_sources.mkdir(exist_ok=True)
    digest = hashlib.sha256()
    for source in sorted(sources.glob("*.ACT")):
        text = source.read_text().replace("\r\n", "\n")
        digest.update(source.name.encode() + b"\0" + text.encode() + b"\0")
        crlf_sources.joinpath(source.name).write_bytes(text.replace("\n", "\r\n").encode())
    report["source_tree_sha256_lf"] = digest.hexdigest()
    baseline = json.loads((Path(__file__).with_name("stage0.json")).read_text())
    for name in ["TN", "TNDBG"]:
        for mode in ["optimized", "mir6502"]:
            stem = f"{name}-{mode}"
            metrics, blob = measure(sources / f"{name}.ACT", stem, mode, "cart")
            _, converted = measure(crlf_sources / f"{name}.ACT", stem + "-crlf", mode, "cart")
            assert blob == converted, stem
            assert metrics["load_file_bytes"] <= baseline["roots"][stem]["load_file_bytes"], stem
            if mode == "optimized":
                assert metrics["sha256"] == baseline["roots"][stem]["sha256"], stem
            report["roots"][stem] = {key: metrics[key] for key in ["load_file_bytes", "sha256"]}
            report["roots"][stem].update(MyDosAfter=metrics["routines"]["MyDosAfter"], lf_crlf_identical=True)
            print(stem, metrics["load_file_bytes"], flush=True)
    (output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
