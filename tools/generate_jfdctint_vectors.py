#!/usr/bin/env python3
"""Generate JPEG integer DCT vectors from pinned TACLeBench C, never Action!."""

import argparse
import ctypes
import hashlib
from pathlib import Path
import random
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/jfdctint"
SOURCE_HASH = "916186c1fcac8f8bec8e4d523c84c055f2693eb87d13e4ea789c34e5911c1468"
README_HASH = "fc0a9acfdabcddad7e5a9b156da2df8f0dd0295658332faa047d63b5c771f36e"


def replace(source, old, new, count=1):
    assert source.count(old) == count, old
    return source.replace(old, new)


def reference_source(native=False):
    # Universal-newline reading precedes hashing and multiline instrumentation.
    source = (FIXTURES / "jfdctint.c").read_text()
    assert hashlib.sha256(source.encode()).hexdigest() == SOURCE_HASH
    assert hashlib.sha256((FIXTURES / "README").read_text().encode()).hexdigest() == README_HASH
    source = source[:source.index("int main( void )\n{")]
    source = replace(source, "int main( void );", "")
    # Wrapping arithmetic operates on unsigned bit patterns. Only the descending
    # loop counter must stay signed. Native C is checked on JPEG-range inputs.
    source = re.sub(r"\bint\b", "int32_t" if native else "uint32_t", source)
    if not native:
        source = replace(source, "uint32_t ctr;", "int32_t ctr;")
    source = re.sub(r"#define DESCALE\(x,n\)[^\n]+", "#define DESCALE(x,n) oracle_descale((x), (n))", source)
    if native:
        # Left shift of a negative signed C value is undefined, even in range.
        source = replace(source, "( tmp10 + tmp11 ) << PASS1_BITS",
                         "( tmp10 + tmp11 ) * (1 << PASS1_BITS)")
        source = replace(source, "( tmp10 - tmp11 ) << PASS1_BITS",
                         "( tmp10 - tmp11 ) * (1 << PASS1_BITS)")
    source = replace(source, "return ( ( checksum == jfdctint_CHECKSUM ) ? 0 : -1 );",
                     "oracle_checksum = (uint32_t)checksum;\n"
                     "  return ( ( checksum == jfdctint_CHECKSUM ) ? 0 : -1 );")
    # Capture the input of the column pass without rewriting its calculations.
    anchor = "  dataptr = jfdctint_data;"
    assert source.count(anchor) == 2
    offset = source.index(anchor, source.index(anchor) + len(anchor))
    source = source[:offset] + "  memcpy(oracle_rows, jfdctint_data, sizeof oracle_rows);\n" + source[offset:]
    edits, conditions = [], 0
    for match in re.finditer(r"\bfor\s*\([^;]+;\s*([^;]+);", source):
        edits.extend([(match.start(1), f"oracle_cond({conditions}, !!("), (match.end(1), "))")])
        conditions += 1
    assert conditions == 4
    for offset, value in sorted(edits, reverse=True):
        source = source[:offset] + value + source[offset:]
    prefix = r"""#include <stdint.h>
#include <string.h>
#include <assert.h>
static uint32_t oracle_rows[64], oracle_checksum;
static uint8_t oracle_branches[8], oracle_signs[6];
static uint32_t oracle_iterations[4];
static int oracle_cond(int id, int value) {
  oracle_branches[id*2 + !!value] = 1;
  oracle_iterations[id] += !!value;
  return value;
}
static uint32_t load(const uint8_t *p) {
  uint32_t value = 0;
  for (int i = 0; i < 4; ++i) value |= (uint32_t)p[i] << (8*i);
  return value;
}
static void store(uint8_t *p, uint32_t value) {
  for (int i = 0; i < 4; ++i) p[i] = (uint8_t)(value >> (8*i));
}
static int32_t signed32(uint32_t value) {
  return value <= INT32_MAX ? (int32_t)value : -1 - (int32_t)(UINT32_MAX-value);
}
"""
    if native:
        prefix += r"""
static int32_t oracle_descale(int32_t x, int n) {
  int64_t divisor = INT64_C(1) << n;
  int64_t rounded = (int64_t)x + divisor/2;
  /* Signed division truncates; adjust it to mathematical floor. */
  return (int32_t)(rounded >= 0 ? rounded/divisor : -((-rounded+divisor-1)/divisor));
}
"""
    else:
        prefix += r"""
static uint32_t oracle_descale(uint32_t x, int n) {
  assert(n == 2 || n == 11 || n == 15);
  uint32_t rounded = x + (UINT32_C(1) << (n-1));
  int negative = !!(rounded & UINT32_C(0x80000000));
  oracle_signs[(n == 2 ? 0 : n == 11 ? 2 : 4) + negative] = 1;
  uint32_t shifted = rounded >> n;
  return negative ? shifted | (UINT32_MAX << (32-n)) : shifted;
}
"""
    wrapper = r"""
void oracle_run(int command, int shift, const uint8_t *input,
                uint8_t *output, uint8_t *coverage) {
  memset(oracle_branches, 0, sizeof oracle_branches);
  memset(oracle_signs, 0, sizeof oracle_signs);
  memset(oracle_iterations, 0, sizeof oracle_iterations);
  memset(oracle_rows, 0xCC, sizeof oracle_rows);
  for (int i = 0; i < 64; ++i) jfdctint_data[i] = INPUT_VALUE;
  if (command == 0) jfdctint_init();
  for (int i = 0; i < 64; ++i) store(output + 4*i, (uint32_t)jfdctint_data[i]);
  if (command == 2) {
    for (int i = 0; i < 64; ++i) jfdctint_data[i] = DESCALE(jfdctint_data[i], shift);
  } else {
    jfdctint_main();
  }
  int32_t status = jfdctint_return() == 0 ? 0 : -1;
  for (int i = 0; i < 64; ++i) {
    store(output + 256 + 4*i, oracle_rows[i]);
    store(output + 512 + 4*i, (uint32_t)jfdctint_data[i]);
  }
  store(output + 768, oracle_checksum);
  output[772] = (uint8_t)status;
  output[773] = (uint8_t)((uint32_t)status >> 8);
  memcpy(coverage, oracle_branches, sizeof oracle_branches);
  memcpy(coverage + 8, oracle_signs, sizeof oracle_signs);
  assert(oracle_iterations[0] == (command == 0 ? 64 : 0));
  assert(oracle_iterations[1] == 64);
  assert(oracle_iterations[2] == (command == 2 ? 0 : 8));
  assert(oracle_iterations[3] == (command == 2 ? 0 : 8));
}
"""
    wrapper = wrapper.replace("INPUT_VALUE", "signed32(load(input + 4*i))" if native else "load(input + 4*i)")
    return prefix + source + wrapper


def packed(values):
    return b"".join((value & 0xFFFFFFFF).to_bytes(4, "little") for value in values)


def signed(value):
    value &= 0xFFFFFFFF
    return value - 0x100000000 if value >= 0x80000000 else value


def build_oracle(directory, cc, native=False):
    name = "native" if native else "wrapping"
    cfile, library = directory / f"{name}.c", directory / f"{name}.so"
    cfile.write_text(reference_source(native))
    subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                    str(cfile), "-o", str(library)], check=True)
    oracle = ctypes.CDLL(str(library)).oracle_run
    pointer = ctypes.POINTER(ctypes.c_uint8)
    oracle.argtypes = [ctypes.c_int, ctypes.c_int, pointer, pointer, pointer]
    oracle.restype = None
    return oracle


def generate(cc):
    lines, coverage, native_checks = [], bytearray(14), 0
    rng = random.Random(0x4A464443)
    with tempfile.TemporaryDirectory(prefix="actionc-jfdctint-") as temporary:
        directory = Path(temporary)
        oracle = build_oracle(directory, cc)
        native = build_oracle(directory, cc, native=True)

        def run(label, values, command=1, shift=0):
            nonlocal native_checks
            assert len(values) == 64
            data = packed(values)

            def execute(function):
                output, hits = (ctypes.c_uint8 * 774)(), (ctypes.c_uint8 * 14)()
                function(command, shift, (ctypes.c_uint8 * 256).from_buffer_copy(data), output, hits)
                return bytes(output), hits

            output, hits = execute(oracle)
            if command == 1 and all(-128 <= v <= 127 for v in values):
                assert execute(native)[0] == output, (label, "native C")
                native_checks += 1
            if command == 0:
                seed, initialized = 1, []
                for _ in range(64):
                    seed = (seed*133+81) % 65535
                    initialized.append(seed)
                assert output[:256] == packed(initialized)
                assert output[768:] == packed([1668124]) + b"\0\0"
            elif command == 2:
                # Independent Python floor division checks the helper at ties
                # and at 32-bit wrapping boundaries, including INT32_MIN/MAX.
                expected = [signed(v + (1 << (shift-1))) // (1 << shift) for v in values]
                assert output[512:768] == packed(expected), label
                assert output[256:512] == b"\xCC" * 256
            elif len(set(values)) == 1 and -128 <= values[0] <= 127:
                assert output[512:768] == packed([values[0]*64] + [0]*63), label
            final = [int.from_bytes(output[i:i+4], "little", signed=True) for i in range(512, 768, 4)]
            checksum = signed(sum(final))
            assert output[768:772] == packed([checksum])
            assert output[772:] == (0 if checksum == 1668124 else -1).to_bytes(2, "little", signed=True)
            for i, hit in enumerate(hits):
                coverage[i] |= hit
            fields = [data, output[:256], output[256:512], output[512:768], output[768:772], output[772:]]
            lines.append(f"{label} {command} {shift} " + " ".join(field.hex() for field in fields))

        run("upstream", [0x7FFFFFFF]*64, command=0)
        for value in [0, 1, -1, 127, -128, 32767, -32768, 65535, 0x7FFFFFFF, -0x80000000]:
            run(f"constant-{value}", [value]*64)
        for i in range(64):
            for value in [127, -128]:
                block = [0]*64
                block[i] = value
                run(f"impulse-{i}-{value}", block)
        run("checkerboard", [127 if (i//8+i%8) % 2 else -128 for i in range(64)])
        run("row-stripes", [127 if i//8 % 2 else -128 for i in range(64)])
        run("column-stripes", [127 if i%8 % 2 else -128 for i in range(64)])
        run("row-ramp", [i//8*32-128 for i in range(64)])
        run("column-ramp", [i%8*32-128 for i in range(64)])
        run("asymmetric-ramp", [i*4-128 for i in range(64)])
        edges = [-0x80000000, 0x7FFFFFFF, -65536, 65535, -32768, 32767, -1, 0]
        run("integer-boundaries", edges*8)
        for i in range(16):
            run(f"random-jpeg-{i}", [rng.randint(-128, 127) for _ in range(64)])
            run(f"random-wide-{i}", [rng.randint(-0x80000000, 0x7FFFFFFF) for _ in range(64)])
        for shift in [2, 11, 15]:
            half, step = 1 << (shift-1), 1 << shift
            values = [signed(base + offset) for base in [0, step, -step, -0x80000000, 0x7FFFFFFF]
                      for offset in [-half-1, -half, -half+1, -1, 0, 1, half-1, half, half+1]]
            values += [rng.randint(-0x80000000, 0x7FFFFFFF) for _ in range(64-len(values))]
            run(f"descale-{shift}", values, command=2, shift=shift)
    assert all(coverage), list(coverage)
    summary = f"{len(lines)} cases; 8/8 loop outcomes; 6/6 descale sign outcomes; {native_checks} native C checks"
    print(summary)
    return "\n".join(["# Generated by tools/generate_jfdctint_vectors.py; do not edit.",
                      "# " + summary,
                      "# label command shift input initialized rows output checksum status",
                      *lines, ""])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = generate(args.cc)
    path = FIXTURES / "vectors.txt"
    if args.check:
        if path.read_text() != generated:
            raise SystemExit("jfdctint vectors are stale; regenerate them")
        print("jfdctint vectors match the pinned C reference")
    else:
        path.write_text(generated)


if __name__ == "__main__":
    main()
