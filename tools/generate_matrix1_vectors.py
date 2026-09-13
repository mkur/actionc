#!/usr/bin/env python3
"""Generate typed matrix1 vectors from pinned TACLeBench C, never Action!."""

import argparse
import ctypes
import hashlib
from pathlib import Path
import random
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/matrix1"
SOURCE_HASH = "de07299ca342c626454bc655a4ba4425935571f36643383e51e5bfc27d46654d"
KINDS = {"LONGINT": ("int32_t", 4, True), "BYTE": ("uint8_t", 1, False),
         "INT": ("int16_t", 2, True), "CARD": ("uint16_t", 2, False)}
SHAPES = [(10, 10, 10), (3, 7, 5), (2, 129, 1)]


def replace(source, old, new, count=1):
    assert source.count(old) == count, old
    return source.replace(old, new)


def reference_source(kind, shape, native=False):
    element, width, signed = KINDS[kind]
    assert not native or kind == "LONGINT"
    # Universal-newline reading precedes hashing and source instrumentation.
    source = (FIXTURES / "matrix1.c").read_text()
    if hashlib.sha256(source.encode()).hexdigest() != SOURCE_HASH:
        raise ValueError("Pinned matrix1.c changed; review provenance/adaptations")
    source = source[:source.index("int main( void )\n{")]
    source = replace(source, "int main( void );", "")
    for name, size in zip("XYZ", shape):
        source = replace(source, f"#define {name} 10", f"#define {name} {size}")
    source = re.sub(r"\bint\b", "int32_t", source)
    source = replace(source, "matrix1_pin_down( int32_t A[  ], int32_t B[  ], int32_t C[  ] )",
                     "matrix1_pin_down( element_t A[  ], element_t B[  ], element_t C[  ] )", 2)
    for name in "ABC":
        source = replace(source, f"int32_t matrix1_{name}[", f"element_t matrix1_{name}[")
    source = replace(source, "register int32_t *p_", "register element_t *p_", 3)
    source = replace(source, "volatile int32_t x = 1;", "volatile element_t x = 1;")
    if not native:
        # Define Action!'s wrapping operations, including signed and narrow
        # extensions, without depending on C signed overflow or narrowing.
        source = replace(source, "*p_c += *p_a++ * *p_b++;",
                         "*p_c = oracle_mac(*p_c, *p_a++, *p_b++);")
        source = replace(source, "checksum += matrix1_C[ i ];",
                         "checksum = oracle_sum(checksum, matrix1_C[ i ]);")
    source = replace(source, "return ( checksum ==  1000 ? 0 : -1 );",
                     "oracle_checksum = checksum;\n  return ( checksum == 1000 ? 0 : -1 );")
    edits, conditions = [], 0
    for match in re.finditer(r"\bfor\s*\([^;]+;\s*([^;]+);", source):
        edits.extend([(match.start(1), f"oracle_cond({conditions}, !!("), (match.end(1), "))")])
        conditions += 1
    assert conditions == 7
    for offset, value in sorted(edits, reverse=True):
        source = source[:offset] + value + source[offset:]
    prefix = f"""#include <stdint.h>
#include <string.h>
#include <assert.h>
typedef {element} element_t;
#define WIDTH {width}
#define IS_SIGNED {int(signed)}
#define ELEMENT_MASK UINT64_C({(1 << (width*8)) - 1})
static int32_t oracle_checksum;
static uint8_t oracle_branches[14];
static uint32_t oracle_iterations[7];
static int oracle_cond(int id, int value) {{
  oracle_branches[id*2 + !!value] = 1;
  oracle_iterations[id] += !!value;
  return value;
}}
static int32_t signed32(uint32_t value) {{
  return value <= INT32_MAX ? (int32_t)value : -1 - (int32_t)(UINT32_MAX-value);
}}
static element_t element(uint64_t value) {{
  uint64_t raw = value & ELEMENT_MASK;
  int64_t decoded = IS_SIGNED && raw > (ELEMENT_MASK >> 1)
                  ? -1 - (int64_t)(ELEMENT_MASK - raw) : (int64_t)raw;
  return (element_t)decoded;
}}
static element_t oracle_mac(element_t accumulator, element_t a, element_t b) {{
  /* Even the largest signed 32-bit product plus accumulator fits int64_t. */
  return element((uint64_t)((int64_t)accumulator + (int64_t)a * (int64_t)b));
}}
static int32_t oracle_sum(int32_t accumulator, element_t value) {{
  return signed32((uint32_t)accumulator + (uint32_t)value);
}}
"""
    wrapper = r"""
static uint32_t load(const uint8_t *p, int width) {
  uint32_t value = 0;
  for (int i = 0; i < width; ++i) value |= (uint32_t)p[i] << (8*i);
  return value;
}
static void store(uint8_t *p, uint32_t value, int width) {
  for (int i = 0; i < width; ++i) p[i] = (uint8_t)(value >> (8*i));
}
void oracle_run(int command, const uint8_t *a, const uint8_t *b, const uint8_t *c,
                uint8_t *output, uint8_t *coverage) {
  for (int i = 0; i < X*Y; ++i) matrix1_A[i] = element(load(a + i*WIDTH, WIDTH));
  for (int i = 0; i < Y*Z; ++i) matrix1_B[i] = element(load(b + i*WIDTH, WIDTH));
  for (int i = 0; i < X*Z; ++i) matrix1_C[i] = element(load(c + i*WIDTH, WIDTH));
  memset(oracle_branches, 0, sizeof oracle_branches);
  memset(oracle_iterations, 0, sizeof oracle_iterations);
  if (command == 0) matrix1_init();
  matrix1_main();
  int32_t result = matrix1_return();
  store(output, (uint32_t)oracle_checksum, 4);
  store(output + 4, (uint32_t)result, 2);
  output += 6;
  for (int i = 0; i < X*Y; ++i) store(output + i*WIDTH, (uint32_t)matrix1_A[i], WIDTH);
  output += X*Y*WIDTH;
  for (int i = 0; i < Y*Z; ++i) store(output + i*WIDTH, (uint32_t)matrix1_B[i], WIDTH);
  output += Y*Z*WIDTH;
  for (int i = 0; i < X*Z; ++i) store(output + i*WIDTH, (uint32_t)matrix1_C[i], WIDTH);
  memcpy(coverage, oracle_branches, sizeof oracle_branches);
  assert(oracle_iterations[0] == (command ? 0 : X*Y));
  assert(oracle_iterations[1] == (command ? 0 : Y*Z));
  assert(oracle_iterations[2] == (command ? 0 : X*Z));
  assert(oracle_iterations[3] == X*Z);
  assert(oracle_iterations[4] == Z);
  assert(oracle_iterations[5] == X*Z);
  assert(oracle_iterations[6] == X*Y*Z);
}
"""
    return prefix + source + wrapper


def packed(values, width):
    mask = (1 << (width*8)) - 1
    return b"".join((value & mask).to_bytes(width, "little") for value in values)


def wrap(value, width, signed):
    bits = width*8
    value &= (1 << bits) - 1
    return value - (1 << bits) if signed and value >= (1 << (bits-1)) else value


def mathematical_product(a, b, shape, width, signed):
    rows, inner, columns = shape
    values, native_safe = [], True
    # Indexed Python dot products are independent of C's advancing pointers.
    for column in range(columns):
        for row in range(rows):
            partial = 0
            for k in range(inner):
                product = a[row*inner+k] * b[column*inner+k]
                native_safe &= -0x80000000 <= product <= 0x7FFFFFFF
                partial += product
                native_safe &= -0x80000000 <= partial <= 0x7FFFFFFF
            values.append(wrap(partial, width, signed))
    checksum = 0
    for value in values:
        checksum += value
        native_safe &= -0x80000000 <= checksum <= 0x7FFFFFFF
    return values, wrap(checksum, 4, True), native_safe


def build_oracle(directory, cc, kind, shape, native=False):
    name = "native" if native else "wrapping"
    cfile, library = directory / f"{name}.c", directory / f"{name}.so"
    cfile.write_text(reference_source(kind, shape, native))
    subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                    str(cfile), "-o", str(library)], check=True)
    oracle = ctypes.CDLL(str(library)).oracle_run
    pointer = ctypes.POINTER(ctypes.c_uint8)
    oracle.argtypes = [ctypes.c_int, pointer, pointer, pointer, pointer, pointer]
    oracle.restype = None
    return oracle


def generate(cc):
    lines, summaries = [], []
    for kind, (_, width, signed) in KINDS.items():
        bits = width*8
        lower = -(1 << (bits-1)) if signed else 0
        upper = (1 << (bits-1))-1 if signed else (1 << bits)-1
        for shape in SHAPES:
            rows, inner, columns = shape
            shape_name = "x".join(map(str, shape))
            na, nb, nc = rows*inner, inner*columns, rows*columns
            rng = random.Random(0x4D415452 + width*2 + signed + rows*10000 + inner*100 + columns)
            with tempfile.TemporaryDirectory(prefix="actionc-matrix1-") as temporary:
                directory = Path(temporary)
                oracle = build_oracle(directory, cc, kind, shape)
                native = build_oracle(directory, cc, kind, shape, native=True) if signed and width == 4 else None
                coverage, count, native_checks = bytearray(14), 0, 0

                def run(label, a, b, command=1):
                    nonlocal count, native_checks
                    assert len(a) == na and len(b) == nb
                    assert all(lower <= value <= upper for value in [*a, *b])
                    c = [rng.randint(lower, upper) for _ in range(nc)]
                    inputs = [packed(a, width), packed(b, width), packed(c, width)]
                    ma, mb = ([1]*na, [1]*nb) if command == 0 else (a, b)
                    expected, checksum, native_safe = mathematical_product(ma, mb, shape, width, signed)
                    size = 6 + (na+nb+nc)*width

                    def execute(function):
                        arguments = [(ctypes.c_uint8 * len(data)).from_buffer_copy(data) for data in inputs]
                        output, hits = (ctypes.c_uint8 * size)(), (ctypes.c_uint8 * 14)()
                        function(command, *arguments, output, hits)
                        return bytes(output), hits

                    output, hits = execute(oracle)
                    expected_bytes = packed([checksum], 4) + packed([0 if checksum == 1000 else -1], 2)
                    expected_bytes += packed(ma, width) + packed(mb, width) + packed(expected, width)
                    assert output == expected_bytes, (kind, shape, label)
                    if native is not None and native_safe:
                        assert execute(native)[0] == output, (kind, shape, label, "native C")
                        native_checks += 1
                    for i, hit in enumerate(hits):
                        coverage[i] |= hit
                    start_b, start_c = 6 + na*width, 6 + (na+nb)*width
                    lines.append(f"{kind} {shape_name} {label} {command} "
                                 + " ".join(data.hex() for data in inputs)
                                 + f" {output[6:start_b].hex()} {output[start_b:start_c].hex()} "
                                 + f"{output[start_c:].hex()} {output[:4].hex()} {output[4:6].hex()}")
                    count += 1

                a = [((i*3+k*5) % 7)+1 for i in range(rows) for k in range(inner)]
                b = [((k*7+j*11) % 13)+1 for j in range(columns) for k in range(inner)]
                identity_a = [int(i == k) for i in range(rows) for k in range(inner)]
                identity_b = [int(k == j) for j in range(columns) for k in range(inner)]
                edges = [lower, upper, 0, 1, -1 if signed else upper-1,
                         min(127, upper), min(128, upper), min(255, upper), min(256, upper)]
                run("upstream", [upper]*na, [lower]*nb, command=0)
                run("zero", [0]*na, [0]*nb)
                run("zero-left", [0]*na, b)
                run("zero-right", a, [0]*nb)
                run("asymmetric", a, b)
                run("left-identity", identity_a, b)
                run("right-identity", a, identity_b)
                run("left-boundaries", [edges[i % len(edges)] for i in range(na)], identity_b)
                run("right-boundaries", identity_a, [edges[i % len(edges)] for i in range(nb)])
                last_a, last_b = [0]*na, [0]*nb
                last_a[-1], last_b[-1] = upper, 1
                run("last-element", last_a, last_b)
                run("accumulate-wrap", [upper]*na, [1]*nb)
                run("multiply-wrap", [upper]*na, [upper]*nb)
                if signed:
                    run("signed", [value if i % 2 else -value for i, value in enumerate(a)], b)
                    run("minimum-negated", [lower]*na, [-1]*nb)
                for index in range(4):
                    low, high = (-16, 16) if signed else (0, 3)
                    run(f"random-small-{index}", [rng.randint(low, high) for _ in range(na)],
                        [rng.randint(low, high) for _ in range(nb)])
                    run(f"random-wide-{index}", [rng.randint(lower, upper) for _ in range(na)],
                        [rng.randint(lower, upper) for _ in range(nb)])
                assert all(coverage), (kind, shape, coverage)
                assert native is None or native_checks > 0
                summaries.append(f"# {kind}/{shape_name}: cases={count} for_outcomes={sum(coverage)}/14 "
                                 f"native_c_crosschecks={native_checks}")
    header = ["# Generated by tools/generate_matrix1_vectors.py; do not edit.", *summaries,
              f"# cases={len(lines)}",
              "# type shape label command a b c expected_a expected_b expected_c checksum result"]
    return "\n".join(header + lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="cc", help="GCC-compatible C compiler")
    parser.add_argument("--check", action="store_true", help="verify committed vectors")
    args = parser.parse_args()
    text = generate(args.cc)
    destination = FIXTURES / "vectors.txt"
    if args.check:
        if destination.read_text() != text:
            raise SystemExit("matrix1 vectors are stale; regenerate them")
        print("matrix1 vectors match the pinned C reference")
    else:
        destination.write_text(text, newline="\n")
        print(f"Wrote {destination.relative_to(ROOT)}")
    print("\n".join(text.splitlines()[1:14]))


if __name__ == "__main__":
    main()
