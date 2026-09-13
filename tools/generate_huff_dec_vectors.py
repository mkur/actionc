#!/usr/bin/env python3
"""Generate Huffman decoder vectors from pinned TACLeBench C, never Action!."""

import argparse
import ctypes
import hashlib
import heapq
from pathlib import Path
import random
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/huff_dec"
SOURCE_HASH = "3c525668e2b1af4ad22ca202fd9212c74778f49d8c94f57384691fef8e647a62"
INPUT_BYTES = 12288
TABLE_BYTES = 257 * 35
POOL_BYTES = 514 * 6
OUTPUT_BYTES = 24 + 1024 + TABLE_BYTES + POOL_BYTES


def pinned():
    source = (FIXTURES / "huff_dec.c").read_text()  # normalize host CRLF
    if hashlib.sha256(source.encode()).hexdigest() != SOURCE_HASH:
        raise ValueError("Pinned huff_dec.c changed; review provenance/adaptations")
    return source


def reference_source():
    source = pinned()
    source = source[:source.index("int main( void )\n{")]
    source = source.replace("int main( void );", "")
    # Preserve the original 32-bit unsigned arithmetic, including stale upper
    # reservoir bits. Node symbols/counts fit in the Action! CARD representation.
    source = re.sub(r"\bunsigned int\b", "uint32_t", source)
    source = re.sub(r"\bunsigned\b(?!\s+char)", "uint32_t", source)
    source = source.replace("#define huff_dec_encoded_len 419",
                            "static int huff_dec_encoded_len;\n"
                            "static const unsigned char *huff_dec_encoded;")
    source = source.replace("huff_dec_encoded[ huff_dec_encoded_len ] =",
                            "upstream_encoded[419] =")
    source = source.replace("  t_bin_val encoding_table[ 257 ];", "")
    source = source.replace("  huff_dec_t_tree heap[ 514 ]; /* space for dynamically allocated nodes */", "")
    anchor = "void _Pragma( \"entrypoint\" ) huff_dec_main( void )"
    source = source.replace(anchor, "static t_bin_val encoding_table[257];\n"
                            "static huff_dec_t_tree heap[514];\n" + anchor)
    source = source.replace("  return ( ptr_tree );", "  oracle_nodes = heap_top;\n  return ( ptr_tree );")
    source = source.replace("    ptr_tree = huff_dec_tree_encoding( encoding_table, heap );",
                            "    ptr_tree = huff_dec_tree_encoding( encoding_table, heap );\n"
                            "    oracle_root = 0x6501;")
    source = source.replace("  return huff_dec_encoded[ huff_dec_input_pos++ ];",
                            "  assert(huff_dec_input_pos < huff_dec_encoded_len);\n"
                            "  return huff_dec_encoded[ huff_dec_input_pos++ ];")
    source = source.replace("  huff_dec_output[ huff_dec_output_pos++ ] = ch;",
                            "  assert(huff_dec_output_pos < 1024);\n"
                            "  huff_dec_output[ huff_dec_output_pos++ ] = ch;")
    source = source.replace("&heap[ heap_top++ ]", "&heap[ oracle_slot(heap_top++) ]")
    # Observe original if/while outcomes without changing evaluation order.
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
static uint16_t oracle_nodes, oracle_root;
static uint8_t oracle_branches[{conditions * 2}];
static int oracle_cond(int id, int value) {{
  oracle_branches[id * 2 + !!value] = 1;
  return value;
}}
static unsigned oracle_slot(unsigned index) {{ assert(index < 514); return index; }}
"""
    wrapper = r"""
static uint32_t load(const uint8_t *p, int n) {
  uint32_t value = 0;
  for (int i = 0; i < n; ++i) value |= (uint32_t)p[i] << (8*i);
  return value;
}
static void store(uint8_t *p, uint32_t value, int n) {
  for (int i = 0; i < n; ++i) p[i] = (uint8_t)(value >> (8*i));
}
static uint16_t address(huff_dec_t_tree *p) {
  if (!p) return 0;
  assert(p >= heap && p < heap + 514);
  return (uint16_t)(0x6501 + 6*(p - heap));
}
void oracle_original(uint8_t *encoded, uint8_t *plain) {
  memcpy(encoded, upstream_encoded, 419);
  memcpy(plain, huff_dec_plaintext, 600);
}
void oracle_run(const uint8_t *header, const uint8_t *input,
                uint8_t *output, uint8_t *coverage) {
  memset(encoding_table, 0, sizeof encoding_table);
  memset(heap, 0, sizeof heap);
  memset(huff_dec_output, 0xCC, sizeof huff_dec_output);
  memset(oracle_branches, 0, sizeof oracle_branches);
  huff_dec_encoded = input;
  huff_dec_encoded_len = load(header + 2, 2);
  huff_dec_input_pos = load(header + 4, 2);
  huff_dec_output_pos = load(header + 6, 2);
  huff_dec_byte_nb_to_read = header[8];
  huff_dec_val_to_read = load(header + 10, 4);
  oracle_root = load(header + 20, 2);
  oracle_nodes = load(header + 22, 2);
  uint32_t result = load(header + 16, 4);
  switch (header[0]) {
    case 0: huff_dec_main(); break;
    case 1: result = huff_dec_read_code_n_bits(load(header + 14, 2)); break;
    case 2: result = huff_dec_read_code_1_bit(); break;
    default: assert(0);
  }
  memcpy(output, header, 24);
  store(output + 4, huff_dec_input_pos, 2);
  store(output + 6, huff_dec_output_pos, 2);
  output[8] = huff_dec_byte_nb_to_read;
  store(output + 10, huff_dec_val_to_read, 4);
  store(output + 16, result, 4);
  store(output + 20, oracle_root, 2);
  store(output + 22, oracle_nodes, 2);
  memcpy(output + 24, huff_dec_output, 1024);
  uint8_t *table = output + 24 + 1024;
  for (int i = 0; i < 257; ++i) {
    assert(encoding_table[i].bits_nb <= 256);
    memcpy(table + i*35, encoding_table[i].bits, 32);
    store(table + i*35 + 32, encoding_table[i].bits_nb, 2);
    table[i*35 + 34] = encoding_table[i].presence;
  }
  uint8_t *pool = table + 257*35;
  for (int i = 0; i < 514; ++i) {
    assert(heap[i].byte <= 257);
    store(pool + i*6, heap[i].byte, 2);
    store(pool + i*6 + 2, address(heap[i].left_ptr), 2);
    store(pool + i*6 + 4, address(heap[i].right_ptr), 2);
  }
  memcpy(coverage, oracle_branches, sizeof oracle_branches);
}
"""
    return prefix + source + wrapper, conditions


def stream(codes, message, bitmap=False, long_lengths=False):
    """Independent wire-format writer; no Huffman decoder or Action! involved."""
    present = sorted(set(codes) - {256})
    assert 256 in codes and present
    words = sorted(codes.values())
    assert all(not b.startswith(a) for a, b in zip(words, words[1:]))
    assert all(1 <= len(code) <= 256 for code in words)
    bits = []

    def emit(value, width):
        assert 0 <= value < 1 << width
        bits.extend(f"{value:0{width}b}")

    emit(int(bitmap), 1)
    if bitmap:
        for symbol in range(256):
            emit(int(symbol in codes), 1)
    else:
        assert len(present) <= 32
        emit(len(present) - 1, 5)
        for symbol in present:
            emit(symbol, 8)
    for symbol in sorted(codes):
        word = codes[symbol]
        wide = long_lengths or len(word) > 32
        emit(int(wide), 1)
        emit(len(word) - 1, 8 if wide else 5)
        bits.extend(word)
    for symbol in [*message, 256]:
        bits.extend(codes[symbol])
    bits.extend("0" * (-len(bits) % 8))
    return bytes(int("".join(bits[i:i+8]), 2) for i in range(0, len(bits), 8))


def huffman_codes(symbols, rng):
    queue = [(rng.randrange(1, 1000), i, {symbol: ""}) for i, symbol in enumerate(symbols)]
    heapq.heapify(queue)
    serial = len(queue)
    while len(queue) > 1:
        weight_a, _, a = heapq.heappop(queue)
        weight_b, _, b = heapq.heappop(queue)
        codes = {symbol: "0" + word for symbol, word in a.items()}
        codes.update({symbol: "1" + word for symbol, word in b.items()})
        heapq.heappush(queue, (weight_a + weight_b, serial, codes))
        serial += 1
    return queue[0][2]


def generate(cc):
    source, conditions = reference_source()
    with tempfile.TemporaryDirectory(prefix="actionc-huff-dec-") as temporary:
        directory = Path(temporary)
        cfile, library = directory / "oracle.c", directory / "oracle.so"
        cfile.write_text(source)
        subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                        str(cfile), "-o", str(library)], check=True)
        reference = ctypes.CDLL(str(library))
        pointer = ctypes.POINTER(ctypes.c_uint8)
        reference.oracle_run.argtypes = [pointer, pointer, pointer, pointer]
        reference.oracle_run.restype = None
        reference.oracle_original.argtypes = [pointer, pointer]
        reference.oracle_original.restype = None
        coverage = bytearray(conditions * 2)
        rows = []
        rng = random.Random(0x48554646)

        def run(label, encoded, plain=None, command=0, count=0, left=0, reservoir=0, pos=0):
            assert len(encoded) <= INPUT_BYTES
            header = bytearray(24)
            header[0], header[1], header[8], header[9] = command, 0x5A, left, 0x5B
            header[2:4] = len(encoded).to_bytes(2, "little")
            header[4:6] = pos.to_bytes(2, "little")
            header[10:14] = reservoir.to_bytes(4, "little")
            header[14:16] = count.to_bytes(2, "little")
            header[16:20] = bytes.fromhex("78563412")
            output = (ctypes.c_uint8 * OUTPUT_BYTES)()
            hits = (ctypes.c_uint8 * len(coverage))()
            reference.oracle_run((ctypes.c_uint8 * 24).from_buffer_copy(header),
                                 (ctypes.c_uint8 * len(encoded)).from_buffer_copy(encoded), output, hits)
            output = bytes(output)
            if plain is not None:
                assert len(plain) <= 1024
                assert int.from_bytes(output[6:8], "little") == len(plain), label
                assert output[24:24+len(plain)] == plain, label
                assert output[24+len(plain):1048] == bytes([0xCC]) * (1024-len(plain)), label
            for i, hit in enumerate(hits):
                coverage[i] |= hit
            table_end = 1048 + TABLE_BYTES
            fields = [label, header.hex(), encoded.hex() or "-", output[:24].hex(),
                      output[24:1048].rstrip(b"\xCC").hex() or "-",
                      output[1048:table_end].rstrip(b"\0").hex() or "-",
                      output[table_end:].rstrip(b"\0").hex() or "-"]
            rows.append(" ".join(fields))
            return output

        original, plain = (ctypes.c_uint8 * 419)(), (ctypes.c_uint8 * 600)()
        reference.oracle_original(original, plain)
        run("upstream", bytes(original), bytes(plain))
        run("empty-input", b"", b"")
        # Both header formats, singleton alphabets, all byte values, output/page
        # boundaries, and randomized trees built independently of the C decoder.
        for number, size in enumerate([1, 2, 7, 31, 32, 33, 127, 255, 256]):
            symbols = sorted(rng.sample(range(256), size))
            codes = huffman_codes([*symbols, 256], rng)
            message = bytes(symbols + [rng.choice(symbols) for _ in range(257)])
            for bitmap in ([False, True] if size <= 32 else [True]):
                run(f"alphabet-{size}-{'bitmap' if bitmap else 'list'}",
                    stream(codes, message, bitmap), message)
        for length in [0, 1, 7, 8, 15, 16, 255, 256, 257, 1024]:
            codes = {0: "0", 256: "1"}
            message = bytes(length)
            run(f"zeros-{length}", stream(codes, message, long_lengths=length % 2 == 1), message)
        for depth in [7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 255, 256]:
            # A full comb-shaped tree reaches length 256 with exactly 513 nodes.
            codes = {i: "1" * i + "0" for i in range(depth)}
            codes[256] = "1" * depth
            message = bytes([depth-1, 0, depth//2, depth-1])
            run(f"depth-{depth}", stream(codes, message, bitmap=depth > 32), message)
        for number in range(12):
            size = rng.randrange(2, 257)
            symbols = sorted(rng.sample(range(256), size))
            codes = huffman_codes([*symbols, 256], rng)
            message = bytes(rng.choice(symbols) for _ in range(rng.randrange(1, 1025)))
            run(f"random-{number:02d}", stream(codes, message, bitmap=True,
                                             long_lengths=number % 2 == 0), message)
        # Preserve the C bit-reader's exact state, including unmasked upper
        # reservoir bits on its full-consumption branch. Only valid reads:
        # enough input bits, request <=16, buffered count <=16.
        for left in range(17):
            for count in sorted({0, 1, 5, 8, 16, left}):
                reservoir = rng.getrandbits(32)
                data = bytes(rng.randrange(256) for _ in range(4))
                run(f"bits-{left}-{count}", data, command=1, count=count,
                    left=left, reservoir=reservoir)
            if left:
                run(f"bits-eof-{left}", b"", command=1, count=left,
                    left=left, reservoir=rng.getrandbits(32))
            run(f"bit-{left}", bytes([0x81]), command=2, left=left,
                reservoir=rng.getrandbits(32))
        for pos in [255, 256, 257]:
            run(f"bits-page-{pos}", bytes(range(256)) * 2, command=1, count=16, pos=pos)

        # The original return/checksum function is retained for attribution but
        # the wrapper compares full output; its comparison IF is not executed.
        missing = {i for i, value in enumerate(coverage) if not value}
        assert missing == {0, 1}, f"Unexpected uncovered C outcomes: {missing}"
        header = [
            "# Generated by tools/generate_huff_dec_vectors.py; do not edit.",
            f"# vectors={len(rows)} if_while_outcomes={sum(coverage)}/{len(coverage)}",
            "# label input_header input_hex expected_header output_cc_tail table_zero_tail pool_zero_tail",
            "# Trailing fill bytes are omitted; '-' means entirely fill bytes.",
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
            raise SystemExit("Huffman decoder vectors are stale; regenerate them")
        print("Huffman decoder vectors match the pinned C reference")
    else:
        destination.write_text(text, newline="\n")
        print(f"Wrote {destination.relative_to(ROOT)} ({len(text)} bytes)")


if __name__ == "__main__":
    main()
