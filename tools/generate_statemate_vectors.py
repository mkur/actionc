#!/usr/bin/env python3
"""Regenerate Statemate VM vectors from the pinned C source, never from Action!.

Requires a GCC-compatible C compiler only when regenerating. VM tests consume
the committed text vectors without a C toolchain or network access.
"""

import argparse
import ctypes
import hashlib
from pathlib import Path
import random
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle"
SOURCE_SHA256 = "e740e35e70bd4c91e5f6f970ae2cb874b78200dba2bfd19a3daddd9f47d50dca"


def layout():
    rows = []
    for line in (FIXTURES / "state.tsv").read_text().splitlines():
        if line.startswith("#"):
            continue
        original, action, kind, offset, size = line.split("\t")
        rows.append((original, action, kind, int(offset), int(size)))
    return rows


def reference_source(rows):
    source = (FIXTURES / "statemate.c").read_text()
    if hashlib.sha256(source.encode()).hexdigest() != SOURCE_SHA256:
        raise ValueError("Pinned Statemate source changed; review provenance and adaptations")
    # The upstream checksum shifts an unsigned long by indices up to 63. Its
    # behavior is not portable to 32-bit long. Check complete state instead.
    source = source[:source.index("int statemate_return()")]
    source = re.sub(r"\bunsigned long\b", "uint32_t", source)
    source = re.sub(r"\bint\b", "int16_t", source)
    source = re.sub(r"\bchar\b", "uint8_t", source)
    # Instrument switch arms and IF outcomes in the C reference to select
    # useful test inputs. This does not modify the controller's state.
    source = re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.S)
    arms = []

    def arm(match):
        index = len(arms)
        arms.append(match.group())
        return match.group() + f" oracle_arms[{index}] = 1; "

    source = re.sub(r"\b(?:case\s+\d+|default)\s*:\s*\{", arm, source)
    conditions = 0
    edits = []
    for match in re.finditer(r"\bif\s*\(", source):
        start = match.end()
        end, depth = start, 1
        while depth:
            depth += (source[end] == "(") - (source[end] == ")")
            end += 1
        edits.extend([(start, f"oracle_cond({conditions}, ("), (end - 1, "))")])
        conditions += 1
    for position, text in sorted(edits, reverse=True):
        source = source[:position] + text + source[position:]
    prefix = f"""#include <stdint.h>
#include <string.h>
static uint8_t oracle_arms[{len(arms)}];
static uint8_t oracle_branches[{conditions * 2}];
static int oracle_cond(int id, int value) {{
  oracle_branches[id * 2 + !!value] = 1;
  return value;
}}
"""
    load, save = [], []
    for original, _, kind, offset, size in rows:
        if size == 64:
            load.append(f"memcpy({original}, input + {offset}, 64);")
            save.append(f"memcpy(output + {offset}, {original}, 64);")
        else:
            ctype = {"BYTE": "uint8_t", "INT": "int16_t", "LONGCARD": "uint32_t"}[kind]
            value = " | ".join(f"((uint32_t)input[{offset + i}] << {8 * i})" for i in range(size))
            load.append(f"{original} = ({ctype})({value});")
            for i in range(size):
                save.append(f"output[{offset + i}] = (uint8_t)((uint32_t){original} >> {8 * i});")
    commands = [
        "statemate_init(); statemate_FH_DU();", "statemate_interface();",
        "statemate_generic_KINDERSICHERUNG_CTRL();", "statemate_generic_FH_TUERMODUL_CTRL();",
        "statemate_generic_EINKLEMMSCHUTZ_CTRL();", "statemate_generic_BLOCK_ERKENNUNG_CTRL();",
        "statemate_FH_DU();",
    ]
    wrapper = "\nvoid oracle_run(int command, const uint8_t *input, uint8_t *output, uint8_t *coverage) {\n"
    wrapper += "memset(oracle_arms, 0, sizeof oracle_arms);\nmemset(oracle_branches, 0, sizeof oracle_branches);\n"
    wrapper += "\n".join(load) + "\nswitch(command) {\n"
    wrapper += "\n".join(f"case {i}: {command} break;" for i, command in enumerate(commands))
    wrapper += "\n}\n" + "\n".join(save)
    wrapper += "\nmemcpy(coverage, oracle_arms, sizeof oracle_arms);"
    wrapper += "\nmemcpy(coverage + sizeof oracle_arms, oracle_branches, sizeof oracle_branches);\n}\n"
    return prefix + source + wrapper, len(arms), conditions * 2


def generate(cc):
    rows = layout()
    size = sum(row[4] for row in rows)
    fields = {action: (offset, length) for _, action, _, offset, length in rows}
    source, arms, branches = reference_source(rows)
    with tempfile.TemporaryDirectory(prefix="actionc-statemate-") as temporary:
        directory = Path(temporary)
        cfile, library = directory / "oracle.c", directory / "oracle.so"
        cfile.write_text(source)
        subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                        str(cfile), "-o", str(library)], check=True)
        oracle = ctypes.CDLL(str(library)).oracle_run
        pointer = ctypes.POINTER(ctypes.c_uint8)
        oracle.argtypes = [ctypes.c_int, pointer, pointer, pointer]
        oracle.restype = None
        buffer = ctypes.c_uint8 * size
        coverage_buffer = ctypes.c_uint8 * (arms + branches)
        seen = set()
        vectors = []

        def put(state, name, value):
            offset, length = fields[name]
            state[offset:offset + length] = (value % (1 << (8 * length))).to_bytes(length, "little")

        def run(label, command, state, keep=True):
            output, coverage = buffer(), coverage_buffer()
            oracle(command, buffer.from_buffer_copy(state), output, coverage)
            hit = {i for i, value in enumerate(coverage) if value}
            if keep or hit - seen:
                vectors.append((label, command, bytes(state).hex(), bytes(output).hex()))
                seen.update(hit)
            return bytearray(output)

        initial = run("startup", 0, bytearray(size))
        assert initial[5] == 1 and sum(initial[:64]) == 1
        assert initial[fields["microstep"][0]] == 100

        # Real controller calls retain the upstream fixed clock and 100 rounds.
        # Feed button/sensor edges between calls and carry all state forward.
        state = initial.copy()
        for label, changes in [
            ("open-press", {"Unit_S_FH_AUFDISC": 1}),
            ("open-release", {"Unit_S_FH_AUFDISC": 0}),
            ("close-press", {"Unit_S_FH_ZUDISC": 1}),
            ("pinch", {"Door_EKS_LEISTE_AKTIV": 1}),
            ("pinch-release", {"Door_EKS_LEISTE_AKTIV": 0}),
            ("close-release", {"Unit_S_FH_ZUDISC": 0}),
            ("ignition-on", {"Door_KL_50": 1, "Unit_S_FH_AUFDISC": 1}),
            ("ignition-off", {"Door_KL_50": 0}),
        ]:
            for field, value in changes.items():
                put(state, field, value)
            state = run("sequence-" + label, 6, state)

        # Timer expiry compares 32-bit unsigned differences, including wrap.
        for now in [1, 0x10001, 0x80000001, 0xFFFFFFFF]:
            for elapsed in [0, 1, 2, 3, 499, 500, 501]:
                state = initial.copy()
                put(state, "time", now)
                for field in fields:
                    if field.startswith("sc_"):
                        put(state, field, now - elapsed)
                for field in ["Door_MFHA_copy", "Door_MFHZ_copy"]:
                    put(state, field, 1)
                run(f"timer-{now:08x}-{elapsed}", 1, state)

        # Seed internal substates deliberately: the fixed-clock WCET driver
        # cannot naturally reach every timer-dependent nested state.
        rng = random.Random(0x53544154)
        for number in range(12000):
            command = [2, 3, 3, 3, 4, 5, 1, 6][number % 8]
            state = bytearray(size)
            for _, action, kind, offset, length in rows:
                if length == 64:
                    state[offset:offset + length] = bytes(rng.randrange(2) for _ in range(length))
                elif kind == "BYTE":
                    if action.endswith("State"):
                        choices = [0, 1, 2, 3, 255]
                    elif action in ("microstep", "stable"):
                        choices = [0, 1]
                    else:
                        choices = [0, 1, 0x80, 0xFF]
                    put(state, action, rng.choice(choices))
                elif kind == "INT":
                    put(state, action, rng.choice([-100, -1, 0, 1, 2, 10, 11, 12, 58, 59, 60, 61, 404, 405, 406]))
                else:
                    put(state, action, rng.choice([0, 1, 2, 3, 499, 500, 501, 0xFFFF, 0x10000, 0xFFFFFFFE]))
            if number % 3:
                state[10] = state[13] = state[16] = state[19] = 1
            run(f"dispatch-{number:05d}", command, state, keep=number < 64)

        missing = set(range(arms)) - seen
        if missing:
            raise RuntimeError(f"Uncovered C switch arms: {sorted(missing)}")
        header = [
            "# Generated by tools/generate_statemate_vectors.py; do not edit.",
            f"# state_bytes={size} switch_arms={arms}/{arms} if_outcomes={len(seen) - arms}/{branches}",
            "# label command input_hex expected_hex",
        ]
        text = "\n".join(header + [" ".join(map(str, row)) for row in vectors]) + "\n"
        return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="cc", help="GCC-compatible C compiler")
    parser.add_argument("--check", action="store_true", help="verify committed vectors")
    args = parser.parse_args()
    text = generate(args.cc)
    destination = FIXTURES / "statemate-vectors.txt"
    if args.check:
        if destination.read_text() != text:
            raise SystemExit("Statemate vectors are stale; regenerate them")
    else:
        destination.write_text(text)
    print(text.splitlines()[1])
    print(f"{len(text.splitlines()) - 3} vectors; SHA-256 {hashlib.sha256(text.encode()).hexdigest()}")


if __name__ == "__main__":
    main()
