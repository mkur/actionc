#!/usr/bin/env python3
"""Generate typed binary-search vectors from pinned TACLeBench C, never Action!."""

import argparse
import ctypes
import hashlib
from pathlib import Path
import random
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/binarysearch"
SOURCE_HASH = "6be708e5870717a7a62b7b26cb9589838eb9ddcb40c64f15e3b883fab90c9bce"
# Element C type, byte width, signedness, result C type, result width.
KINDS = {
    "LONGINT": ("int32_t", 4, True, "int32_t", 4),
    "BYTE": ("uint8_t", 1, False, "int16_t", 2),
    "INT": ("int16_t", 2, True, "int16_t", 2),
    "CARD": ("uint16_t", 2, False, "int32_t", 4),
}


def replace_once(source, old, new):
    assert source.count(old) == 1, old
    return source.replace(old, new)


def reference_source(kind):
    element, width, signed, result, result_width = KINDS[kind]
    # Normalize checked-out CRLF before hash verification or instrumentation.
    source = (FIXTURES / "binarysearch.c").read_text()
    if hashlib.sha256(source.encode()).hexdigest() != SOURCE_HASH:
        raise ValueError("Pinned binarysearch.c changed; review provenance/adaptations")
    source = source[:source.index("int main( void )\n{")]
    source = replace_once(source, "int main( void );", "")
    source = re.sub(r"\b(?:int|long)\b", "int32_t", source)
    source = replace_once(source, "int32_t key;\n  int32_t value;",
                          f"{element} key;\n  {element} value;")
    source = replace_once(source, "int32_t binarysearch_result;", f"{result} binarysearch_result;")
    source = replace_once(source, "int32_t binarysearch_binary_search( int32_t );",
                          f"{result} binarysearch_binary_search( {element} );")
    source = replace_once(source, "int32_t binarysearch_binary_search( int32_t x )",
                          f"{result} binarysearch_binary_search( {element} x )")
    source = replace_once(source, "int32_t fvalue, mid, up, low;",
                          f"{result} fvalue;\n  int32_t mid, up, low;")
    conditions, edits = 0, []
    for match in re.finditer(r"\b(?:if|while)\s*\(", source):
        start, depth = match.end(), 1
        end = start
        while depth:
            depth += (source[end] == "(") - (source[end] == ")")
            end += 1
        edits.extend([(start, f"oracle_cond({conditions}, !!("), (end - 1, "))")])
        conditions += 1
    assert conditions == 3
    for offset, value in sorted(edits, reverse=True):
        source = source[:offset] + value + source[offset:]
    prefix = f"""#include <stdint.h>
#include <string.h>
#include <assert.h>
#define ELEMENT {element}
#define WIDTH {width}
#define IS_SIGNED {int(signed)}
#define RESULT_WIDTH {result_width}
static uint8_t oracle_branches[6];
static int oracle_cond(int id, int value) {{
  oracle_branches[id * 2 + !!value] = 1;
  return value;
}}
"""
    wrapper = r"""
static int32_t load(const uint8_t *p, int width, int is_signed) {
  uint32_t value = 0;
  for (int i = 0; i < width; ++i) value |= (uint32_t)p[i] << (8*i);
  uint32_t mask = UINT32_MAX >> (32 - 8*width);
  return is_signed && value > (mask >> 1)
       ? -1 - (int32_t)(mask - value) : (int32_t)value;
}
static void store(uint8_t *p, uint32_t value, int width) {
  for (int i = 0; i < width; ++i) p[i] = (uint8_t)(value >> (8*i));
}
void oracle_run(int command, const uint8_t *query, const uint8_t *seed,
                const uint8_t *input, uint8_t *output, uint8_t *coverage) {
  binarysearch_seed = load(seed, 4, 1);
  for (int i = 0; i < 15; ++i) {
    binarysearch_data[i].key = (ELEMENT)load(input + 2*i*WIDTH, WIDTH, IS_SIGNED);
    binarysearch_data[i].value = (ELEMENT)load(input + (2*i+1)*WIDTH, WIDTH, IS_SIGNED);
  }
  memset(oracle_branches, 0, sizeof oracle_branches);
  if (command == 0) {
    binarysearch_init();
    binarysearch_main();
  } else {
    for (int i = 1; i < 15; ++i)
      assert(binarysearch_data[i-1].key <= binarysearch_data[i].key);
    binarysearch_result = binarysearch_binary_search((ELEMENT)load(query, WIDTH, IS_SIGNED));
  }
  store(output, (uint32_t)binarysearch_seed, 4);
  for (int i = 0; i < 15; ++i) {
    store(output + 4 + 2*i*WIDTH, (uint32_t)binarysearch_data[i].key, WIDTH);
    store(output + 4 + (2*i+1)*WIDTH, (uint32_t)binarysearch_data[i].value, WIDTH);
  }
  store(output + 4 + 30*WIDTH, (uint32_t)binarysearch_return(), RESULT_WIDTH);
  memcpy(coverage, oracle_branches, sizeof oracle_branches);
}
"""
    return prefix + source + wrapper


def packed(values, width):
    mask = (1 << (8*width)) - 1
    return b"".join((value & mask).to_bytes(width, "little") for value in values)


def generate(cc):
    rows, summaries = [], []
    for kind, (_, width, signed, _, result_width) in KINDS.items():
        bits = 8*width
        lower = -(1 << (bits-1)) if signed else 0
        upper = (1 << (bits-1)) - 1 if signed else (1 << bits) - 1
        rng = random.Random(0x42494E53 + width*2 + signed)
        with tempfile.TemporaryDirectory(prefix="actionc-binarysearch-") as temporary:
            directory = Path(temporary)
            cfile, library = directory / "oracle.c", directory / "oracle.so"
            cfile.write_text(reference_source(kind))
            subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                            str(cfile), "-o", str(library)], check=True)
            oracle = ctypes.CDLL(str(library)).oracle_run
            pointer = ctypes.POINTER(ctypes.c_uint8)
            oracle.argtypes = [ctypes.c_int, pointer, pointer, pointer, pointer, pointer]
            oracle.restype = None
            coverage, count = bytearray(6), 0

            def run(label, keys, values, query, command=1):
                nonlocal count
                assert len(keys) == len(values) == 15
                assert all(lower <= value <= upper for value in [*keys, *values, query])
                assert command == 0 or keys == sorted(keys)
                query_bytes = packed([query], width)
                seed_bytes = packed([rng.randrange(-0x80000000, 0x80000000)], 4)
                data_bytes = packed([v for pair in zip(keys, values) for v in pair], width)
                output = (ctypes.c_uint8 * (4 + 30*width + result_width))()
                hits = (ctypes.c_uint8 * 6)()
                def argument(data):
                    return (ctypes.c_uint8 * len(data)).from_buffer_copy(data)
                oracle(command, argument(query_bytes), argument(seed_bytes), argument(data_bytes),
                       output, hits)
                output = bytes(output)
                result_bytes = output[4+30*width:]
                result = int.from_bytes(result_bytes, "little", signed=True)
                if command:
                    # Linear lookup independently validates unique-key results;
                    # C determines which matching record wins for duplicates.
                    matches = [value for key, value in zip(keys, values) if key == query]
                    assert result in (matches or [-1]), (kind, label, result)
                    assert output[:4] == seed_bytes
                    assert output[4:4+30*width] == data_bytes
                else:
                    assert result == -1, (kind, result)
                for i, hit in enumerate(hits):
                    coverage[i] |= hit
                rows.append(f"{kind} {label} {command} {query_bytes.hex()} {seed_bytes.hex()} "
                            f"{data_bytes.hex()} {output[:4].hex()} "
                            f"{output[4:4+30*width].hex()} {result_bytes.hex()}")
                count += 1

            # Poison input proves that Init resets both the table and seed.
            run("upstream", [upper]*15, [lower]*15, lower, command=0)
            boundaries = {
                "BYTE": [0, 1, 2, 63, 64, 126, 127, 128, 129, 191, 192, 252, 253, 254, 255],
                "INT": [-32768, -32767, -256, -255, -129, -128, -1, 0, 1, 127, 128, 255, 256, 32766, 32767],
                "CARD": [0, 1, 127, 128, 255, 256, 32766, 32767, 32768, 32769, 65279, 65280, 65533, 65534, 65535],
                "LONGINT": [lower, lower+1, -65536, -32769, -32768, -1, 0, 1, 32767, 32768,
                            65535, 65536, 16777216, upper-1, upper],
            }[kind]
            queries = range(256) if kind == "BYTE" else sorted({
                key + delta for key in boundaries for delta in [-1, 0, 1]
                if lower <= key + delta <= upper
            })
            for query in queries:
                run(f"boundary-{query}", boundaries, boundaries[::-1], query)
            keys = [20 + 3*i for i in range(15)]
            for query in sorted({*keys, *(key+1 for key in keys), 19, 63}):
                run(f"position-{query}", keys, boundaries[7:] + boundaries[:7], query)
            for query in [19, 20, 21, 29, 30, 31, 39, 40, 41]:
                run(f"duplicates-{query}", [20]*5 + [30]*5 + [40]*5, list(range(10, 25)), query)
            for query in [29, 30, 31]:
                run(f"all-equal-{query}", [30]*15, list(range(10, 25)), query)
            for index in range(8):
                keys = sorted(rng.sample(range(lower, upper+1), 15))
                values = [rng.randint(lower, upper) for _ in range(15)]
                missing = set()
                while len(missing) < 5:
                    query = rng.randint(lower, upper)
                    if query not in keys:
                        missing.add(query)
                for query in sorted([*keys, *missing]):
                    run(f"random-{index:02d}-{query}", keys, values, query)
            assert all(coverage), (kind, coverage)
            summaries.append(f"# {kind}: cases={count} if_while_outcomes={sum(coverage)}/6")
    header = ["# Generated by tools/generate_binarysearch_vectors.py; do not edit.",
              *summaries, f"# cases={len(rows)}",
              "# type label command query seed data expected_seed expected_data result"]
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
            raise SystemExit("Binary-search vectors are stale; regenerate them")
        print("Binary-search vectors match the pinned C reference")
    else:
        destination.write_text(text, newline="\n")
        print(f"Wrote {destination.relative_to(ROOT)}")
    print("\n".join(text.splitlines()[1:6]))


if __name__ == "__main__":
    main()
