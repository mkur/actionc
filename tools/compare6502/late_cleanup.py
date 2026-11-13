#!/usr/bin/env python3
"""Qualify terminal cleanup against frozen phase-tracing artifacts (never overwrite them)."""
import argparse
import hashlib
import json
from pathlib import Path
try:
    import resource
except ImportError:  # Windows has no getrusage.
    resource = None
import sys
import tempfile

from phase_trace import ROOT, run, read_bundle, controls


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, default=ROOT / "build/checked-arena/shared-target/debug/actionc-emit")
    parser.add_argument("--reference", type=Path, default=ROOT / "build/mir6502-phase-trace/baseline")
    parser.add_argument("--reference-compiler", type=Path, help="regenerate references in memory with this old compiler, checking frozen hashes")
    parser.add_argument("--output", type=Path, default=ROOT / "build/mir6502-late-cleanup/qualification")
    parser.add_argument("--resume", type=Path, help="continue an incomplete run after validating its completed artifact prefix")
    parser.add_argument("--api-rlib", type=Path)
    parser.add_argument("--baseline-receipt", type=Path, help="compare against a previous late-cleanup qualification; also isolate producer cleanup in API checks")
    parser.add_argument("--mads", help="MADS executable; omit to skip listing roundtrips")
    args = parser.parse_args()
    compiler = args.compiler.resolve()
    frozen = json.loads((ROOT / "docs/benchmarks/6502-phase-tracing/baseline.json").read_text())
    previous = json.loads((ROOT / "docs/benchmarks/6502-phase-tracing/qualification.json").read_text())
    if args.baseline_receipt:
        previous = json.loads(args.baseline_receipt.read_text())
        old_cases = {c["id"]: c for c in previous["cases"]}
        frozen["revision"] = previous["implementation_revision"]
        for case in frozen["cases"]:
            old = old_cases[case["id"]]
            case["artifacts"] = {suffix: a["after_sha256"] for suffix, a in old["artifacts"].items()}
            case["reference_directory"] = old.get("directory", case["id"])
    old_work = {c["id"]: c["work"] for c in previous["cases"]}
    args.output.mkdir(parents=True, exist_ok=True)
    work = args.resume.resolve() if args.resume else Path(tempfile.mkdtemp(prefix="run-", dir=args.output)).resolve()
    revision, _ = run(["git", "rev-parse", "HEAD"])
    report = dict(baseline_revision=frozen["revision"], implementation_revision=revision.decode().strip(), compiler_sha256=sha(compiler.read_bytes()), cases=[])
    if args.reference_compiler:
        report["reference_compiler_sha256"] = sha(args.reference_compiler.read_bytes())
    if args.resume:
        saved = json.loads((work / "qualification.json").read_text())
        assert saved["baseline_revision"] == report["baseline_revision"]
        assert saved["compiler_sha256"] == report["compiler_sha256"]
        assert "api" not in saved, "run is already complete"
        report = saved
    completed = len(report["cases"])
    for index, case in enumerate(frozen["cases"]):
        assert sha((ROOT / case["source"]).read_bytes().replace(b"\r\n", b"\n")) == case["source_sha256"], case["id"]
        if index < completed:
            saved = report["cases"][index]
            assert saved["id"] == case["id"]
            directory = work / saved.get("directory", case["id"])
            for suffix, artifact in saved["artifacts"].items():
                assert sha((directory / f"output.{suffix}").read_bytes()) == artifact["after_sha256"]
                assert artifact["before_sha256"] == case["artifacts"][suffix]
            read_bundle(directory / "trace-xex")
            continue
        directory = work / case["id"]
        if directory.exists():
            directory = Path(tempfile.mkdtemp(prefix=case["id"] + "-retry-", dir=work))
        else:
            directory.mkdir()
        command = [compiler, "--profile", case["profile"], "--backend", "mir6502", "--runtime", case["runtime"], "--origin", case["origin"]]
        row = dict(id=case["id"], directory=directory.relative_to(work).as_posix(), artifacts={}, seconds={})
        journals = []
        for suffix, mode in [("xex", "--emit-load"), ("asm", "--emit-listing"), ("map", "--emit-map")]:
            if args.reference_compiler:
                before, _ = run([args.reference_compiler.resolve()] + command[1:] + [mode, case["source"]])
            else:
                before = (args.reference / case.get("reference_directory", case["id"]) / f"output.{suffix}").read_bytes()
            assert sha(before) == case["artifacts"][suffix]
            plain, plain_time = run(command + [mode, case["source"]])
            bundle = directory / f"trace-{suffix}"
            flags = ["--trace-mir6502", bundle]
            if case["name"] == "tn":
                flags += ["--trace-routine", "FormatDirectoryRow", "--trace-routine", "SetWin"]
            traced, traced_time = run(command + [mode] + flags + [case["source"]])
            assert plain == traced, (case["id"], suffix, "trace changed output")
            (directory / f"output.{suffix}").write_bytes(traced)
            row["artifacts"][suffix] = dict(before_sha256=sha(before), after_sha256=sha(traced), before_bytes=len(before), after_bytes=len(traced))
            row["seconds"][suffix] = dict(plain=plain_time, traced=traced_time)
            journals.append(sha((bundle / "events.jsonl").read_bytes()))
        assert len(set(journals)) == 1, "output modes selected different pipelines"
        manifest, events = read_bundle(directory / "trace-xex")
        result = next((e for e in events if e["kind"] == "late-cleanup-result"), None)
        row.update(work=manifest["work"], previous_work=old_work[case["id"]], detail_bytes=manifest["detail_bytes"],
                   result=result["data"] if result else dict(accepted=False, reason="feature-disabled"), observations=controls(events))
        assert row["work"].get("materializations") == row["previous_work"].get("materializations")
        assert row["work"]["emission_invocations"] - row["previous_work"]["emission_invocations"] <= 2
        if result:
            requests = [e for e in events if e["kind"] == "attempt-start" and e["data"].get("parent") == result["attempt"] and e["data"]["stage"] == "emission-request"]
            assert len(requests) <= 2
            row["terminal_cost_emissions"] = len(requests)
        assert row["artifacts"]["xex"]["after_bytes"] <= row["artifacts"]["xex"]["before_bytes"]
        if row["result"]["accepted"]:
            snapshots = {e["data"]["stage"]: e["data"] for e in events if e["kind"] == "snapshot" and e["attempt"] == result["attempt"]}
            assert snapshots["attempt-input"]["frames"] == snapshots["proposed"]["frames"]
        if case["name"] == "tn":
            assert row["result"]["accepted"], row["result"]
            final = next(e["data"] for e in reversed(events) if e["kind"] == "snapshot" and e["data"]["stage"] == "pre-emission")
            checked = []
            for rid, slots in [(72, [12, 13, 26, 27]), (73, [110, 111])]:
                homes = next(r["homes"] for r in final["accesses"] if r["routine"] == rid)
                frame = next(r for r in final["frames"] if r["routine"] == rid)
                for slot in slots:
                    h = next(h for h in homes if h["home"]["kind"] == "spill" and h["home"]["id"] == slot and h["home"]["offset"] == 0)
                    assert h["counts"]["writes"] == h["counts"]["reads"] == 0, h
                    assert any(h["id"] == slot for h in frame["spills"]), frame
                    checked.append(dict(routine=rid, spill=slot))
            row["tn_removed_stores_retained_homes"] = checked
        if args.mads and case["name"] in ("tn", "word", "ranges"):
            for ending in ["lf", "crlf"]:
                listing = (directory / "output.asm").read_bytes().replace(b"\r\n", b"\n")
                asm = directory / f"roundtrip-{ending}.asm"
                image = directory / f"roundtrip-{ending}.xex"
                asm.write_bytes(listing if ending == "lf" else listing.replace(b"\n", b"\r\n"))
                run([args.mads, asm, f"-o:{image}", "-s"])
                assert image.read_bytes() == (directory / "output.xex").read_bytes()
            row["mads_lf_crlf"] = "equal"
        report["cases"].append(row)
        (work / "qualification.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"{case['id']}: {row['artifacts']['xex']['before_bytes']} -> {row['artifacts']['xex']['after_bytes']}; {row['result']}", flush=True)
    rlib = args.api_rlib or max((compiler.parent / "deps").glob("libactionc-*.rlib"), key=lambda p: p.stat().st_mtime)
    probe = work / ("api-probe.exe" if sys.platform == "win32" else "api-probe")
    run(["rustc", "--edition=2024", ROOT / "tools/compare6502/late_cleanup_api.rs", "--extern", f"actionc={rlib}", "-L", f"dependency={rlib.parent}", "-o", probe])
    output, elapsed = run([probe, work / "api"] + (["--producer-only"] if args.baseline_receipt else []))
    report["api"] = [json.loads(line) for line in output.decode().splitlines()]
    report["api_seconds"] = elapsed
    report["peak_child_rss_bytes"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss * (1 if sys.platform == "darwin" else 1024) if resource else None
    report["rss_scope"] = "maximum child RSS across CLI compiler, rustc API-probe build, API probe and MADS"
    if args.resume:
        report["rss_scope"] += f"; resumed invocation excludes {completed} previously completed CLI cases"
    (work / "qualification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(work / "qualification.json")


if __name__ == "__main__":
    main()
