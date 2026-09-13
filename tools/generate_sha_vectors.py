#!/usr/bin/env python3
"""Regenerate SHA-0 VM vectors from TACLeBench C, never from the Action! port.

Only regeneration needs Python and a GCC-compatible C compiler. The VM tests
read committed vectors and need neither network access nor a C toolchain.
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
FIXTURES = ROOT / "fixtures/runtime/tacle/sha"
HASHES = {
    "sha.c": "e730bd5d5b3e0d67f7a6078a3b64a00da9a82452e15759f23e29ee4c21e4ef81",
    "sha.h": "af50a8e0614bcd3bc085f13345f221bd1256a8251fb210a36241321a2613fa10",
}
STATE_BYTES = 92  # five digest words, two counters, sixteen block words
OUTPUT_BYTES = STATE_BYTES + 80 * 4  # also expose the final message schedule


def pinned(name):
    # read_text normalizes CRLF before checking the pinned source or adapting it.
    text = (FIXTURES / name).read_text()
    if hashlib.sha256(text.encode()).hexdigest() != HASHES[name]:
        raise ValueError(f"Pinned {name} changed; review provenance and adaptations")
    return text


def reference_source():
    header = pinned("sha.h").replace("typedef unsigned long LONG;", "typedef uint32_t LONG;")
    header = header.replace("typedef unsigned size_t;", "")
    source = pinned("sha.c")
    source = re.sub(r'^#include "[^\"]+"\n', "", source, flags=re.M)
    # Replace only the file/memory glue. Keep the compression, initialization,
    # update, and finalization code from the pinned C implementation.
    start = source.index("size_t sha_fread(")
    end = source.index("/* update the SHA digest */", start)
    source = source[:start] + source[end:]
    source = source[:source.index("/* compute the SHA digest of a FILE stream */")]
    source = source.replace("sha_glibc_memcpy", "memcpy").replace("sha_glibc_memset", "memset")
    source = re.sub(r"\b(0x[0-9a-fA-F]+|[0-9]+)L\b", r"UINT32_C(\1)", source)
    # The original byte reversal assumes a little-endian C host. Decode each
    # input word explicitly so the oracle also works on a big-endian host.
    start = source.index("void sha_byte_reverse(")
    end = source.index("/* initialize the SHA digest */", start)
    source = source[:start] + """void sha_byte_reverse(LONG *buffer, int count) {
  const uint8_t *bytes = (const uint8_t *)buffer;
  for (int i = 0; i < count / 4; ++i) {
    uint32_t word = ((uint32_t)bytes[4*i] << 24)
                  | ((uint32_t)bytes[4*i+1] << 16)
                  | ((uint32_t)bytes[4*i+2] << 8) | bytes[4*i+3];
    buffer[i] = word;
  }
}
""" + source[end:]
    anchor = "  sha_info->digest[ 4 ] += E;"
    assert source.count(anchor) == 1
    source = source.replace(anchor, anchor + "\n  memcpy(oracle_schedule, W, sizeof W);")
    prefix = "#include <stdint.h>\n#include <stddef.h>\n#include <string.h>\n"
    prefix += "static uint32_t oracle_schedule[80];\n"
    wrapper = r"""
static uint32_t load32(const uint8_t *p) {
  return (uint32_t)p[0] | ((uint32_t)p[1] << 8)
       | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static void store32(uint8_t *p, uint32_t value) {
  for (int i = 0; i < 4; ++i) p[i] = (uint8_t)(value >> (8*i));
}
void oracle_run(int command, int length, const uint8_t *state,
                const uint8_t *message, uint8_t *output) {
  for (int i = 0; i < 5; ++i) sha_info.digest[i] = load32(state + 4*i);
  sha_info.count_lo = load32(state + 20);
  sha_info.count_hi = load32(state + 24);
  for (int i = 0; i < 16; ++i) sha_info.data[i] = load32(state + 28 + 4*i);
  if (command == 1) {
    sha_transform(&sha_info);
  } else {
    if (command == 0) sha_init();
    sha_update(&sha_info, (BYTE *)message, length);
    sha_final(&sha_info);
  }
  for (int i = 0; i < 5; ++i) store32(output + 4*i, sha_info.digest[i]);
  store32(output + 20, sha_info.count_lo);
  store32(output + 24, sha_info.count_hi);
  for (int i = 0; i < 16; ++i) store32(output + 28 + 4*i, sha_info.data[i]);
  for (int i = 0; i < 80; ++i) store32(output + 92 + 4*i, oracle_schedule[i]);
}
"""
    return prefix + header + source + wrapper


def generate(cc):
    with tempfile.TemporaryDirectory(prefix="actionc-sha-") as temporary:
        directory = Path(temporary)
        cfile, library = directory / "oracle.c", directory / "oracle.so"
        cfile.write_text(reference_source())
        subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                        str(cfile), "-o", str(library)], check=True)
        oracle = ctypes.CDLL(str(library)).oracle_run
        pointer = ctypes.POINTER(ctypes.c_uint8)
        oracle.argtypes = [ctypes.c_int, ctypes.c_int, pointer, pointer, pointer]
        oracle.restype = None
        rng = random.Random(0x53484130)
        vectors = []

        def random_bytes(length):
            return bytes(rng.randrange(256) for _ in range(length))

        def run(label, message, command=0, state=None):
            if state is None:
                # Init must overwrite all state, regardless of previous contents.
                state = random_bytes(STATE_BYTES)
            output = (ctypes.c_uint8 * OUTPUT_BYTES)()
            oracle(command, len(message), (ctypes.c_uint8 * STATE_BYTES).from_buffer_copy(state),
                   (ctypes.c_uint8 * len(message)).from_buffer_copy(message), output)
            vectors.append((label, command, state.hex(), message.hex() or "-", bytes(output).hex()))
            return bytes(output)

        # Published SHA-0 answers, also recorded in OpenSSL's shatest.c.
        for label, message, digest in [
            ("abc", b"abc", "0164b8a914cd2a5e74c4f7ff082c4d97f1edf880"),
            ("fips-56", b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
             "d2516ee1acfa5baf33dfc1c471e438449ef134c8"),
        ]:
            output = run(label, message)
            actual = b"".join(output[i:i+4][::-1] for i in range(0, 20, 4)).hex()
            if actual != digest:
                raise RuntimeError(f"C reference failed published SHA-0 answer for {label}: {actual}")

        # Every final-block length, including both padding branches. Selected
        # larger cases cross block, byte-index, and input page boundaries.
        for length in list(range(65)) + [65, 119, 120, 127, 128, 129, 255, 256, 257, 1025]:
            run(f"ramp-{length}", bytes(i % 256 for i in range(length)))
        for length in [1, 55, 56, 63, 64, 65, 128, 257]:
            for pattern in [0, 0xFF, 0xAA]:
                run(f"fill-{pattern:02x}-{length}", bytes([pattern]) * length)
        for number in range(16):
            length = rng.randrange(1, 513)
            run(f"random-{number:02d}-{length}", random_bytes(length))

        # Raw compression tests cover arbitrary chaining words and schedules,
        # not just states reachable from the standard initialization vector.
        for pattern in [0, 0xFF, 0x80, 0x55]:
            run(f"transform-fill-{pattern:02x}", b"", 1, bytes([pattern]) * STATE_BYTES)
        for number in range(16):
            run(f"transform-random-{number:02d}", b"", 1, random_bytes(STATE_BYTES))

        # Seed counters at full-block boundaries, matching Update's contract.
        # This tests low-word carry, high-word wrap, and 64-bit length encoding
        # without executing half a gigabyte of input on a 6502 VM.
        for high in [0, 0x7FFFFFFF, 0xFFFFFFFF]:
            for length in [0, 63, 64, 65, 120, 128]:
                state = bytearray(random_bytes(STATE_BYTES))
                state[20:24] = (0xFFFFFE00).to_bytes(4, "little")
                state[24:28] = high.to_bytes(4, "little")
                run(f"counter-{high:08x}-{length}", random_bytes(length), 2, state)

        header = [
            "# Generated by tools/generate_sha_vectors.py; do not edit.",
            f"# SHA-0 state_bytes={STATE_BYTES} output_bytes={OUTPUT_BYTES} vectors={len(vectors)}",
            "# label command state_hex message_hex_or_dash expected_state_and_schedule_hex",
        ]
        return "\n".join(header + [" ".join(map(str, row)) for row in vectors]) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="cc", help="GCC-compatible C compiler")
    parser.add_argument("--check", action="store_true", help="verify committed vectors")
    args = parser.parse_args()
    text = generate(args.cc)
    destination = FIXTURES / "vectors.txt"
    if args.check:
        if destination.read_text() != text:
            raise SystemExit("SHA-0 vectors are stale; regenerate them")
    else:
        destination.write_text(text)
    print(text.splitlines()[1])
    print(f"SHA-256 {hashlib.sha256(text.encode()).hexdigest()}")


if __name__ == "__main__":
    main()
