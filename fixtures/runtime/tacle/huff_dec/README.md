# TACLeBench Huffman decoder

`huff_dec.act` ports David Bourgin's Huffman decoder from TACLeBench using
existing Action! records, typed pointers, arrays, and bit operations. Tree
construction and traversal are iterative. The full **257-entry code table**,
**514-node pool**, **32-byte code words**, and **1,024-byte output buffer** are
retained. No new language constructs or allocator are needed.

This is a host-driven compiler fixture. `huff_dec.act` allocates portable storage
and shares `types.inc` and `kernel.inc` with the
[6502 driver](../../../../tools/vm-runtime-tests/fixtures/huff_dec_driver.act).
That driver runs in both modern 6502 backends and runtimes at **`$8000`**, above
the host packet and below the cartridge at `$A000`. The portable driver also
runs from public MC68000 CLI artifacts.

## Provenance

- [Pinned upstream source](https://github.com/tacle/tacle-bench/blob/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/sequential/huff_dec/huff_dec.c),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 2.0.
- Author: **David Bourgin**, 1994; originally `dcodhuff.c`. TACLeBench replaced
  file I/O with arrays and dynamic allocation with a fixed node pool.
- `huff_dec.c` is the unmodified reference, retaining the complete copyright
  and permission notice. That notice requires the author's name in the source
  header; its other requests are explicitly optional. The Action! source also
  retains his attribution.
- LF-normalized SHA-256:
  `3c525668e2b1af4ad22ca202fd9212c74778f49d8c94f57384691fef8e647a62`.
- The Action! adaptation was added on 2026-09-13 under the repository's
  [GNU GPL license](../../../../LICENSE), with the original notice preserved.

## Adaptations and source behavior

Each code record has 32 bytes of bits, a CARD length, and a BYTE presence flag
(35 bytes on 6502). Each tree node has a CARD symbol and two typed pointers
(six bytes on 6502). Native layout is queried from the compiler.
Symbols 256 and 257 mean end-of-message and internal node, respectively.
Pointers name actual pool records; bit 1 chooses the left child.

Counts, positions, symbols, and lengths fit CARD. The bit reservoir and ReadBits
result use **LONGCARD**, preserving the C reference's 32-bit unsigned arithmetic.
In particular, ReadBits retains stale upper reservoir bits and does not mask
them on its full-consumption branch. Tests compare this behavior exactly.
Explicit casts narrow bit results; C increment expressions become statements
in the same order. The original main routine becomes Decode; Main adds the host
commands and completion marker. Root address and allocated-node count are
exposed for comparison without changing construction order.

The host supplies a complete, valid stream, zero positions and bit state, and
zeroed code/tree arrays before each decode. This initialization is significant:
the upstream sparse header only sets present symbols and does **not** clear
absent presence flags. The C wrapper initializes its working arrays explicitly,
avoiding uninitialized automatic storage. Empty input leaves them untouched.
There is no added malformed-stream recovery or Action! bounds checking. Inputs
must fit the declared buffers and tree pool and contain an end-of-message code.

## Host memory contract

In the 6502 packet, words and pointer addresses are little-endian. A tree pointer is null or
`$6501 + 6*index`, for index 0..513.

| Address | Meaning |
| --- | --- |
| `$0600` | Command: 0 Decode, 1 ReadBits(request), 2 ReadBit |
| `$0602` | Encoded input length, CARD |
| `$0604`, `$0606` | Input and output positions, CARD |
| `$0608` | Buffered bit count, BYTE |
| `$060A` | Bit reservoir, LONGCARD |
| `$060E` | Requested bit count, CARD |
| `$0610` | Bit-reader result, LONGCARD; unchanged by Decode |
| `$0614`, `$0616` | Root address and allocated-node count, CARD |
| `$06FF` | Completion marker, `$A5` |
| `$0801..$3800` | Encoded input, capacity 12,288 bytes |
| `$3901..$3D00` | Decoded output, capacity 1,024 bytes |
| `$4001..$6323` | 257 code records |
| `$6501..$710C` | 514 tree records |

The unused bytes in the 24-byte control area are preserved. Commands 1 and 2
accept seeded reader state for focused tests. The tested ReadBits contract has
requests 0..16 and buffered counts 0..16, with enough bits available; Decode
itself requests at most eight bits at a time.

The native adapter reads each field as a numeric value or exact byte string.
Tree links travel through slot identities and the native pool's symbol-derived
base and stride. The portable `rootAddress` is a Tree POINTER. Command 255 is a
test-only `SIZEOF`/`OFFSETOF` query, executed once per compiled image; it avoids
assuming native padding or field offsets in the host adapter.

## Coverage and checks

`vectors.txt` contains **185 C-reference cases**, run in all four backend/runtime
combinations (**740 VM executions**):

- 52 complete decode cases, including the original **419-byte stream yielding
  the original 600-byte text**, empty input, both header formats, both code-length
  formats, all byte values, output lengths through 1,024, and deterministic random
  trees and messages.
- Code lengths around byte/word boundaries and through **256 bits**. The deepest
  full tree uses **513 nodes**, covering indexes above 255 and the final code byte.
- 116 multi-bit and 17 single-bit reader cases, including zero requests,
  refill/consumption boundaries, end-of-input buffered reads, stale upper bits,
  and input indexes 255/256/257.

The C instrumentation reaches every **38/38 decoder and bit-reader if/while
outcomes**. The two outcomes in the unused original return/checksum wrapper are
excluded; regeneration checks that these are the only missing outcomes among
the 40 instrumented outcomes. This is branch coverage, not full path coverage.

Every execution compares the entire control area, output buffer, code table,
and node pool, including unused bytes and every pointer. It checks the unchanged
input and all surrounding guard bytes in `$0600..$7FFF`. Structures start at odd
addresses and cross pages. Standalone runs load no ROMs. LF and CRLF source and
vector text pass through the actual compiler, parser, and VM path.

The MC68000 target adds **370 executions**, all 185 cases in raw and optimized
NIR from CLI artifacts. It compares the complete logical header, output and
poison tail, all code bits/lengths/presence flags, every tree symbol and link,
root identity, node count and unchanged input. Native record padding is poisoned
and checked separately from reference fields. Both source includes and vectors
pass through the actual LF/CRLF paths. No recursive or malformed-stream behavior
is added.

The port exposed two general compiler bugs, fixed with regression coverage:
MIR6502 parameter-register availability survived stores that materialize through
A, and classic word equality lost A while preparing an indirect operand address.
The Action! port retains its original statement order and direct field comparisons.

```sh
cd tools/vm-runtime-tests
cargo test --locked --test huff_dec
```

## Regenerating vectors

```sh
python3 tools/generate_huff_dec_vectors.py
python3 tools/generate_huff_dec_vectors.py --check
```

Regeneration requires Python 3 and a GCC-compatible C compiler (`--cc clang` is
supported). It verifies the pinned source hash, compiles the C decoder with
explicit 32-bit unsigned arithmetic, supplies initialized working storage, and
serializes fields independently of host padding, pointer width, and endianness.
C wrapper assertions catch input/output and node-pool overruns.

An independent Python bitstream writer creates prefix-free codebooks and known
messages; the C output must match those messages before a vector is accepted.
Expected decoder state always comes from C, never from the Action! program.
The original stream and plaintext are read directly from the vendored reference.
Normal Rust tests need neither a C compiler nor network access.

Vectors encode binary bytes as hex. Output fields omit trailing `$CC` bytes;
code/pool fields omit trailing zeros. `-` means the entire field is fill bytes.
The parser restores complete buffers before comparison, so this compact format
does not omit checks of unused storage.
