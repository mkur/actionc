#!/usr/bin/env python3
"""Account for frozen Strcat instructions and verify them against measured XEXs."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "docs/benchmarks/6502-argument-staging/strcat-breakdown.json"
BUCKETS = ["entry_homing", "pointer_setup", "computation", "outgoing_arguments", "call_return"]
# Half-open offsets in the frozen Action body. These describe observed costs,
# not rewrite patterns or claims that any instruction can be deleted.
REGIONS = [
    (0, 14, "entry_homing", "entry_homing"),
    (14, 21, "pointer_setup", "initial_s_pointer"),
    (21, 47, "computation", "destination_arithmetic"),
    (47, 55, "outgoing_arguments", "destination_capture"),
    (55, 61, "computation", "source_low_arithmetic"),
    (61, 63, "outgoing_arguments", "source_low_capture"),
    (63, 68, "computation", "source_high_arithmetic"),
    (68, 70, "outgoing_arguments", "source_high_abi"),
    (70, 80, "pointer_setup", "length_t_pointer"),
    (80, 84, "computation", "length_read"),
    (84, 86, "outgoing_arguments", "length_abi"),
    (86, 92, "outgoing_arguments", "argument_register_reloads"),
    (92, 95, "call_return", "call"),
    (95, 115, "pointer_setup", "post_call_pointers"),
    (115, 124, "computation", "length_update"),
    (124, 125, "call_return", "return"),
]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def xex_memory(data):
    assert data[:2] == b"\xff\xff"
    at, memory = 2, {}
    while at < len(data):
        start = int.from_bytes(data[at:at + 2], "little")
        if start == 0xFFFF:
            at += 2
            continue
        end = int.from_bytes(data[at + 2:at + 4], "little")
        size = end - start + 1
        at += 4
        assert size > 0 and at + size <= len(data)
        memory.update(enumerate(data[at:at + size], start))
        at += size
    assert at == len(data)
    return memory


def parse_action(text):
    text = text.replace("\r\n", "\n")
    header = re.search(r"; ===== PROC Strcat \$([0-9A-F]+)\.\.\$([0-9A-F]+) =====", text)
    assert header
    start, end = (int(value, 16) for value in header.groups())
    body = text[header.end():].split("; ===== END PROC Strcat =====", 1)[0]
    rows = []
    for line in body.splitlines():
        match = re.fullmatch(r"\s*(.*?)\s+; \$([0-9A-F]+): (.*)", line)
        if not match:
            continue
        raw = []
        for token in match[3].split():
            if not re.fullmatch(r"[0-9A-F]{2}", token):
                break
            raw.append(int(token, 16))
        rows.append(dict(offset=int(match[2], 16) - start, bytes=raw, assembly=match[1]))
    return start, end, rows


def parse_oscar(text):
    text = text.replace("\r\n", "\n")
    body = text.split("\nStrcat: ;", 1)[1].split("\n----", 1)[0]
    rows = []
    for line in body.splitlines():
        match = re.fullmatch(r"([0-9a-f]+) : ([0-9a-f_]{2}) ([0-9a-f_]{2}) ([0-9a-f_]{2}) (.*)", line)
        if match:
            rows.append(dict(address=int(match[1], 16),
                             bytes=[int(b, 16) for b in match.groups()[1:4] if b != "__"],
                             assembly=match[5]))
    start = rows[0]["address"]
    for row in rows:
        row["offset"] = row.pop("address") - start
    return start, start + 51, rows


def classify(rows, action):
    totals, details = Counter(), Counter()
    cursor = 0
    for row in rows:
        assert row["offset"] == cursor, row
        cursor += len(row["bytes"])
        if action:
            region = next(r for r in REGIONS if r[0] <= row["offset"] < r[1])
            assert cursor <= region[1]
            bucket, detail = region[2:]
        else:
            bucket = ("outgoing_arguments" if row["offset"] in [13, 15, 23, 30, 36]
                      else "call_return" if row["offset"] in [38, 50] else "computation")
            detail = bucket
        row.update(bucket=bucket, detail=detail)
        totals[bucket] += len(row["bytes"])
        details[detail] += len(row["bytes"])
    assert cursor == (125 if action else 51)
    return dict(totals), dict(details)


def verify_rows(memory, start, rows):
    for row in rows:
        assert [memory[start + row["offset"] + i] for i in range(len(row["bytes"]))] == row["bytes"]


def measure():
    baseline = json.loads((OUT.parent / "baseline.json").read_text())
    kernels = json.loads((ROOT / "surveys/tn/c-oscar64/kernels.json").read_text())
    action, controls = None, []
    for name in ["TN", "TNDBG"]:
        for runtime, label in [("cart", "ActionCart"), ("standalone", "Standalone")]:
            path = ROOT / f"build/argument-staging/before/{name}-{runtime}-2C00.asm"
            text = path.read_text()
            parsed = parse_action(text)
            assert parsed == parse_action(text.replace("\r\n", "\n").replace("\n", "\r\n"))
            start, end, rows = parsed
            totals, details = classify(rows, True)
            assert end - start == 125
            frozen = next(a for a in baseline["artifacts"] if a["runtime"] == runtime
                          and a["source"] == f"samples/tn/modern/{name}.ACT")
            for enabled in [False, True]:
                image_path = ROOT / f"build/argument-staging/measured/{name}-{label}-{str(enabled).lower()}.xex"
                image = image_path.read_bytes()
                assert digest(image) == frozen["sha256"]
                verify_rows(xex_memory(image), start, rows)
                controls.append(dict(image=str(image_path.relative_to(ROOT)), sha256=digest(image),
                                     start=start, end=end, instruction_count=len(rows), bytes=125))
            if action is None:
                action = dict(listing=str(path.relative_to(ROOT)), listing_sha256=digest(path.read_bytes()),
                              start=start, end=end, totals=totals, details=details, instructions=rows)
            assert totals == action["totals"]
    oscar = []
    for profile in ["Os", "O3"]:
        path = ROOT / f"build/tn-c-oscar64/kernels-{profile}.asm"
        text = path.read_text()
        parsed = parse_oscar(text)
        assert parsed == parse_oscar(text.replace("\r\n", "\n").replace("\n", "\r\n"))
        start, end, rows = parsed
        totals, _ = classify(rows, False)
        image_path = path.with_suffix(".xex")
        image = image_path.read_bytes()
        assert digest(image) == kernels["artifacts"][image_path.name]
        verify_rows(xex_memory(image), start, rows)
        oscar.append(dict(profile=profile, image_sha256=digest(image), start=start, end=end,
                          totals=totals, instructions=rows))
    assert oscar[0]["totals"] == oscar[1]["totals"]
    gap = {key: action["totals"].get(key, 0) - oscar[0]["totals"].get(key, 0) for key in BUCKETS}
    assert sum(gap.values()) == 74
    return dict(date="2026-11-11", compiler_code_revision="6311c110", baseline_code_revision="476cfecf",
                scope="Static body bytes only; excludes callers, MovePage body and storage. Categories describe emitted costs, not proven removable bytes.",
                action=action, oscar=oscar, gap_bytes=gap, verified_action_images=controls,
                action_outgoing_argument_split=dict(
                    temporary_staging=sum(action["details"][k] for k in [
                        "destination_capture", "source_low_capture", "argument_register_reloads"]),
                    required_memory_abi=action["details"]["source_high_abi"] + action["details"]["length_abi"]),
                action_destination_scratch_traffic_bytes=sum(
                    len(row["bytes"]) for row in action["instructions"]
                    if row["offset"] in [23, 28, 30, 34, 36, 41, 45]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare with committed byte accounting")
    parser.add_argument("--roles-output", type=Path, help="write instruction roles for the opt-in VM profile")
    args = parser.parse_args()
    report = measure()
    if args.check:
        assert json.loads(OUT.read_text()) == report, "Strcat accounting differs"
    else:
        OUT.write_text(json.dumps(report, indent=2) + "\n")
    if args.roles_output:
        args.roles_output.parent.mkdir(parents=True, exist_ok=True)
        args.roles_output.write_text("".join(f"{row['offset']:04X}\t{row['bucket']}\n"
                                            for row in report["action"]["instructions"]))
    for bucket in BUCKETS:
        print(bucket, report["action"]["totals"].get(bucket, 0),
              report["oscar"][0]["totals"].get(bucket, 0), report["gap_bytes"][bucket])
    print("Verified 8 Action images, both C controls and LF/CRLF listing parsing")


if __name__ == "__main__":
    main()
