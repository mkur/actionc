#!/usr/bin/env python3
"""Measure address fixtures and optional full TN roots with a supplied compiler."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("tn_audit", Path(__file__).with_name("audit-current-output.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def measure(compiler, source, stem, output, mode="mir6502", runtime="cart"):
    listing, image = output / f"{stem}.asm", output / f"{stem}.xex"
    command = [str(compiler), "--mode", mode, "--runtime", runtime, "--listing", str(listing), "-o", str(image)]
    subprocess.run(command + [str(source)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    return audit.census(listing.read_text().replace("\r\n", "\n"), image.read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--roots", action="store_true")
    parser.add_argument("--crlf", action="store_true")
    args = parser.parse_args()
    compiler = args.compiler.resolve()
    output = args.out_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"compiler_sha256": hashlib.sha256(compiler.read_bytes()).hexdigest(), "fixtures": {}, "roots": {}}
    for name in ["known-record-address", "escaping-record-address", "escaping-record-address-wide"]:
        source = ROOT / "surveys/tn/reproducers" / f"{name}.act"
        source_text = source.read_text().replace("\r\n", "\n")
        for runtime in ["cart", "standalone"]:
            stem = f"{name}-{runtime}"
            metrics = measure(compiler, source, stem, output, runtime=runtime)
            if args.crlf:
                crlf = output / f"{name}-crlf.act"
                crlf.write_bytes(source_text.replace("\n", "\r\n").encode())
                measure(compiler, crlf, stem + "-crlf", output, runtime=runtime)
                assert (output / f"{stem}.xex").read_bytes() == (output / f"{stem}-crlf.xex").read_bytes(), stem
            selected = {r: {key: v[key] for key in ["instruction_form_bytes", "home_bytes", "static_home_operand_accesses", "calls", "pointer_scratch_writes", "indirect_y_operations"]}
                        for r, v in metrics["routines"].items() if r in ["Address", "ReadViaLocal", "ReadDirect", "AddressZero", "AddressField", "AddressExpanded", "ReadFixed", "ReadFixedWord", "ReadLocal", "WriteLocal", "ReadOrdinary", "FieldAddress"]}
            report["fixtures"][stem] = {"load_file_bytes": metrics["load_file_bytes"], "routines": selected, "lf_crlf_identical": args.crlf}
            print(stem, json.dumps(selected), flush=True)
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
                report["roots"][stem] = {"load_file_bytes": metrics["load_file_bytes"], "sha256": hashlib.sha256(blob).hexdigest(), "lf_crlf_identical": args.crlf,
                                        "instruction_form_bytes": metrics["instruction_form_bytes"],
                                        "directive_form_bytes": metrics["directive_form_bytes"],
                                        "BatchItem": metrics["routines"].get("BatchItem")}
                print(stem, len(blob), flush=True)
    (output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
