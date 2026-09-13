#!/usr/bin/env python3
"""Generate encoder state/quantizer vectors from pinned TACLeBench C."""

import argparse
import ctypes
import hashlib
from pathlib import Path
import random
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/adpcm_enc"
SOURCE_HASH = "a7993c77e3898040664274127662d2ed94bf18f3e1ec1ee4cac008132b282e88"
STATE = [("encoded", 1)] + [(name, 1) for name in """
xl xh il szl spl sl el nbl al1 al2 plt plt1 plt2 dlt rlt rlt1 rlt2 detl deth
sh eh dh ih nbh szh sph ph yh ah1 ah2 ph1 ph2 rh1 rh2
""".split()] + [("tqmf", 24), ("delay_bpl", 6), ("delay_dltx", 6),
               ("delay_bph", 6), ("delay_dhx", 6), ("test_data", 6), ("compressed", 3)]
WORDS = sum(size for _, size in STATE)


def replace(source, old, new, count=1):
    assert source.count(old) == count, old
    return source.replace(old, new)


def reference_source(native=False):
    source = (FIXTURES / "adpcm_enc.c").read_text()
    assert hashlib.sha256(source.encode()).hexdigest() == SOURCE_HASH
    source = source[:source.index("int main( void )\n{")]
    # C int arithmetic remains 32-bit; this revision explicitly uses 64-bit
    # long long products/accumulators. Never narrow those to match Action!.
    source = re.sub(r"\blong long(?:\s+int)?\b", "int64_t", source)
    source = re.sub(r"\blong(?:\s+int)?\b|\bint\b", "int32_t", source)
    source = re.sub(r"(\d+)L\b", r"\1", source)
    source = re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.S)
    source = re.sub(r'_Pragma\(\s*"[^"]*"\s*\)', "", source)
    source = re.sub(r"^#pragma[^\n]*", "", source, flags=re.M)
    macros = "\n".join(re.findall(r"^#define[^\n]*", source, flags=re.M)) + "\n"
    source = re.sub(r"^#define[^\n]*", "", source, flags=re.M)
    source = re.sub(r"\s+", " ", source)
    shifts = [
        ("( xa + xb ) >> 15", "oracle_asr(xa + xb, 15)"),
        ("( xa - xb ) >> 15", "oracle_asr(xa - xb, 15)"),
        ("adpcm_enc_il >> 2", "oracle_asr(adpcm_enc_il, 2)"),
        ("( ( int64_t ) adpcm_enc_detl * adpcm_enc_qq4_code4_table[ oracle_asr(adpcm_enc_il, 2) ] ) >> 15", "oracle_asr((int64_t)adpcm_enc_detl * adpcm_enc_qq4_code4_table[ oracle_asr(adpcm_enc_il, 2) ], 15)"),
        ("( 564 * ( int64_t )adpcm_enc_deth ) >> 12", "oracle_asr(564 * (int64_t)adpcm_enc_deth, 12)"),
        ("( ( int64_t )adpcm_enc_deth * adpcm_enc_qq2_code2_table[ adpcm_enc_ih ] ) >> 15", "oracle_asr((int64_t)adpcm_enc_deth * adpcm_enc_qq2_code2_table[ adpcm_enc_ih ], 15)"),
        ("zl >> 14", "oracle_asr(zl, 14)"),
        ("pl >> 15", "oracle_asr(pl, 15)"),
        ("( adpcm_enc_decis_levl[ mil ] * ( int64_t )detl ) >> 15", "oracle_asr(adpcm_enc_decis_levl[ mil ] * (int64_t)detl, 15)"),
        ("( ( int64_t )nbl * 127 ) >> 7", "oracle_asr((int64_t)nbl * 127, 7)"),
        ("il >> 2", "oracle_asr(il, 2)"),
        ("nbl >> 6", "oracle_asr(nbl, 6)"),
        ("nbl >> 11", "oracle_asr(nbl, 11)"),
        ("adpcm_enc_ilb_table[ wd1 ] >> ( shift_constant + 1 - wd2 )", "oracle_asr(adpcm_enc_ilb_table[ wd1 ], shift_constant + 1 - wd2)"),
        ("( 255 * bli[ i ] ) >> 8", "oracle_asr(255 * bli[ i ], 8)"),
        ("wd2 >> 7", "oracle_asr(wd2, 7)"),
        ("127 * ( int64_t )al2 >> 7", "oracle_asr(127 * (int64_t)al2, 7)"),
        ("( ( int64_t )al1 * 255 ) >> 8", "oracle_asr((int64_t)al1 * 255, 8)"),
        ("( ( int64_t )nbh * 127 ) >> 7", "oracle_asr((int64_t)nbh * 127, 7)"),
    ]
    if not native:
        for old, new in shifts:
            source = replace(source, old, new, 2 if "bli[ i ]" in old else 1)
        assert ">>" not in source
        # Explicit modulo-2^32 narrowing at the potentially oversized results.
        source = replace(source, "adpcm_enc_xl = oracle_asr(xa + xb, 15);", "adpcm_enc_xl = oracle_i32(oracle_asr(xa + xb, 15));")
        source = replace(source, "adpcm_enc_xh = oracle_asr(xa - xb, 15);", "adpcm_enc_xh = oracle_i32(oracle_asr(xa - xb, 15));")
        source = replace(source, "( int32_t )( oracle_asr(zl, 14) )", "oracle_i32(oracle_asr(zl, 14))")
        source = replace(source, "( int32_t )( oracle_asr(pl, 15) )", "oracle_i32(oracle_asr(pl, 15))")
    source = replace(source, "wd3 << 3", "wd3 * 8")
    # Avoid the final unused pointer stepping before tqmf[0]. Accesses unchanged.
    source = replace(source, "*tqmf_ptr-- = *tqmf_ptr1--;", "{ *tqmf_ptr-- = *tqmf_ptr1; tqmf_ptr1 -= (i < 21); }")
    source = replace(source, "adpcm_enc_tqmf[ i ] = 0; return;", "adpcm_enc_tqmf[ i ] = 0; oracle_capture(1); return;")
    source = replace(source, "adpcm_enc_test_data[ i ] += x; } }", "adpcm_enc_test_data[ i ] += x; } oracle_capture(3); }")
    source = replace(source, "adpcm_enc_compressed[ i / 2 ] = adpcm_enc_encode( adpcm_enc_test_data[ i ], adpcm_enc_test_data[ i + 1 ] );",
        "{ adpcm_enc_compressed[ i / 2 ] = adpcm_enc_encode( adpcm_enc_test_data[ i ], adpcm_enc_test_data[ i + 1 ] ); oracle_encoded = adpcm_enc_compressed[i/2]; oracle_capture(2); }")
    source = replace(source, "return ( ril );", "oracle_bins[2*mil + (el >= 0)] = 1; return ( ril );")
    # Check the narrow-coefficient bounds used by the Action! 32-by-16 MAC.
    source = replace(source, "zl = ( int64_t )( *bpl++ )", "for (int32_t j=0; j<6; ++j) assert(bpl[j] >= INT16_MIN && bpl[j] <= INT16_MAX); zl = ( int64_t )( *bpl++ )")
    source = replace(source, "pl = 2 * rlt1;", "assert(al1 >= INT16_MIN && al1 <= INT16_MAX && al2 >= INT16_MIN && al2 <= INT16_MAX); pl = 2 * rlt1;")
    # Count genuine upstream IF outcomes (not our bounds assertions).
    conditions, edits = [], []
    for match in re.finditer(r"\bif\s*\(", source):
        start, end, depth = match.end(), match.end(), 1
        while depth:
            depth += (source[end] == '(') - (source[end] == ')')
            end += 1
        idx = len(conditions)
        conditions.append(source[start:end-1].strip())
        edits.extend([(start, f"oracle_cond({idx}, !!("), (end-1, "))")])
    for pos, value in sorted(edits, reverse=True):
        source = source[:pos] + value + source[pos:]
    assert len(conditions) == 19, conditions
    prefix = r"""#include <stdint.h>
#include <assert.h>
#include <string.h>
static uint8_t *oracle_output;
static uint32_t oracle_events;
static int32_t oracle_encoded;
static uint8_t oracle_coverage[38], oracle_bins[62];
static void oracle_capture(int kind);
static int oracle_cond(int id, int value) {
  oracle_coverage[2*id + !!value] = 1;
  return value;
}
static int64_t oracle_asr(int64_t n, int count) {
  assert(count >= 0 && count < 63);
  int64_t divisor = INT64_C(1) << count;
  int64_t q = n/divisor;
  return q - (n < 0 && n%divisor != 0);
}
static int32_t oracle_i32(int64_t n) {
  uint32_t bits = (uint32_t)n;
  return bits <= INT32_MAX ? (int32_t)bits : -1 - (int32_t)(UINT32_MAX-bits);
}
static void store(uint8_t *p, uint32_t value) {
  for (int i=0; i<4; ++i) p[i] = (uint8_t)(value >> (8*i));
}
static int32_t load(const uint8_t *p) {
  uint32_t bits = 0;
  for (int i=0; i<4; ++i) bits |= (uint32_t)p[i] << (8*i);
  return oracle_i32(bits);
}
"""
    capture = "\nstatic void oracle_capture(int kind) {\n *oracle_output++ = (uint8_t)kind;\n"
    for name, size in STATE:
        name = "oracle_encoded" if name == "encoded" else f"adpcm_enc_{name}"
        if size == 1:
            capture += f" store(oracle_output, (uint32_t){name}); oracle_output += 4;\n"
        else:
            capture += f" for (int i=0; i<{size}; ++i) {{ store(oracle_output, (uint32_t){name}[i]); oracle_output += 4; }}\n"
    capture += " ++oracle_events;\n}\n"
    wrapper = "\nint oracle_run(int command, int count, const uint8_t *pairs, const uint8_t *resets, uint8_t *output, uint8_t *coverage, uint8_t *result) {\n"
    for name, size in STATE:
        name = "oracle_encoded" if name == "encoded" else f"adpcm_enc_{name}"
        wrapper += f" memset(&{name}, 0, sizeof {name});\n"
    wrapper += r"""
  memset(oracle_coverage, 0, sizeof oracle_coverage);
  memset(oracle_bins, 0, sizeof oracle_bins);
  oracle_output = output;
  oracle_events = 0;
  if (command == 0) {
    adpcm_enc_init();
    adpcm_enc_main();
    store(result, (uint32_t)adpcm_enc_return());
    store(result+4, (uint32_t)(adpcm_enc_compressed[0] + adpcm_enc_compressed[1]));
  } else {
    adpcm_enc_reset();
    for (int i=0; i<count; ++i) {
      if (resets[i]) adpcm_enc_reset();
      int32_t a=load(pairs+8*i), b=load(pairs+8*i+4);
      oracle_encoded = command == 1 ? adpcm_enc_encode(a, b) : adpcm_enc_quantl(a, b);
      oracle_capture(2);
    }
  }
  memcpy(coverage, oracle_coverage, sizeof oracle_coverage);
  memcpy(coverage+38, oracle_bins, sizeof oracle_bins);
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
    fn = ctypes.CDLL(str(library)).oracle_run
    fn.argtypes = [ctypes.c_int, ctypes.c_int] + [ctypes.POINTER(ctypes.c_uint8)]*5
    fn.restype = ctypes.c_int
    return fn, conditions


def packed(values):
    return b''.join((v & 0xFFFFFFFF).to_bytes(4, 'little') for v in values)


def signed32(value):
    return (value + 2**31) % 2**32 - 2**31


def generate(cc):
    lines = ["# Generated by tools/generate_adpcm_enc_vectors.py; do not edit.",
             "# state " + " ".join(f"{name}:{size}" for name, size in STATE),
             "# case label command sample-pairs resets; then event kind state; then end status/checksum"]
    rng = random.Random(0xAD0CEAC)
    cases = [("upstream", 0, [(0, 0)]*2, [0]*2), ("reset-only", 1, [], [])]
    for value in [0, 32767, -32768]:
        cases.append((f"constant-{value}", 1, [(value, value)]*128, [0]*128))
    cases += [
        ("alternating", 1, [(32767, -32768), (-32768, 32767)]*64, [0]*128),
        ("ramp", 1, [(i*512-32768, 32767-i*512) for i in range(128)], [0]*128),
        ("random-pcm", 1, [(rng.randint(-32768, 32767), rng.randint(-32768, 32767)) for _ in range(128)], [0]*128),
        ("random-wide", 1, [(rng.randint(-2**31, 2**31-1), rng.randint(-2**31, 2**31-1)) for _ in range(128)], [0]*128),
        ("warm-resets", 1, [(rng.randint(-32768, 32767), rng.randint(-32768, 32767)) for _ in range(128)], [int(i in [1, 11, 12, 63, 64, 126, 127]) for i in range(128)]),
    ]
    # Full-width samples exercise carries/sign extension in the 64-bit QMF.
    edges = [-2**31, -2**31+1, -65536, -32768, -1, 0, 1, 32767, 65535, 2**31-1]
    cases.append(("wide-boundaries", 1, [(a, b) for a in edges for b in edges], [0]*100))
    cases.append(("impulses", 1, [(32767, 0)] + [(0, 0)]*23 + [(0, -32768)] + [(0, 0)]*23, [0]*48))
    c_source = (FIXTURES / "adpcm_enc.c").read_text()
    def table(name):
        values = re.search(r"adpcm_enc_" + name + r"\[\s*\d+\s*\] = \{([^}]+)", c_source)[1]
        return [int(x) for x in values.replace(',', ' ').split()]
    levels = table("decis_levl")
    coefficients = table("h")
    positive, negative = table("quant26bt_pos"), table("quant26bt_neg")
    offsets, offset = {}, 0
    for name, size in STATE:
        offsets[name] = offset
        offset += size
    quant = []
    for scale in [32, 16384]:
        for threshold in levels:
            decision = threshold*scale//32768
            for offset in [-1, 0, 1]:
                for sign in [-1, 1]:
                    quant.append((sign*(decision+offset), scale))
        quant += [(-2**31, scale), (2**31-1, scale)]
    for i in range(0, len(quant), 128):
        batch = quant[i:i+128]
        cases.append((f"quantizer-{i//128}", 2, batch, [0]*len(batch)))
    coverage, bins, events, encodes, quantized = bytearray(38), bytearray(62), 0, 0, 0
    wide_witnesses = 0
    with tempfile.TemporaryDirectory(prefix="actionc-adpcm-enc-") as temp:
        oracle, conditions = build_oracle(Path(temp), cc, False)
        native, _ = build_oracle(Path(temp), cc, True)
        for label, command, pairs, resets in cases:
            assert len(pairs) == len(resets) <= 128
            inputs, flags = packed([v for pair in pairs for v in pair]), bytes(resets)
            data = [(ctypes.c_uint8 * len(x)).from_buffer_copy(x) for x in [inputs, flags]]
            def execute(fn):
                out = (ctypes.c_uint8 * (258*(1+4*WORDS)))()
                hits, report = (ctypes.c_uint8 * 100)(), (ctypes.c_uint8 * 8)(*[0xCC]*8)
                count = fn(command, len(pairs), *data, out, hits, report)
                assert count == (4 if command == 0 else 1+len(pairs)+sum(resets))
                return bytes(out[:count*(1+4*WORDS)]), bytes(hits), bytes(report)
            output, hits, report = execute(oracle)
            assert execute(native) == (output, hits, report), (label, "native C comparison")
            if command == 0:
                assert report == packed([0, 385]), report.hex()
                # Record the actual initialized input, although command 0 asks
                # the Action! program to compute it with its own initializer.
                stride = 1 + 4*WORDS
                init_state = output[stride+1:2*stride]
                first = offsets["test_data"]*4
                inputs = init_state[first:first+16]
            lines.append(f"case {label} {command} {inputs.hex() or '-'} {flags.hex() or '-'}")
            previous, pair_index = None, 0
            for start in range(0, len(output), 1+4*WORDS):
                record = output[start:start+1+4*WORDS]
                state = [int.from_bytes(record[i:i+4], 'little', signed=True)
                         for i in range(1, len(record), 4)]
                if record[0] == 2 and command != 2:
                    history = previous[offsets["tqmf"]:offsets["tqmf"]+24]
                    xa = sum(history[i]*coefficients[i] for i in range(0, 24, 2))
                    xb = sum(history[i]*coefficients[i] for i in range(1, 24, 2))
                    for name, value in [("xl", xa+xb), ("xh", xa-xb)]:
                        assert state[offsets[name]] == signed32(value//32768), (label, name)
                        wide_witnesses += value//32768 != signed32(value)//32768
                elif record[0] == 2:
                    error, scale = pairs[pair_index]
                    magnitude = signed32(abs(error))
                    mil = next((i for i, level in enumerate(levels)
                                if magnitude <= level*scale//32768), 30)
                    assert state[0] == (positive if error >= 0 else negative)[mil], label
                pair_index += record[0] == 2
                previous = state
                lines.append(f"event {record[0]} {record[1:].hex()}")
                events += 1
            lines.append(f"end {report.hex()}")
            encodes += len(pairs) if command != 2 else 0
            quantized += len(pairs) if command == 2 else 0
            for i, hit in enumerate(hits[:38]): coverage[i] |= hit
            for i, hit in enumerate(hits[38:]): bins[i] |= hit
    assert all(bins), [i for i, hit in enumerate(bins) if not hit]
    assert wide_witnesses > 0, "vectors must distinguish 64-bit from truncated QMF sums"
    summary = f"{len(cases)} cases; {encodes} sample pairs; {quantized} quantizer inputs; {events} state checkpoints; {sum(coverage)}/38 IF outcomes; 62/62 signed quantizer bins"
    print(summary)
    print(f"  {wide_witnesses} QMF results require accumulation beyond 32 bits")
    for i, condition in enumerate(conditions):
        if not all(coverage[i*2:i*2+2]): print(f"  partial condition {i}: {condition} -> {list(coverage[i*2:i*2+2])}")
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
            raise SystemExit("adpcm_enc vectors are stale; regenerate them")
        print("adpcm_enc vectors match the pinned C reference")
    else:
        path.write_text(generated)


if __name__ == "__main__":
    main()
