#!/usr/bin/env python3
"""Cross-check a leaf-facts dump against its disassembly.

This is a static audit aid, not a semantic proof or rewrite permission. Indexed
and indirect accesses, external transfers, and register restore sequences need
manual review. Implicit CPU stack/return accesses are outside the memory summary
contract. All instructions are scanned, conservatively including unreachable ones.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re

INSTRUCTION = re.compile(r"^([0-9A-F]{4})\s+((?:[0-9A-F]{2}\s+)+)([A-Z]{3}|\.BYTE)(?:\s+(.*))?$")
READS = set("ADC AND ASL BIT CMP CPX CPY DEC EOR INC LDA LDX LDY LSR ORA ROL ROR SBC".split())
WRITES = set("STA STX STY INC DEC ASL LSR ROL ROR".split())
X_WRITES = set("LDX TAX TSX INX DEX".split())
Y_WRITES = set("LDY TAY INY DEY".split())


def audit(routine):
    direct_reads, direct_writes = set(), set()
    dynamic_reads, dynamic_writes, transfers, unparsed = [], [], [], []
    register_writes = {"x": [], "y": []}
    cursor = routine["start"]
    for line in routine["assembly"].splitlines():
        m = INSTRUCTION.fullmatch(line)
        if not m:
            unparsed.append(line)
            continue
        address, encoded, op, arg = m.groups()
        address = int(address, 16)
        assert address == cursor, (routine["name"], line, cursor)
        cursor += len(encoded.split())
        arg = arg or ""
        if op == ".BYTE":
            unparsed.append(line)
            continue
        direct = re.fullmatch(r"\$([0-9A-F]{2,4})", arg)
        if op in READS and direct:
            direct_reads.add(int(direct[1], 16))
        if op in WRITES and direct:
            direct_writes.add(int(direct[1], 16))
        dynamic = "," in arg or "(" in arg
        if op in READS and dynamic:
            dynamic_reads.append(line)
        if op in WRITES and dynamic:
            dynamic_writes.append(line)
        if op == "JSR" or (op == "JMP" and (
            direct is None or not routine["start"] <= int(direct[1], 16) < routine["end"]
        )):
            transfers.append(line)
        for register, operations in [("x", X_WRITES), ("y", Y_WRITES)]:
            if op in operations:
                register_writes[register].append(line)
    if not unparsed:
        assert cursor == routine["end"], routine["name"]

    summary = routine["rewrite_summary"]
    checks = {}
    for kind, actual in [("reads", direct_reads), ("writes", direct_writes)]:
        fact = summary[kind]
        if fact["kind"] == "exact":
            resolved = {b["address"] for b in fact["bytes"] if b["address"] is not None}
            checks[kind] = {
                "direct_accesses_not_covered": sorted(actual - resolved),
                "unresolved_identities": [b["identity"] for b in fact["bytes"] if b["address"] is None],
            }
        else:
            checks[kind] = None
    return {
        "name": routine["name"], "class": routine["class"],
        "direct_reads": sorted(direct_reads), "direct_writes": sorted(direct_writes),
        "dynamic_reads": dynamic_reads, "dynamic_writes": dynamic_writes,
        "external_transfers": transfers, "unparsed_lines": unparsed,
        "exact_summary_checks": checks,
        "claimed_preserved_register_writes": {
            reg: sites for reg, sites in register_writes.items() if summary["preserves"][reg] and sites
        },
        "unclaimed_unmodified_index_registers": [
            reg for reg, sites in register_writes.items()
            if not summary["preserves"][reg] and not sites and not transfers and not unparsed
        ],
        "unknown_write_operations": sorted({o["kind"] for o in routine["ops"] if o["makes_writes_unknown"]}),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("facts", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compact-facts", type=Path,
                        help="Save the same facts with repeated MIR context stored once")
    args = parser.parse_args()
    facts = json.loads(args.facts.read_text())
    rows = [audit(r) for r in facts["leaf_candidates"]]
    result = {"schema": 1, "scope": "static instruction scan; manual review required",
              "raw_facts_sha256": hashlib.sha256(args.facts.read_bytes()).hexdigest(), "routines": rows}
    if args.compact_facts:
        contexts = set()
        for r in facts["leaf_candidates"]:
            context, body = r["mir"].split("\nroutine ", 1)
            contexts.add(context)
            r["mir"] = "routine " + body
        assert len(contexts) == 1, "The printer's shared context changed between routines"
        facts["mir_context"] = contexts.pop()
        args.compact_facts.write_text(json.dumps(facts, indent=2) + "\n")
        result["compact_facts_sha256"] = hashlib.sha256(args.compact_facts.read_bytes()).hexdigest()
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
