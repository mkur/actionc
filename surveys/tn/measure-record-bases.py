#!/usr/bin/env python3
"""Measure generic record bases and retain typed MIR plus LF/CRLF evidence."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("census", Path(__file__).with_name("audit-current-output.py"))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--reporter", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    output = args.out_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    source = ROOT / "surveys/tn/reproducers/record-base-reuse.act"
    text = source.read_text().replace("\r\n", "\n")
    crlf = output / "record-base-reuse-crlf.act"
    crlf.write_bytes(text.replace("\n", "\r\n").encode())
    report = {"compiler_sha256": hashlib.sha256(args.compiler.read_bytes()).hexdigest(),
              "reporter_sha256": hashlib.sha256(args.reporter.read_bytes()).hexdigest(),
              "source_sha256_lf": hashlib.sha256(text.encode()).hexdigest(), "cases": {}}
    for runtime in ["cart", "standalone"]:
        stem = "record-bases-" + runtime
        for ending, path in [("", source), ("-crlf", crlf)]:
            with (output / (stem + ending + ".log")).open("w") as log:
                subprocess.run([str(args.compiler.resolve()), "--mode", "mir6502", "--runtime", runtime,
                                "--listing", str(output / (stem + ending + ".asm")), "-o",
                                str(output / (stem + ending + ".xex")), str(path)], cwd=ROOT, stdout=log, stderr=log, check=True)
            subprocess.run([str(args.reporter.resolve()), str(path), str(output / (stem + ending)), runtime], cwd=ROOT, check=True)
        blob = (output / (stem + ".xex")).read_bytes()
        assert blob == (output / (stem + "-crlf.xex")).read_bytes()
        for suffix in ["pre.mir", "post.mir", "bin", "json"]:
            assert (output / (stem + "." + suffix)).read_bytes() == (output / (stem + "-crlf." + suffix)).read_bytes(), suffix
        measured = census.census((output / (stem + ".asm")).read_text().replace("\r\n", "\n"), blob)
        report["cases"][runtime] = {"load_file_bytes":len(blob), "image_sha256":hashlib.sha256(blob).hexdigest(),
                                    "lf_crlf_identical":True, "routines":measured["routines"]}
    (output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    print({k:v["load_file_bytes"] for k,v in report["cases"].items()})


if __name__ == "__main__":
    main()
