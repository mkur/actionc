#!/usr/bin/env python3
"""Generate insertion-sort vectors from pinned TACLeBench C, never Action!."""

import argparse
import ctypes
import hashlib
import itertools
from pathlib import Path
import random
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/insertsort"
SOURCE_HASH = "2c35c25aff3fcfa23caa5fbe3c1d94195e1a336cb3043faf1db2419c10d3368f"
INITIAL_STATS = [0, 100000, 0, 0, 100000, 0]
ORIGINAL = [0, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2]


def reference_source():
    # read_text normalizes checked-out CRLF before hashing or instrumentation.
    source = (FIXTURES / "insertsort.c").read_text()
    if hashlib.sha256(source.encode()).hexdigest() != SOURCE_HASH:
        raise ValueError("Pinned insertsort.c changed; review provenance/adaptations")
    source = source[:source.index("int main( void )\n{")]
    source = source.replace("int main( void );", "")
    source = re.sub(r"\bunsigned int\b", "uint32_t", source)
    source = re.sub(r"\bint\b", "int32_t", source)
    # The C temporary only copies unsigned elements. Give it the same type to
    # avoid implementation-defined conversions for extended high-bit inputs.
    source = source.replace("int32_t  i, j, temp;", "int32_t i, j;\n  uint32_t temp;")
    # Define the extended-input checksum modulo 2^32, without signed overflow.
    source = source.replace("int32_t i, returnValue = 0;", "int32_t i;\n  uint32_t returnValue = 0;")
    assert "uint32_t temp;" in source and "uint32_t returnValue" in source
    conditions, edits = 0, []
    for match in re.finditer(r"\b(?:if|while)\s*\(", source):
        start, depth = match.end(), 1
        end = start
        while depth:
            depth += (source[end] == "(") - (source[end] == ")")
            end += 1
        edits.extend([(start, f"oracle_cond({conditions}, !!("), (end - 1, "))")])
        conditions += 1
    for offset, value in sorted(edits, reverse=True):
        source = source[:offset] + value + source[offset:]
    prefix = f"""#include <stdint.h>
#include <string.h>
#include <assert.h>
static uint8_t oracle_branches[{conditions * 2}];
static int oracle_cond(int id, int value) {{
  oracle_branches[id * 2 + !!value] = 1;
  return value;
}}
"""
    wrapper = r"""
static uint32_t load32(const uint8_t *p) {
  return (uint32_t)p[0] | ((uint32_t)p[1] << 8)
       | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static int32_t signed32(uint32_t value) {
  return value <= INT32_MAX ? (int32_t)value : -1 - (int32_t)(UINT32_MAX - value);
}
static void store32(uint8_t *p, uint32_t value) {
  for (int i = 0; i < 4; ++i) p[i] = (uint8_t)(value >> (8*i));
}
void oracle_run(int command, const uint8_t *stats, const uint8_t *input,
                uint8_t *output, uint8_t *coverage) {
  uint32_t values[11];
  for (int i = 0; i < 11; ++i) values[i] = load32(input + 4*i);
  if (command) {
    for (int i = 1; i < 11; ++i) assert(values[0] <= values[i]);
  }
  insertsort_iters_i = signed32(load32(stats));
  insertsort_min_i = signed32(load32(stats + 4));
  insertsort_max_i = signed32(load32(stats + 8));
  insertsort_iters_a = signed32(load32(stats + 12));
  insertsort_min_a = signed32(load32(stats + 16));
  insertsort_max_a = signed32(load32(stats + 20));
  memset(insertsort_a, 0xA5, sizeof insertsort_a);
  memset(oracle_branches, 0, sizeof oracle_branches);
  if (command == 0) insertsort_init();
  else insertsort_initialize(values);
  insertsort_main();
  store32(output, insertsort_iters_i);
  store32(output + 4, insertsort_min_i);
  store32(output + 8, insertsort_max_i);
  store32(output + 12, insertsort_iters_a);
  store32(output + 16, insertsort_min_a);
  store32(output + 20, insertsort_max_a);
  for (int i = 0; i < 11; ++i) store32(output + 24 + 4*i, insertsort_a[i]);
  output[68] = (uint8_t)insertsort_return();
  memcpy(coverage, oracle_branches, sizeof oracle_branches);
}
"""
    return prefix + source + wrapper, conditions * 2


def words(values):
    return b"".join((value & 0xFFFFFFFF).to_bytes(4, "little") for value in values)


def generate(cc):
    source, branches = reference_source()
    with tempfile.TemporaryDirectory(prefix="actionc-insertsort-") as temporary:
        directory = Path(temporary)
        cfile, library = directory / "oracle.c", directory / "oracle.so"
        cfile.write_text(source)
        subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                        str(cfile), "-o", str(library)], check=True)
        oracle = ctypes.CDLL(str(library)).oracle_run
        pointer = ctypes.POINTER(ctypes.c_uint8)
        oracle.argtypes = [ctypes.c_int, pointer, pointer, pointer, pointer]
        oracle.restype = None
        rows, coverage = [], bytearray(branches)
        rng = random.Random(0x494E5345)

        def run(label, values, stats=INITIAL_STATS, command=1):
            assert len(values) == 11 and all(0 <= value <= 0xFFFFFFFF for value in values)
            assert command == 0 or all(value >= values[0] for value in values[1:])
            stats_bytes, input_bytes = words(stats), words(values)
            output, hits = (ctypes.c_uint8 * 69)(), (ctypes.c_uint8 * branches)()
            oracle(command, (ctypes.c_uint8 * 24).from_buffer_copy(stats_bytes),
                   (ctypes.c_uint8 * 44).from_buffer_copy(input_bytes), output, hits)
            output = bytes(output)
            original = ORIGINAL if command == 0 else values
            expected = words([original[0], *sorted(original[1:])])
            assert output[24:68] == expected, label
            assert output[68] == int(sum(original) & 0xFFFFFFFF != 65), label
            for i, hit in enumerate(hits):
                coverage[i] |= hit
            rows.append(f"{label} {command} {stats_bytes.hex()} {input_bytes.hex()} "
                        f"{output[:24].hex()} {output[24:68].hex()} {output[68]}")
            return output

        run("upstream", [0xDEADBEEF] * 11, [-1] * 6, command=0)
        patterns = {
            "ascending": list(range(2, 12)),
            "descending": list(range(11, 1, -1)),
            "all-zero": [0] * 10,
            "all-max": [0xFFFFFFFF] * 10,
            "alternating": [0xFFFFFFFF, 0] * 5,
            "duplicates": [9, 3, 9, 3, 0, 9, 0, 3, 9, 0],
            "word-boundaries": [0x10000, 0xFFFF, 0x100, 0xFF, 0x8000, 0x7FFF, 0, 1, 0x80, 0x7F],
            "signed-boundaries": [0xFFFFFFFF, 0x7FFFFFFF, 0x80000000, 0, 0x80000001,
                                  1, 0xFFFFFFFE, 0x7FFFFFFE, 0x1000000, 0xFFFFFF],
            "last-minimum": [*range(1, 10), 0],
            "first-maximum": [0xFFFFFFFF, *range(9)],
            "checksum-wrap-65": [0xFFFFFFFF, 66, *([0] * 8)],
        }
        for label, values in patterns.items():
            run(label, [0, *values])
        for sentinel in [1, 0x12345678, 0x80000000, 0xFFFFFFF0]:
            run(f"sentinel-{sentinel:08x}", [sentinel, *(sentinel + i for i in range(9, -1, -1))])
        for index, values in enumerate(itertools.permutations(range(5))):
            run(f"permutation-{index:03d}", [0, *values, *range(5, 10)])
        for index in range(32):
            values = [0, *(rng.getrandbits(32) for _ in range(10))]
            output = run(f"random-{index:02d}", values)
            previous_stats = [int.from_bytes(output[i:i+4], "little", signed=True)
                              for i in range(0, 24, 4)]
            # A second invocation retains observed minima/maxima and sorts an
            # already sorted input, checking persistent statistics and idempotence.
            run(f"repeat-{index:02d}", [0, *sorted(values[1:])], previous_stats)
        for seed in [-0x80000000, -1, 0, 1, 8, 9, 10, 100000, 0x7FFFFFFF]:
            run(f"stats-{seed}", [0, *patterns["duplicates"]], [seed] * 6)

        assert all(coverage), f"Uncovered C outcomes: {[i for i, hit in enumerate(coverage) if not hit]}"
        header = [
            "# Generated by tools/generate_insertsort_vectors.py; do not edit.",
            f"# cases={len(rows)} if_while_outcomes={sum(coverage)}/{branches}",
            "# label command input_stats24 input_values44 expected_stats24 expected_values44 result",
        ]
        return "\n".join(header + rows) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="cc", help="GCC-compatible C compiler")
    parser.add_argument("--check", action="store_true", help="verify committed vectors")
    args = parser.parse_args()
    text = generate(args.cc)
    destination = FIXTURES / "vectors.txt"
    if args.check:
        if destination.read_text() != text:
            raise SystemExit("Insertion-sort vectors are stale; regenerate them")
        print("Insertion-sort vectors match the pinned C reference")
    else:
        destination.write_text(text, newline="\n")
        print(f"Wrote {destination.relative_to(ROOT)}")
    print(text.splitlines()[1])


if __name__ == "__main__":
    main()
