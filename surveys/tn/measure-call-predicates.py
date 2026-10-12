#!/usr/bin/env python3
"""Measure call-predicate fixtures and intact TN roots with a supplied compiler."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("tn_audit", Path(__file__).with_name("audit-current-output.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def measure(compiler, source, stem, output, mode="mir6502", runtime="cart"):
    listing, image = output / f"{stem}.asm", output / f"{stem}.xex"
    subprocess.run([str(compiler), "--mode", mode, "--runtime", runtime,
                    "--listing", str(listing), "-o", str(image), str(source)],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    return audit.census(listing.read_text().replace("\r\n", "\n"), image.read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--roots", action="store_true")
    parser.add_argument("--crlf", action="store_true")
    args = parser.parse_args()
    compiler, output = args.compiler.resolve(), args.out_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"compiler_sha256": hashlib.sha256(compiler.read_bytes()).hexdigest(), "fixtures": {}, "roots": {}}
    for fixture in ["captured-call-arguments", "call-predicate-snapshots"]:
        source = ROOT / "surveys/tn/reproducers" / f"{fixture}.act"
        for runtime in ["cart", "standalone"]:
            stem = f"{fixture}-{runtime}"
            metrics = measure(compiler, source, stem, output, runtime=runtime)
            if args.crlf:
                crlf = output / f"{fixture}-crlf.act"
                crlf.write_bytes(source.read_text().replace("\r\n", "\n").replace("\n", "\r\n").encode())
                measure(compiler, crlf, stem + "-crlf", output, runtime=runtime)
                assert (output / f"{stem}.xex").read_bytes() == (output / f"{stem}-crlf.xex").read_bytes(), stem
            report["fixtures"][stem] = {**metrics, "lf_crlf_identical": args.crlf}
            print(stem, metrics["load_file_bytes"], flush=True)
    if args.roots:
        sources = ROOT / "samples/tn/modern"
        crlf_sources = output / "crlf-roots"
        if args.crlf:
            crlf_sources.mkdir(exist_ok=True)
            for source in sources.glob("*.ACT"):
                crlf_sources.joinpath(source.name).write_bytes(source.read_text().replace("\r\n", "\n").replace("\n", "\r\n").encode())
        for name in ["TN", "TNDBG"]:
            for mode in ["optimized", "mir6502"]:
                stem = f"{name}-{mode}"
                metrics = measure(compiler, sources / f"{name}.ACT", stem, output, mode=mode)
                blob = (output / f"{stem}.xex").read_bytes()
                if args.crlf:
                    measure(compiler, crlf_sources / f"{name}.ACT", stem + "-crlf", output, mode=mode)
                    assert blob == (output / f"{stem}-crlf.xex").read_bytes(), stem
                report["roots"][stem] = {key: metrics[key] for key in ["load_file_bytes", "instruction_form_bytes", "directive_form_bytes"]}
                report["roots"][stem].update(sha256=hashlib.sha256(blob).hexdigest(), lf_crlf_identical=args.crlf,
                                             SetTagged=metrics["routines"]["SetTagged"], BatchItem=metrics["routines"]["BatchItem"])
                print(stem, len(blob), flush=True)
    (output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
