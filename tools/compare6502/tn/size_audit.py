#!/usr/bin/env python3
"""Measure and qualify TN's size-first source changes against frozen sources."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
from concurrent.futures import ThreadPoolExecutor

import window_frames as harness

ROOT = harness.ROOT
BUILD = ROOT / "build/tn-size"
BASE = "5d002c27"
DOC = ROOT / "docs/benchmarks/6502-tn-size"
NAMES = ["TN.ACT", "TNDBG.ACT", "LIB.ACT"]


def run(command, env, log):
    with log.open("w") as out:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=out, stderr=subprocess.STDOUT)
    if result.returncode:
        raise SystemExit(f"Check failed; see {log}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, default=ROOT / "samples/tn/modern")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--ui", action="store_true")
    parser.add_argument("--mads", action="store_true")
    args = parser.parse_args()
    BUILD.mkdir(parents=True, exist_ok=True)
    hashes = {}
    for variant in ["before", "after"]:
        directory = BUILD / variant
        directory.mkdir(exist_ok=True)
        for name in NAMES:
            data = (subprocess.check_output(["git", "show", f"{BASE}:samples/tn/modern/{name}"], cwd=ROOT)
                    if variant == "before" else (args.source_dir / name).read_bytes())
            data = data.replace(b"\r\n", b"\n")
            (directory / name).write_bytes(data)
            hashes[f"{variant}/{name}"] = hashlib.sha256(data).hexdigest()
    # Reuse the maintained complete-program oracles and their image bindings.
    harness.BUILD = BUILD
    env = harness.prepare_probe()
    deps = harness.VM / "target/release/deps"
    command = ["rustc", "--edition=2024", "--test", "-O", "--crate-name", "tn_size_audit",
               str(Path(__file__).with_name("size_audit_probe.rs")), "-L", f"dependency={deps}",
               "-o", str(BUILD / "size_probe")]
    for crate in ["actionc", "actionc_vm", "serde_json"]:
        paths = list(deps.glob(f"lib{crate}-*.rlib"))
        assert len(paths) == 1, (crate, paths)
        command += ["--extern", f"{crate}={paths[0]}"]
    subprocess.run(command, cwd=ROOT, env=env, check=True)
    env = dict(env, ACTIONC_TN_SIZE_BUILD=str(BUILD))
    run([str(BUILD / "size_probe"), "--exact", "compile_images", "--nocapture"], env, BUILD / "compile.log")
    run([str(BUILD / "size_probe"), "--exact", "check_machine_interfaces", "--nocapture"], env, BUILD / "interfaces.log")
    if args.ui:
        def check_ui(variant):
            for test in ["checked::real_arena_success_and_window_checkpoints_with_lf_and_crlf",
                         "dispatch::tn_dispatch_and_panel_state_preserve_behavior"]:
                run([str(BUILD / "probe"), "--exact", test, "--nocapture"],
                    dict(env, ACTIONC_TN_FRAME_SOURCES=str(BUILD / variant)),
                    BUILD / f"{variant}-{test.split('::')[0]}.log")
        with ThreadPoolExecutor(max_workers=2) as pool:
            list(pool.map(check_ui, ["before", "after"]))
    if args.mads:
        count = 0
        for variant in ["before", "after"]:
            for asm in sorted((BUILD / variant).glob("*.asm")):
                if ".roundtrip." in asm.name:
                    continue
                listing = asm.read_text().replace("\r\n", "\n")
                for crlf in [False, True]:
                    source, image = asm.with_suffix(".roundtrip.asm"), asm.with_suffix(".roundtrip.xex")
                    source.write_bytes((listing.replace("\n", "\r\n") if crlf else listing).encode())
                    result = subprocess.run([os.environ.get("ACTIONC_MADS", "mads"), str(source), f"-o:{image}", "-s"], capture_output=True, text=True)
                    assert result.returncode == 0, result.stdout + result.stderr
                    assert image.read_bytes() == asm.with_suffix(".xex").read_bytes()
                    source.unlink()
                    image.unlink()
                    count += 1
        assert count == 32, count
        print(f"MADS: {count} identical roundtrips")
    raw = json.loads((BUILD / "images.json").read_text())
    rows = []
    for before in [r for r in raw if r["variant"] == "before"]:
        after = next(r for r in raw if r["variant"] == "after" and all(r[k] == before[k] for k in ["program", "mode", "runtime"]))
        assert after["xex_bytes"] < before["xex_bytes"]
        assert after["reserved_bytes"] < before["reserved_bytes"]
        routines = {n: [before["routine_bytes"].get(n, 0), after["routine_bytes"].get(n, 0)]
                    for n in sorted(set(before["routine_bytes"]) | set(after["routine_bytes"]))
                    if before["routine_bytes"].get(n) != after["routine_bytes"].get(n)}
        rows.append({**{k: before[k] for k in ["program", "mode", "runtime"]},
                     "xex_bytes": [before["xex_bytes"], after["xex_bytes"]],
                     "reserved_bytes": [before["reserved_bytes"], after["reserved_bytes"]],
                     "copy_buffer": [before["copy_buffer"], after["copy_buffer"]],
                     "routine_changes": routines,
                     "image_sha256": {v: hashlib.sha256((BUILD / v / f'{before["program"]}-{before["mode"]}-{before["runtime"]}.xex').read_bytes()).hexdigest() for v in ["before", "after"]}})
    result = dict(schema=1, baseline_source_revision=BASE, source_sha256=hashes, measurements=rows)
    path = DOC / "results.json"
    if args.check:
        assert result == json.loads(path.read_text()), "TN size receipts changed"
    else:
        DOC.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(result, indent=2) + "\n")
    print(f"TN size audit {'checked' if args.check else 'recorded'}")


if __name__ == "__main__":
    main()
