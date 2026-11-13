#!/usr/bin/env python3
"""Qualify diagnostic-only MIR6502 changes against frozen pre-change artifacts.

Run with an already built actionc-emit. --check validates all hashes, complete
journals and the six TN observations; bundles stay in a fresh ignored build run.
Timings and child RSS are measurements, never pass/fail thresholds.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
BASELINE = ROOT / "docs/benchmarks/6502-phase-tracing/baseline.json"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(args):
    start = time.perf_counter()
    env = dict(os.environ)
    env.pop("ACTIONC_MIR6502_PEEPHOLES", None)
    result = subprocess.run([str(x) for x in args], cwd=ROOT, env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    return result.stdout, time.perf_counter() - start


def read_bundle(directory):
    manifest = json.loads((directory / "manifest.json").read_text())
    assert manifest["status"] == "complete", manifest
    events = [json.loads(line) for line in (directory / "events.jsonl").read_text().splitlines()]
    by_id = {e["id"]: e for e in events}
    assert len(by_id) == len(events)
    assert list(by_id) == sorted(by_id)
    attempts = {e["attempt"]: e for e in events if e["kind"] == "attempt-start"}
    ends = collections.Counter(e["attempt"] for e in events if e["kind"] == "attempt-end")
    for attempt, start in attempts.items():
        if start["data"].get("scope") == "attempt":
            assert ends[attempt] == 1, (attempt, start["data"], ends[attempt])
    generations = {(e["attempt"],e["data"]["routine"],e["data"]["generation"]): {b["block"]:b["ops"] for b in e["data"]["blocks"]} for e in events if e["kind"] == "rewrite-generation"}
    for e in events:
        d = e["data"]
        if e["kind"] == "rewrite":
            blocks = generations[e["attempt"],d["routine"],d["generation"]]
            assert 0 <= d["range"][0] <= d["range"][1] <= blocks[d["block"]]

        assert e["attempt"] == 0 or e["attempt"] in attempts
        if e["kind"] == "attempt-start":
            assert d["parent"] < e["attempt"]
            assert d["parent"] == 0 or d["parent"] in attempts
        if e["kind"] == "attempt-end" and d.get("returned_snapshot") is not None:
            returned = by_id[d["returned_snapshot"]]
            if d.get("selection_event") is not None:
                assert by_id[d["selection_event"]]["attempt"] == e["attempt"]
            assert returned["attempt"] == e["attempt"]
            for source in d["equivalent_source_snapshots"]:
                assert by_id[source]["data"]["program_identity"] == returned["data"]["program_identity"]
                assert source < returned["id"]
        if e["kind"] == "home-remap":
            assert d["from_snapshot"] in by_id and d["to_snapshot"] in by_id
            assert by_id[d["from_snapshot"]]["attempt"] == e["attempt"]
        if "mir_file" in d:
            assert (directory / d["mir_file"]).is_file()
    final = by_id[manifest["final_snapshot"]]
    assert final["data"]["stage"] == "pre-emission"
    assert by_id[manifest["final_layout"]]["data"]["snapshot"] == final["id"]
    assert isinstance(by_id[manifest["final_layout"]]["data"]["storage_symbols"], list)
    blocks = {(r["id"], b["id"]): b["ops"] for r in final["data"]["routines"] for b in r["blocks"]}
    ranges = [e for e in events if e["kind"] == "emitted-range"]
    assert len(ranges) == sum(n + 1 for n in blocks.values())
    points = set()
    for e in ranges:
        d = e["data"]
        assert d["snapshot"] == final["id"]
        assert d["op"] is None or d["op"] < blocks[d["routine"], d["block"]]
        assert d["offset_start"] <= d["offset_end"]
        point = d["routine"], d["block"], d["op"]
        assert point not in points
        points.add(point)
    return manifest, events


def access(snapshot, routine, home):
    for r in snapshot["data"]["accesses"]:
        if r["routine"] == routine:
            return next((h["counts"] for h in r["homes"] if h["home"] == home), None)
    return None


def tn_diagnosis(events, origin):
    """The reviewed IDs are investigation inputs only, never compiler decisions."""
    final = next(e for e in events if e["kind"] == "snapshot" and e["data"]["stage"] == "pre-emission")
    names = {r["name"]: r["id"] for r in final["data"]["routines"]}
    starts = {e["attempt"]: e["data"] for e in events if e["kind"] == "attempt-start"}
    # T3 is directly on the returned pipeline. Find its scoped input and output.
    transaction = next(a for a, d in starts.items() if d["stage"] == "pointer_retention")
    snapshots = {e["data"]["stage"]: e for e in events if e["attempt"] == transaction and e["kind"] == "snapshot"}
    before, after = snapshots["attempt-input"], snapshots["attempt-return"]
    assert before["data"]["frames"] == after["data"]["frames"]
    selection = next(e for e in events if e["attempt"] == transaction and e["kind"] == "selection")
    assert selection["data"]["outcome"] == "accepted"
    rows = []
    for name, ids in [("FormatDirectoryRow", [12, 13, 26, 27]), ("SetWin", [110, 111])]:
        routine = names[name]
        assert routine in selection["data"]["accepted_routines"]
        for spill in ids:
            home = dict(kind="spill", routine=routine, id=spill, offset=0)
            first, later, last = (access(s, routine, home) for s in (before, after, final))
            assert first["reads"] == 1 and later["reads"] == 0 and last["reads"] == 0
            assert first["writes"] == later["writes"] == last["writes"] == 1
            stops = [e for e in events if e["kind"] == "compat-spill-stop" and e["data"]["home"] == home]
            stop = stops[-1]
            assert stop["data"]["reason"] == "explicit-read"
            assignment = next(e for e in reversed(events) if e["kind"] == "home-assignment" and e["data"]["planned_home"] == home)
            assert assignment["data"]["plan"] == "coupled"
            demand = next(e for e in events if e["attempt"] == assignment["attempt"] and e["kind"] == "home-plan" and e["data"]["routine"] == routine and e["data"]["temp"] == assignment["data"]["temp"] and e["data"]["byte"] == assignment["data"]["byte"])
            assert demand["data"]["consumer_kind"] == "materialize-address"
            placement = next(e for e in events if e["kind"] == "final-home" and e["data"]["home"] == home)
            point = last["write_points"][0]
            emitted = next(e for e in events if e["kind"] == "emitted-range" and e["data"]["routine"] == routine and e["data"]["block"] == point["block"] and e["data"]["op"] == point["op"])
            assert emitted["data"]["offset_end"] - emitted["data"]["offset_start"] == 3
            assert placement["data"]["bytes"] == 1
            assert not any(e["kind"] in ("compat-spill-outcome", "home-prune") and e["id"] > after["id"] for e in events)
            rows.append(dict(routine=name, home=home, temp=assignment["data"]["temp"], lane=assignment["data"]["byte"],
                             demand_event=demand["id"], assignment_event=assignment["id"], cleanup_stop_event=stop["id"],
                             last_consumer=first["read_points"], before_snapshot=before["id"], after_snapshot=after["id"],
                             selection_event=selection["id"], final_snapshot=final["id"], physical_address=placement["data"]["address"],
                             store_address=origin + emitted["data"]["offset_start"], store_bytes=3, allocated_bytes=1,
                             cause="pointer retention removes last explicit reader after core cleanup; frame preserved"))
    return rows


def controls(events):
    final = next(e for e in events if e["kind"] == "snapshot" and e["data"]["stage"] == "pre-emission")
    private = [h for r in final["data"]["accesses"] for h in r["homes"] if h["home"]["kind"] in ("spill", "virtual-zp")]
    empty_ranges = {(e["data"]["routine"], e["data"]["block"], e["data"]["op"]) for e in events if e["kind"] == "emitted-range" and e["data"]["offset_start"] == e["data"]["offset_end"]}
    empty_reads = {(r["routine"], p["block"], p["op"]) for r in final["data"]["accesses"] for h in r["homes"] for p in h["counts"]["read_points"]} & empty_ranges
    outer = {e["attempt"]: e["data"]["stage"] for e in events if e["kind"] == "attempt-start" and e["data"]["parent"] == 0 and e["data"]["purpose"] == "pipeline-attempt"}
    boundaries = {(e["attempt"], e["data"]["stage"]): e for e in events if e["kind"] == "snapshot" and e["attempt"] in outer}
    changes = []
    for home in [h["home"] for h in private if h["counts"]["writes"] and not h["counts"]["reads"]]:
        for attempt, stage in outer.items():
            before, after = (boundaries.get((attempt, s)) for s in ("attempt-input", "attempt-return"))
            if before and after:
                a, b = (access(s, home["routine"], home) for s in (before, after))
                if a and b and a["reads"] and not b["reads"]:
                    changes.append(dict(home=home, stage=stage, before_snapshot=before["id"], after_snapshot=after["id"], before_reads=a["reads"], after_reads=b["reads"]))
    return dict(zero_byte_memory_read_operations=len(empty_reads), access_count_disappearances=changes,
                compat_outcomes=dict(collections.Counter("retained" if e["data"]["retained"] else "removed" for e in events if e["kind"] == "compat-spill-outcome")),
                final_written_homes_without_explicit_reads=sum(h["counts"]["writes"] > 0 and h["counts"]["reads"] == 0 for h in private),
                final_declared_homes_without_explicit_accesses=sum(h["counts"]["writes"] == h["counts"]["reads"] == 0 for h in private),
                selection_outcomes=dict(collections.Counter(e["data"]["outcome"] for e in events if e["kind"] == "selection")),
                cleanup_stops=dict(collections.Counter(e["data"]["reason"] for e in events if e["kind"] == "compat-spill-stop")),
                proof_blockers=dict(collections.Counter(e["data"]["reason"] for e in events if e["kind"] == "rewrite-blocked")),
                zero_byte_operations=sum(e["kind"] == "emitted-range" and e["data"]["offset_start"] == e["data"]["offset_end"] for e in events))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="require frozen artifact equality and complete diagnostic evidence")
    parser.add_argument("--compiler", type=Path, default=ROOT / "build/checked-arena/shared-target/debug/actionc-emit")
    parser.add_argument("--baseline", type=Path, default=BASELINE)
    parser.add_argument("--output", type=Path, default=ROOT / "build/mir6502-phase-trace/qualification")
    parser.add_argument("--api-rlib", type=Path, help="actionc rlib matching --compiler; defaults to its most recently built deps/libactionc-*.rlib")
    parser.add_argument("--mads", type=str, help="roundtrip traced listings through this MADS executable")
    args = parser.parse_args()
    args.compiler = args.compiler.resolve()
    args.output = args.output.resolve()
    if args.api_rlib:
        args.api_rlib = args.api_rlib.resolve()
    baseline = json.loads(args.baseline.read_text())
    args.output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="run-", dir=args.output))
    report = dict(baseline_revision=baseline["revision"], compiler_sha256=sha(args.compiler.read_bytes()), cases=[])
    for case in baseline["cases"]:
        assert sha((ROOT / case["source"]).read_bytes().replace(b"\r\n", b"\n")) == case["source_sha256"], case["id"]
        directory = work / case["id"]
        directory.mkdir()
        command = [args.compiler, "--profile", case["profile"], "--backend", "mir6502", "--runtime", case["runtime"], "--origin", case["origin"]]
        row = dict(id=case["id"], artifacts={}, journals={}, timings_seconds={})
        for suffix, mode in [("xex", "--emit-load"), ("asm", "--emit-listing"), ("map", "--emit-map")]:
            off, off_time = run(command + [mode, case["source"]])
            trace = directory / ("trace" if suffix == "xex" else f"trace-{suffix}")
            flags = ["--trace-mir6502", trace]
            if case["name"] == "tn":
                flags += ["--trace-routine", "FormatDirectoryRow", "--trace-routine", "SetWin"]
            on, on_time = run(command + [mode] + flags + [case["source"]])
            assert off == on and sha(on) == case["artifacts"][suffix], (case["id"], suffix)
            (directory / f"output.{suffix}").write_bytes(on)
            assert json.loads((trace / "manifest.json").read_text())["status"] == "complete"
            row["artifacts"][suffix] = sha(on)
            row["journals"][suffix] = sha((trace / "events.jsonl").read_bytes())
            row["timings_seconds"][suffix] = dict(off=off_time, on=on_time)
        assert len(set(row["journals"].values())) == 1, "emit modes must observe the same compilation"
        manifest, events = read_bundle(directory / "trace")
        row.update(detail_bytes=manifest["detail_bytes"], bundle_bytes=sum(p.stat().st_size for p in (directory / "trace").iterdir()),
                   work=manifest["work"], observations=controls(events))
        if case["name"] == "tn":
            row["diagnosis"] = tn_diagnosis(events, int(case["origin"], 0))
        if args.mads and case["name"] in ("tn", "word", "ranges"):
            # Reuse the actual traced listing and expected XEX; preserve binary bytes.
            for newline in ("lf", "crlf"):
                listing = (directory / "output.asm").read_bytes().replace(b"\r\n", b"\n")
                asm = directory / f"roundtrip-{newline}.asm"
                image = directory / f"roundtrip-{newline}.xex"
                asm.write_bytes(listing if newline == "lf" else listing.replace(b"\n", b"\r\n"))
                run([args.mads, asm, f"-o:{image}", "-s"])
                assert image.read_bytes() == (directory / "output.xex").read_bytes()
            row["mads_lf_crlf"] = "equal"
        report["cases"].append(row)
        print(f"{case['id']}: equal; complete {manifest['detail_bytes']} byte trace", flush=True)
    rlib = args.api_rlib or max((args.compiler.parent / "deps").glob("libactionc-*.rlib"), key=lambda p: p.stat().st_mtime)
    probe = work / ("api-probe.exe" if os.name == "nt" else "api-probe")
    run(["rustc", "--edition=2024", ROOT / "tools/compare6502/phase_trace_api.rs", "--extern", f"actionc={rlib}", "-L", f"dependency={rlib.parent}", "-o", probe])
    run([probe, work / "api"])
    report["api_artifacts"] = {}
    for name, expected in baseline["api_cases"].items():
        actual = sha((work / "api" / name).read_bytes())
        assert actual == expected, name
        report["api_artifacts"][name] = actual
    report["api_controls"] = {}
    for directory in sorted((work / "api").glob("trace-*")):
        manifest, events = read_bundle(directory)
        row = dict(detail_bytes=manifest["detail_bytes"], work=manifest["work"], observations=controls(events))
        if "ranges-optimized" in directory.name:
            attempt = next(e["attempt"] for e in events if e["kind"] == "attempt-start" and e["data"]["stage"] == "pointer_retention/checked_ranges")
            snapshots = [e for e in events if e["attempt"] == attempt and e["kind"] == "snapshot"]
            assert snapshots[0]["data"]["program_identity"] != snapshots[-1]["data"]["program_identity"]
            row["checked_range_changed_final_mir"] = True
        report["api_controls"][directory.name.removeprefix("trace-")] = row
    if args.check:
        outcomes = {outcome for row in report["cases"] for outcome in row["observations"]["selection_outcomes"]}
        assert {"accepted", "rejected", "partial"} <= outcomes, outcomes
        assert sum(len(row.get("diagnosis", [])) for row in report["cases"]) == 12
    report["peak_encoded_collector_bytes"] = max([row["detail_bytes"] for row in report["cases"]] + [row["detail_bytes"] for row in report["api_controls"].values()])
    try:
        import resource
        rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        report["peak_child_rss_bytes"] = rss if sys.platform == "darwin" else rss * 1024
        report["rss_scope"] = "maximum child RSS, including compiler, Rust API probe build/run and MADS; not isolated collector heap"
    except ImportError:
        report["rss_scope"] = "unavailable on this host; encoded collector size is recorded separately"
    (work / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: {len(report['cases']) * 3} frozen artifacts; {work / 'report.json'}")


if __name__ == "__main__":
    main()
