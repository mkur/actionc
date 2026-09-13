#!/usr/bin/env python3
"""Generate stateful ADPCM decoder vectors from pinned TACLeBench C."""

import argparse
import ctypes
import hashlib
from pathlib import Path
import random
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/adpcm_dec"
SOURCE_HASH = "162af092c94771911ba58acc08b73ddb30dcb0d25ac48f7aa72a45060ce90514"
# Every mutable global read/written by Decode, including scratch values and
# histories. Immutable coefficient tables retain their upstream initializers.
STATE = [(name, 1) for name in """
xout1 xout2 xs xd il ilr ih rl dl rh
 dec_deth dec_detl dec_dlt dec_plt dec_plt1 dec_plt2 dec_szl dec_spl dec_sl
 dec_rlt1 dec_rlt2 dec_rlt dec_al1 dec_al2 dec_nbl dec_dh dec_nbh dec_szh
 dec_rh1 dec_rh2 dec_ah1 dec_ah2 dec_ph dec_sph dec_sh dec_ph1 dec_ph2
""".split()] + [(name, size) for name, size in [
    ("accumc", 11), ("accumd", 11), ("dec_del_bpl", 6), ("dec_del_dltx", 6),
    ("dec_del_bph", 6), ("dec_del_dhx", 6),
]]
WORDS = sum(size for _, size in STATE)


def replace(source, old, new, count=1):
    assert source.count(old) == count, old
    return source.replace(old, new)


def reference_source(native_shifts=False):
    source = (FIXTURES / "adpcm_dec.c").read_text()
    assert hashlib.sha256(source.encode()).hexdigest() == SOURCE_HASH
    # The sine/cosine initialization fills encoder-only test_data, never read
    # by this decoder. Keep the decoder, reset and original benchmark driver.
    begin = source.index("/* MAX: 1 */")
    end = source.index("/*\n  Algorithm core functions", begin)
    source = source[:begin] + source[end:]
    begin = source.index("void adpcm_dec_init()\n{")
    end = source.index("int adpcm_dec_return()\n{", begin)
    source = source[:begin] + source[end:]
    source = source[:source.index("int main( void )\n{")]
    source = re.sub(r"\blong(?:\s+int)?\b|\bint\b", "int32_t", source)
    source = re.sub(r"(\d+)L\b", r"\1", source)
    # Define every potentially signed right shift using int64 floor division.
    # The comparison build retains the C compiler's native signed shifts.
    # Whitespace normalization makes these anchors readable without depending on
    # the upstream's line wrapping. Comments/pragmas carry no executable meaning.
    source = re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.S)
    source = re.sub(r'_Pragma\(\s*"[^"]*"\s*\)', "", source)
    macros = "\n".join(re.findall(r"^#define[^\n]*", source, flags=re.M)) + "\n"
    source = re.sub(r"^#define[^\n]*", "", source, flags=re.M)
    source = re.sub(r"\s+", " ", source)
    shifts = [
        ("input >> 6", "oracle_asr(input, 6)"),
        ("adpcm_dec_ilr >> 2", "oracle_asr(adpcm_dec_ilr, 2)"),
        ("( ( int32_t )adpcm_dec_dec_detl * adpcm_dec_qq4_code4_table[ oracle_asr(adpcm_dec_ilr, 2) ] ) >> 15", "oracle_asr(adpcm_dec_dec_detl * adpcm_dec_qq4_code4_table[ oracle_asr(adpcm_dec_ilr, 2) ], 15)"),
        ("( ( int32_t )adpcm_dec_dec_detl * adpcm_dec_qq6_code6_table[ adpcm_dec_il ] ) >> 15", "oracle_asr(adpcm_dec_dec_detl * adpcm_dec_qq6_code6_table[ adpcm_dec_il ], 15)"),
        ("( ( int32_t )adpcm_dec_dec_deth * adpcm_dec_qq2_code2_table[ adpcm_dec_ih ] ) >> 15", "oracle_asr(adpcm_dec_dec_deth * adpcm_dec_qq2_code2_table[ adpcm_dec_ih ], 15)"),
        ("xa1 >> 14", "oracle_asr(xa1, 14)"),
        ("xa2 >> 14", "oracle_asr(xa2, 14)"),
        ("zl >> 14", "oracle_asr(zl, 14)"),
        ("pl >> 15", "oracle_asr(pl, 15)"),
        ("( ( int32_t )nbl * 127 ) >> 7", "oracle_asr(nbl * 127, 7)"),
        ("il >> 2", "oracle_asr(il, 2)"),
        ("nbl >> 6", "oracle_asr(nbl, 6)"),
        ("nbl >> 11", "oracle_asr(nbl, 11)"),
        ("adpcm_dec_ilb_table[ wd1 ] >> ( shift_constant + 1 - wd2 )", "oracle_asr(adpcm_dec_ilb_table[ wd1 ], shift_constant + 1 - wd2)"),
        ("( 255 * bli[ i ] ) >> 8", "oracle_asr(255 * bli[ i ], 8)"),
        ("wd2 >> 7", "oracle_asr(wd2, 7)"),
        ("127 * ( int32_t )al2 >> 7", "oracle_asr(127 * al2, 7)"),
        ("( ( int32_t )al1 * 255 ) >> 8", "oracle_asr(al1 * 255, 8)"),
        ("( ( int32_t )nbh * 127 ) >> 7", "oracle_asr(nbh * 127, 7)"),
    ]
    if not native_shifts:
        for old, new in shifts:
            source = replace(source, old, new, 2 if "bli[ i ]" in old else 1)
        assert ">>" not in source
    # This scale-factor shift is nonnegative and in range; multiplication also
    # avoids depending on the C signed-left-shift rules.
    source = replace(source, "wd3 << 3", "wd3 * 8")
    # The original final post-decrement forms a pointer before the array, even
    # though it is never dereferenced. Keep that last pointer at element zero.
    source = replace(source, "*ac_ptr-- = *ac_ptr1--;", "*ac_ptr-- = *ac_ptr1; ac_ptr1 -= (i < 9);")
    source = replace(source, "*ad_ptr-- = *ad_ptr1--;", "*ad_ptr-- = *ad_ptr1; ad_ptr1 -= (i < 9);")
    source = replace(source, "*ad_ptr = adpcm_dec_xs; return;", "*ad_ptr = adpcm_dec_xs; oracle_capture(2); return;")
    source = replace(source, "adpcm_dec_accumd[ i ] = 0; } return;", "adpcm_dec_accumd[ i ] = 0; } oracle_capture(1); return;")
    # Instrument decoder/helper IF conditions without changing their values.
    edits, conditions = [], []
    for match in re.finditer(r"\bif\s*\(", source):
        start, depth, end = match.end(), 1, match.end()
        while depth:
            depth += (source[end] == '(') - (source[end] == ')')
            end += 1
        condition = source[start:end-1].strip()
        idx = len(conditions)
        conditions.append(condition)
        edits.extend([(start, f"oracle_cond({idx}, !!("), (end-1, "))")])
    for pos, text in sorted(edits, reverse=True):
        source = source[:pos] + text + source[pos:]
    assert len(conditions) == 13, conditions
    prefix = r"""#include <stdint.h>
#include <assert.h>
#include <string.h>
static uint8_t *oracle_output;
static uint32_t oracle_events;
static uint8_t oracle_coverage[26];
static void oracle_capture(int kind);
static int oracle_cond(int id, int value) {
  oracle_coverage[2*id + !!value] = 1;
  return value;
}
static int32_t oracle_asr(int32_t value, int count) {
  assert(count >= 0 && count < 32);
  int64_t n = value, divisor = INT64_C(1) << count;
  return (int32_t)(n >= 0 ? n/divisor : -((-n+divisor-1)/divisor));
}
static void store(uint8_t *p, uint32_t value) {
  for (int i = 0; i < 4; ++i) p[i] = (uint8_t)(value >> (8*i));
}
"""
    capture = "\nstatic void oracle_capture(int kind) {\n *oracle_output++ = (uint8_t)kind;\n"
    for name, size in STATE:
        if size == 1:
            capture += f" store(oracle_output, (uint32_t)adpcm_dec_{name}); oracle_output += 4;\n"
        else:
            capture += f" for (int i=0; i<{size}; ++i) {{ store(oracle_output, (uint32_t)adpcm_dec_{name}[i]); oracle_output += 4; }}\n"
    capture += " ++oracle_events;\n}\n"
    wrapper = "\nint oracle_run(int original, int count, const uint8_t *codes, const uint8_t *resets, uint8_t *output, uint8_t *coverage, uint8_t *result) {\n"
    for name, size in STATE:
        wrapper += f" memset(&adpcm_dec_{name}, 0, sizeof adpcm_dec_{name});\n"
    wrapper += r"""
  memset(adpcm_dec_result, 0, sizeof adpcm_dec_result);
  memset(oracle_coverage, 0, sizeof oracle_coverage);
  oracle_output = output;
  oracle_events = 0;
  adpcm_dec_reset();
  if (original) {
    adpcm_dec_main();
    store(result, (uint32_t)adpcm_dec_return());
    int32_t sum = 0;
    for (int i=0; i<4; ++i) { sum += adpcm_dec_result[i]; store(result+8+4*i, (uint32_t)adpcm_dec_result[i]); }
    store(result+4, (uint32_t)sum);
  } else {
    for (int i=0; i<count; ++i) {
      if (resets[i]) adpcm_dec_reset();
      adpcm_dec_decode(codes[i]);
    }
  }
  memcpy(coverage, oracle_coverage, sizeof oracle_coverage);
  return (int)oracle_events;
}
"""
    return prefix + macros + source + capture + wrapper, conditions


def build_oracle(directory, cc, native):
    name = "native" if native else "floor"
    source, conditions = reference_source(native)
    cfile, library = directory / f"{name}.c", directory / f"{name}.so"
    cfile.write_text(source)
    subprocess.run([cc, "-std=c11", "-O2", "-fwrapv", "-shared", "-fPIC", str(cfile), "-o", str(library)], check=True)
    lib = ctypes.CDLL(str(library))
    fn = lib.oracle_run
    fn.argtypes = [ctypes.c_int, ctypes.c_int] + [ctypes.POINTER(ctypes.c_uint8)]*5
    fn.restype = ctypes.c_int
    return fn, conditions


def generate(cc):
    lines = ["# Generated by tools/generate_adpcm_dec_vectors.py; do not edit.",
             "# state " + " ".join(f"{name}:{size}" for name, size in STATE),
             "# case label original codes resets; then event kind state; then end result (status/checksum/four samples)"]
    coverage = bytearray(26)
    rng = random.Random(0xAD0CDEC)
    cases = [("upstream", True, bytes([0, 253]), bytes(2)),
             ("reset-only", False, b"", b""),
             ("each-code-from-reset", False, bytes(range(256)), bytes([1])*256),
             ("ascending", False, bytes(range(256)), bytes(256)),
             ("descending", False, bytes(reversed(range(256))), bytes(256))]
    for code in [0, 4, 32, 63, 128, 253, 255]:
        cases.append((f"constant-{code}", False, bytes([code])*128, bytes(128)))
    cases += [("alternating", False, bytes([0, 255, 4, 128])*64, bytes(256)),
              ("random", False, bytes(rng.randrange(256) for _ in range(256)), bytes(256)),
              ("random-with-resets", False, bytes(rng.randrange(256) for _ in range(256)),
               bytes(int(i in [1, 2, 11, 12, 63, 127, 128, 254]) for i in range(256)))]
    events, decodes = 0, 0
    with tempfile.TemporaryDirectory(prefix="actionc-adpcm-dec-") as temp:
        oracle, conditions = build_oracle(Path(temp), cc, False)
        native, _ = build_oracle(Path(temp), cc, True)
        for label, original, codes, resets in cases:
            assert len(codes) == len(resets) <= 256
            data = [(ctypes.c_uint8 * len(x)).from_buffer_copy(x) for x in [codes, resets]]
            def execute(fn):
                out = (ctypes.c_uint8 * (513*(1+4*WORDS)))()
                hits, result = (ctypes.c_uint8 * 26)(), (ctypes.c_uint8 * 24)(*[0xCC]*24)
                count = fn(original, len(codes), *data, out, hits, result)
                assert count == 1 + len(codes) + (0 if original else sum(resets))
                return bytes(out[:count*(1+4*WORDS)]), bytes(hits), bytes(result)
            output, hits, result = execute(oracle)
            assert execute(native) == (output, hits, result), (label, "native shift comparison")
            if original:
                assert result == bytes(4) + (-2).to_bytes(4, 'little', signed=True) + b''.join(v.to_bytes(4, 'little', signed=True) for v in [0, 0, -1, -1]), result.hex()
            lines.append(f"case {label} {int(original)} {codes.hex() or '-'} {resets.hex() or '-'}")
            for start in range(0, len(output), 1+4*WORDS):
                record = output[start:start+1+4*WORDS]
                lines.append(f"event {record[0]} {record[1:].hex()}")
                events += 1
            lines.append(f"end {result.hex()}")
            decodes += len(codes)
            for i, hit in enumerate(hits):
                coverage[i] |= hit
    summary = f"{len(cases)} cases; {decodes} codewords; {events} state checkpoints; {sum(coverage)}/26 IF outcomes"
    print(summary)
    for i, condition in enumerate(conditions):
        if not all(coverage[i*2:i*2+2]):
            print(f"  partial condition {i}: {condition} -> {list(coverage[i*2:i*2+2])}")
    lines.insert(1, "# " + summary)
    return "\n".join([*lines, ""])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = generate(args.cc)
    path = FIXTURES / "vectors.txt"
    if args.check:
        if path.read_text() != generated:
            raise SystemExit("adpcm_dec vectors are stale; regenerate them")
        print("adpcm_dec vectors match the pinned C reference")
    else:
        path.write_text(generated)


if __name__ == "__main__":
    main()
