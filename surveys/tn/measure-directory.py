#!/usr/bin/env python3
"""Build the four cartridge programs and measure emitted and reserved memory.

Uses emitted listing bytes for symbols (splitlines accepts LF and CRLF), and
loads the XEX segments to read the actual final SET BUFFER=* pointer. Output
stays under target; no generated executables are added to the source tree.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ORIGIN = 0x2C00


def measure(source, mode, compiler, out, reuse=False):
    stem = f"{source.stem}-{mode}"
    xex, asm = out / f"{stem}.xex", out / f"{stem}.asm"
    if not reuse:
        subprocess.run([str(compiler), "--mode", mode, "--runtime", "cart",
                        "-o", str(xex), "--listing", str(asm), str(source)], check=True)
    blob = xex.read_bytes()
    memory = bytearray(65536)
    loaded = set()
    offset = 0
    while offset < len(blob):
        start = int.from_bytes(blob[offset:offset+2], "little")
        offset += 2
        if start == 0xFFFF:
            continue
        end = int.from_bytes(blob[offset:offset+2], "little") + 1
        offset += 2
        assert end > start and offset + end - start <= len(blob)
        memory[start:end] = blob[offset:offset+end-start]
        loaded.update(range(start, end))
        offset += end - start
    listing = asm.read_text().splitlines()
    index = listing.index("global_buffer:")
    address = next(int(m[1], 16) for line in listing[index+1:]
                   if (m := re.search(r"; \$([0-9A-F]+):", line)))
    buffer = int.from_bytes(memory[address:address+2], "little")
    main = {a for a in loaded if a >= ORIGIN}
    assert max(main) < buffer < 0xA000

    # All emitted homes and spills belonging to the new shared routines, plus
    # their shared I/O buffers. Count addresses, not alias labels. Include
    # source-local constants conservatively in this scratch estimate.
    routines = []
    for name in ["DIR.ACT", "MYDOS.ACT", "LOCATION.ACT", "PANELDIR.ACT"]:
        file = source.parent / name
        if file.exists():
            routines += re.findall(r"^(?:\w+\s+)?(?:PROC|FUNC)\s+(\w+)", file.read_text(), re.M)
    routines += ["Draw", "GoTo", "GoLast", "TaggedFiles", "IsTagged", "IsProtected",
                 "IsDirectory", "Tag", "Untag", "TagAll", "FindNext", "ActivateLocation",
                 "SetWin", "EnterDirectory", "ParentDirectory", "PrintCachedEntry"]
    prefixes = tuple(f"{kind}_{routine.lower()}_" for routine in routines
                     for kind in ["local", "param", "spill", "static_string"])
    scratch_globals = {"global_directoryinput", "global_rowdata", "global_fname",
                       "global_fnamelen", "global_currentbatch", "global_currenttags",
                       "global_currentlocation", "global_directoryerror"}
    scratch = set()
    owner = False
    for line in listing:
        if line.startswith("; ====="):
            owner = False
        if match := re.fullmatch(r"(\w+):", line):
            owner = match[1] in scratch_globals or match[1].startswith(prefixes)
        if owner and (match := re.search(r"; \$([0-9A-F]+):((?: [0-9A-F]{2})+)", line)):
            base = int(match[1], 16)
            scratch.update(range(base, base + len(match[2].split())))
    reserved = buffer - ORIGIN
    return dict(program=source.name, mode=mode, load_file=len(blob),
                emitted=len(main), deferred=reserved-len(main), reserved=reserved,
                buffer=f"${buffer:04X}", copy_to_A000=0xA000-buffer,
                shared_homes_and_buffers=len(scratch))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, default=ROOT / "target/debug/actionc")
    parser.add_argument("--source-dir", type=Path, default=ROOT / "samples/tn/modern")
    parser.add_argument("--out-dir", type=Path, default=ROOT / "target/tn-directory/final")
    parser.add_argument("--reuse", action="store_true", help="measure existing output files without compiling")
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    results = [measure(args.source_dir / name, mode, args.compiler, args.out_dir, args.reuse)
               for name in ["TN.ACT", "TNDBG.ACT"]
               for mode in ["optimized", "mir6502"]]
    report = json.dumps(results, indent=2)
    (args.out_dir / "memory.json").write_text(report + "\n")
    print(report)


if __name__ == "__main__":
    main()
