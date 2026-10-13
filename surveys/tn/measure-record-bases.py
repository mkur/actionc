#!/usr/bin/env python3
"""Measure generic record bases and retain typed MIR plus LF/CRLF evidence."""
import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("census", Path(__file__).with_name("audit-current-output.py"))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)
spec = importlib.util.spec_from_file_location("size_audit", Path(__file__).with_name("audit-size.py"))
size_audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(size_audit)


def measure_roots(args, output):
    sources = ROOT / "samples/tn/modern"
    destination = output / "roots"
    crlf = destination / "crlf"
    crlf.mkdir(parents=True, exist_ok=True)
    for source in sources.glob("*.ACT"):
        text = source.read_text().replace("\r\n", "\n")
        (destination / source.name).write_text(text)
        (crlf / source.name).write_bytes(text.replace("\n", "\r\n").encode())
    report = {}
    def compile_root(item):
        name, mode, ending = item
        stem = name + "-" + mode
        source = (crlf if ending else destination) / (name + ".ACT")
        with (destination / (stem + ending + ".log")).open("w") as log:
            subprocess.run([str(args.compiler.resolve()), "--mode", mode,
                            "--listing", str(destination / (stem + ending + ".asm")),
                            "-o", str(destination / (stem + ending + ".xex")), str(source)],
                           cwd=ROOT, check=True, stdout=log, stderr=log,
                           env=dict(os.environ, ACTIONC_MIR6502_PEEPHOLES="sites"))
    jobs = [(name, mode, ending) for name in ["TN", "TNDBG"]
            for mode in ["optimized", "mir6502"] for ending in ["", "-crlf"]]
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        list(pool.map(compile_root, jobs))
    for name in ["TN", "TNDBG"]:
        inventory = size_audit.source_inventory(destination, name)
        for mode in ["optimized", "mir6502"]:
            stem = name + "-" + mode
            blob = (destination / (stem + ".xex")).read_bytes()
            assert blob == (destination / (stem + "-crlf.xex")).read_bytes(), stem
            listing = (destination / (stem + ".asm")).read_text().replace("\r\n", "\n")
            current = size_audit.analyze(listing, blob, inventory)
            assert current == size_audit.analyze((destination / (stem + "-crlf.asm")).read_text(), blob, inventory)
            old = size_audit.analyze((args.baseline / (stem + ".asm")).read_text(),
                                     (args.baseline / (stem + ".xex")).read_bytes(), inventory)
            assert len(blob) <= old["load_file_bytes"], stem
            if mode == "optimized":
                assert blob == (args.baseline / (stem + ".xex")).read_bytes(), stem
            changed = {}
            for routine, after in current["routines"].items():
                before = old["routines"][routine]
                if before["code_bytes"] != after["code_bytes"]:
                    changed[routine] = {"before_code": before["code_bytes"], "after_code": after["code_bytes"]}
            remaining = [m for m in current["motifs"] if m["kind"] == "repeated_pair_setup"]
            report[stem] = {"before_file_bytes": old["load_file_bytes"], "after_file_bytes": len(blob),
                            "image_sha256": hashlib.sha256(blob).hexdigest(), "lf_crlf_identical": True,
                            "changed_routines": changed, "remaining_repeated_setups": remaining,
                            "remaining_full_setups": sum(m["kind"] == "pair_setup" for m in current["motifs"]),
                            "code_buckets": current["code_buckets"], "data_roles": current["data_roles"]}
            print(stem, old["load_file_bytes"], "->", len(blob), changed, flush=True)
    def report_root(item):
        name, policy = item
        subprocess.run([str(args.reporter.resolve()), str(destination / (name + ".ACT")),
                        str(destination / (name + "-typed-" + policy)), "cart", policy],
                       cwd=ROOT, check=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        list(pool.map(report_root, [(name, policy) for name in ["TN", "TNDBG"]
                                   for policy in ["off", "parameters"]]))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--reporter", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--controls", action="store_true", help="also compare off/read/write/parameter selection and raw/optimized NIR")
    parser.add_argument("--roots", action="store_true", help="also qualify TN/TNDBG and retain typed feature controls")
    parser.add_argument("--baseline", type=Path, help="qualified four-image baseline directory, required with --roots")
    parser.add_argument("--jobs", type=int, default=1, help="independent root compilation workers")
    args = parser.parse_args()
    if args.roots and args.baseline is None:
        parser.error("--roots requires --baseline")
    if args.jobs < 1:
        parser.error("--jobs must be positive")
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
        if args.controls:
            controls = {}
            for policy in ["off", "reads", "writes", "parameters"]:
                for raw in [False, True]:
                    key = policy + ("-raw" if raw else "")
                    prefix = output / (stem + "-" + key)
                    for ending, path in [("", source), ("-crlf", crlf)]:
                        command = [str(args.reporter.resolve()), str(path), str(prefix) + ending, runtime, policy]
                        if raw:
                            command.append("raw")
                        subprocess.run(command, cwd=ROOT, check=True)
                    for suffix in ["pre.mir", "post.mir", "bin", "json"]:
                        assert prefix.with_suffix("." + suffix).read_bytes() == Path(str(prefix) + "-crlf." + suffix).read_bytes(), (key, suffix)
                    item = json.loads(prefix.with_suffix(".json").read_text())
                    item["payload_sha256"] = hashlib.sha256(prefix.with_suffix(".bin").read_bytes()).hexdigest()
                    item["lf_crlf_identical"] = True
                    controls[key] = item
            report["cases"][runtime]["controls"] = controls
    (output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    if args.roots:
        report["roots"] = measure_roots(args, output)
        (output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    print({k:v["load_file_bytes"] for k,v in report["cases"].items()})


if __name__ == "__main__":
    main()
