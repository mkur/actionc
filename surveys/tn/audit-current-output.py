#!/usr/bin/env python3
"""Census source-annotated MADS listings, checked against their XEX bytes.

This is diagnostic analysis, not an optimizer. Counts describe instruction and
directive forms; authored machine code and embedded runtimes need separate
interpretation. --check exercises the real parser with LF and CRLF input.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
PROC = re.compile(r"; ===== PROC (.+?) \$([0-9A-F]+)\.\.\$([0-9A-F]+)(?: entry \$([0-9A-F]+))? =====")
ROW = re.compile(r"^(.*?)\s*; \$([0-9A-F]+):((?: [0-9A-F]{2})+)(?:\s|$)")
LABEL = re.compile(r"(\w+):$")
FIXED = re.compile(r"(\w+) = \$([0-9A-F]+)$")
HOME_KINDS = ("spill", "param", "local", "static_string")
WRITES = {0x81, 0x85, 0x8D, 0x91, 0x95, 0x99, 0x9D, 0x84, 0x86, 0x8C, 0x8E, 0x94, 0x96}
RMW = {0x06, 0x26, 0x46, 0x66, 0xC6, 0xE6, 0x16, 0x36, 0x56, 0x76, 0xD6, 0xF6,
       0x0E, 0x2E, 0x4E, 0x6E, 0xCE, 0xEE, 0x1E, 0x3E, 0x5E, 0x7E, 0xDE, 0xFE}
EXTRA_ZP = {0xA4, 0xA6, 0x84, 0x86, 0xC4, 0xE4, 0x24, 0xB4, 0x94, 0xB6, 0x96}
EXTRA_ABS = {0xAC, 0xAE, 0x8C, 0x8E, 0xCC, 0xEC, 0x2C, 0xBC, 0xBE}


def xex_memory(blob):
    memory, offset = {}, 0
    while offset < len(blob):
        assert offset + 2 <= len(blob)
        start = int.from_bytes(blob[offset:offset+2], "little")
        offset += 2
        if start == 0xFFFF:
            continue
        assert offset + 2 <= len(blob)
        end = int.from_bytes(blob[offset:offset+2], "little") + 1
        offset += 2
        assert end > start and offset + end - start <= len(blob)
        for address, byte in enumerate(blob[offset:offset+end-start], start):
            assert address not in memory, f"overlapping XEX address ${address:04X}"
            memory[address] = byte
        offset += end - start
    return memory


def parse_listing(text):
    rows, routines, aliases, active_labels = [], {}, {}, []
    current = None
    for line in text.splitlines():
        if match := PROC.fullmatch(line):
            name, start, end, entry = match.groups()
            assert name not in routines
            routines[name] = dict(start=int(start, 16), end=int(end, 16),
                                  entry=int(entry or start, 16))
            current, active_labels = name, []
        elif line.startswith("; ===== END PROC "):
            current, active_labels = None, []
        elif line.startswith("; ====="):
            active_labels = []
        if match := FIXED.fullmatch(line):
            aliases[match[1]] = {int(match[2], 16)}
        if match := LABEL.fullmatch(line):
            active_labels.append(match[1])
        if match := ROW.match(line):
            form, address, byte_text = match.groups()
            address = int(address, 16)
            data = bytes.fromhex(byte_text)
            form = form.strip()
            directive = form.startswith((".BYTE", ".WORD", "DTA "))
            rows.append(dict(address=address, data=data, form=form, routine=current,
                             directive=directive))
            if directive:
                for label in active_labels:
                    aliases.setdefault(label, set()).update(range(address, address+len(data)))
            else:
                active_labels = []
    return rows, routines, aliases


def direct_operand(data):
    """Static direct operand base; indirect and immediate operands excluded."""
    op = data[0]
    if op & 3 == 1:
        mode = op & 0x1C
        if mode in {0x04, 0x14}:
            return data[1]
        if mode in {0x0C, 0x18, 0x1C}:
            return int.from_bytes(data[1:3], "little")
    if op in EXTRA_ZP or op in RMW and op & 0x0C == 0x04:
        return data[1]
    if op in EXTRA_ABS or op in RMW and op & 0x0C == 0x0C:
        return int.from_bytes(data[1:3], "little")
    return None


def census(listing, blob):
    rows, ranges, aliases = parse_listing(listing)
    loaded = xex_memory(blob)
    loaded_addresses = set(loaded)
    listed = {}
    for row in rows:
        for address, byte in enumerate(row["data"], row["address"]):
            assert address not in listed, f"duplicate listing address ${address:04X}"
            listed[address] = byte
    # MADS output writes RUNAD symbolically, without address/byte comments.
    # Reconcile every annotated byte and separately resolve that symbolic word.
    unannotated = set(loaded) - set(listed)
    if unannotated:
        assert unannotated == {0x02E2, 0x02E3}, unannotated
        target = re.search(r"ORG \$02E2\s+DTA A\((\w+)\)", listing)[1]
        lines = listing.splitlines()
        index = lines.index(target + ":")
        row = next(ROW.match(line) for line in lines[index+1:] if ROW.match(line))
        assert int(row[2], 16) == loaded[0x02E2] + 256*loaded[0x02E3]
    assert listed == {a:b for a,b in loaded.items() if a not in unannotated}, \
        "listing bytes do not exactly reconcile with XEX"
    home_sets = {kind: set() for kind in HOME_KINDS}
    for label, addresses in aliases.items():
        for kind in HOME_KINDS:
            if label.startswith(kind + "_"):
                home_sets[kind].update(addresses)
    metrics = {}
    for name, extent in ranges.items():
        owned = {kind: set() for kind in HOME_KINDS}
        for label, addresses in aliases.items():
            for kind in HOME_KINDS:
                if label.lower().startswith(f"{kind}_{name.lower()}_"):
                    owned[kind].update(addresses)
        metrics[name] = dict(**extent, instruction_form_bytes=0, directive_form_bytes=0,
                             instructions=0, direct_calls=0, calls={}, pointer_scratch_writes=0,
                             indirect_y_operations=0, abi_cell_accesses=0,
                             home_bytes={k: len(v & loaded_addresses) for k, v in owned.items()},
                             fixed_home_bytes={k: len(v - loaded_addresses) for k, v in owned.items()},
                             static_home_operand_accesses={k: 0 for k in HOME_KINDS},
                             mnemonics={})
    starts = {extent["entry"]: name for name, extent in ranges.items()}
    for row in rows:
        if row["routine"] is None:
            continue
        metric = metrics[row["routine"]]
        data = row["data"]
        metric["directive_form_bytes" if row["directive"] else "instruction_form_bytes"] += len(data)
        if row["directive"]:
            continue
        metric["instructions"] += 1
        mnemonic = row["form"].split()[0].split(".")[0]
        metric["mnemonics"][mnemonic] = metric["mnemonics"].get(mnemonic, 0) + 1
        op = data[0]
        if op == 0x20:
            metric["direct_calls"] += 1
            target = int.from_bytes(data[1:3], "little")
            target = starts.get(target, f"${target:04X}")
            metric["calls"][target] = metric["calls"].get(target, 0) + 1
        if op & 0x1F == 0x11:
            metric["indirect_y_operations"] += 1
        if (operand := direct_operand(data)) is not None:
            accesses = 2 if op in RMW else 1
            if op in WRITES and 0xAC <= operand <= 0xAF:
                metric["pointer_scratch_writes"] += 1
            if 0xA0 <= operand <= 0xA5:
                metric["abi_cell_accesses"] += accesses
            for kind, addresses in home_sets.items():
                if operand in addresses:
                    metric["static_home_operand_accesses"][kind] += accesses
    return dict(load_file_bytes=len(blob), loaded_bytes=len(loaded),
                unannotated_run_vector_bytes=len(unannotated),
                instruction_form_bytes=sum(len(r["data"]) for r in rows if not r["directive"]),
                directive_form_bytes=sum(len(r["data"]) for r in rows if r["directive"]),
                routine_count=len(metrics), routines=metrics,
                image_sha256=hashlib.sha256(blob).hexdigest())


def check():
    fixture = """; ===== DATA spill0 $3000 =====
spill_Demo_0:
        .BYTE $00,$01,$02,$03,$04,$05,$06,$07 ; $3000: 00 01 02 03 04 05 06 07
; ===== PROC Demo $3008..$3016 entry $3008 =====
proc_Demo:
        LDA.A $3007 ; $3008: AD 07 30
        STA.Z $AC ; $300B: 85 AC
        INC.A $3000 ; $300D: EE 00 30
        JSR.A proc_Demo ; $3010: 20 08 30
        .BYTE $40,$41 ; $3013: 40 41
        RTS ; $3015: 60
; ===== END PROC Demo =====
"""
    payload = bytes(range(8)) + bytes.fromhex("AD 07 30 85 AC EE 00 30 20 08 30 40 41 60")
    blob = bytes.fromhex("FF FF 00 30 15 30") + payload
    result = census(fixture, blob)
    assert result == census(fixture.replace("\n", "\r\n"), blob)
    demo = result["routines"]["Demo"]
    assert demo["static_home_operand_accesses"]["spill"] == 3
    assert demo["pointer_scratch_writes"] == 1 and demo["calls"] == {"Demo": 1}
    assert demo["instruction_form_bytes"] == 12 and demo["directive_form_bytes"] == 2
    assert demo["home_bytes"]["spill"] == 8
    try:
        census(fixture, blob[:-1] + b"\xEA")
    except AssertionError:
        pass
    else:
        raise AssertionError("mismatched image was accepted")
    classic = """; ===== PROC Dummy $3000..$3004 entry $3003 =====
; ===== DATA Dummy storage $3000 =====
param_Dummy_x:
        .BYTE $00,$00,$00 ; $3000: 00 00 00
proc_Dummy:
        RTS ; $3003: 60
; ===== END PROC Dummy =====
; ===== PROC Caller $3004..$3008 =====
proc_Caller:
        JSR.A proc_Dummy ; $3004: 20 03 30
        RTS ; $3007: 60
; ===== END PROC Caller =====
        ORG $02E2
        DTA A(proc_Caller)
"""
    image = bytes.fromhex("FF FF 00 30 07 30 00 00 00 60 20 03 30 60 E2 02 E3 02 04 30")
    result = census(classic, image)
    assert result == census(classic.replace("\n", "\r\n"), image)
    assert result["routines"]["Caller"]["calls"] == {"Dummy": 1}
    assert result["routines"]["Dummy"]["home_bytes"]["param"] == 3
    assert result["unannotated_run_vector_bytes"] == 2
    print("LF/CRLF, long data, routine data, physical spill lanes and byte reconciliation passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, default=ROOT / "target/debug/actionc")
    parser.add_argument("--out-dir", type=Path, default=ROOT / "target/tn-output-audit")
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--experiment", choices=["masks", "predicate", "combined"],
                        help="build a source counterfactual under out-dir; production is unchanged")
    args = parser.parse_args()
    if args.check:
        check()
        return
    args.out_dir.mkdir(parents=True, exist_ok=True)
    source_dir = ROOT / "samples/tn/modern"
    if args.experiment:
        source_dir = args.out_dir / "source"
        if not args.reuse:
            source_dir.mkdir(exist_ok=True)
            for path in (ROOT / "samples/tn/modern").glob("*.ACT"):
                shutil.copy2(path, source_dir / path.name)
            path = source_dir / "DIR.ACT"
            source = path.read_text()
            if args.experiment in {"masks", "combined"}:
                assert source.count("mask=1 LSH BYTE(ordinal&7)") == 2
                assert source.count("mask=(1 LSH mask)-1") == 1
                source = "BYTE ARRAY auditPowers(8)=[1,2,4,8,16,32,64,128]\n" + source
                source = source.replace("mask=1 LSH BYTE(ordinal&7)", "mask=auditPowers(BYTE(ordinal&7))")
                source = source.replace("mask=(1 LSH mask)-1", "mask=auditPowers(mask)-1")
            if args.experiment in {"predicate", "combined"}:
                original = "RETURN(ChangeTagged(tags,ordinal,value#0))"
                assert source.count(original) == 1
                source = source.replace(original, "  BYTE normalized\n  normalized=value#0\nRETURN(ChangeTagged(tags,ordinal,normalized))")
            path.write_text(source)
    report = {}
    for program in ["TN", "TNDBG"]:
        for mode in ["optimized", "mir6502"]:
            stem = f"{program}-{mode}"
            listing, image = args.out_dir / f"{stem}.asm", args.out_dir / f"{stem}.xex"
            if not args.reuse:
                subprocess.run([str(args.compiler), "--mode", mode, "--runtime", "cart",
                                "--listing", str(listing), "-o", str(image),
                                str(source_dir / f"{program}.ACT")], check=True)
            report[stem] = census(listing.read_text(), image.read_bytes())
    (args.out_dir / "census.json").write_text(json.dumps(report, indent=2) + "\n")
    for stem, data in report.items():
        ranked = sorted(data["routines"].items(), key=lambda r:-r[1]["instruction_form_bytes"])
        print(stem, data["load_file_bytes"], "load bytes", data["routine_count"], "routines")
        for name, routine in ranked[:15]:
            print(f"  {name:24} {routine['instruction_form_bytes']:5} bytes; "
                  f"{routine['static_home_operand_accesses']['spill']:3} spill operands; "
                  f"{routine['pointer_scratch_writes']:3} pointer-scratch writes; "
                  f"{routine['direct_calls']:2} calls")


if __name__ == "__main__":
    main()
