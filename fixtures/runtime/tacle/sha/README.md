# TACLeBench SHA-0

`sha.act` ports TACLeBench's SHA kernel to Action!, with unsigned 32-bit
`LONGCARD` arithmetic. It preserves the 80-word message schedule, four groups
of 20 rounds, initialization, block update, and final padding. This is **SHA-0**:
the schedule omits the extra rotation introduced by SHA-1.

The fixture exercises wide additions and carry propagation, shifts and
rotations, bitwise expressions, indexed LONGCARD reads and writes, byte/word
views of the same storage, pointer parameters, loops, and calls. It uses the
modern language profile in both the classic (`--mode optimized`) and MIR6502
backends, with cartridge and standalone runtimes. It has no OS I/O.

## Provenance and license

- Original authors: Peter C. Gutmann, heavily modified by Uwe Hollerbach.
- Source: TACLeBench `bench/kernel/sha`, pinned at
  [`c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`](https://github.com/tacle/tacle-bench/tree/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/kernel/sha).
- `sha.c` and `sha.h` are unmodified upstream files, retaining their notices.
  The C source identifies the GNU Lesser General Public License without a
  version; the header separately permits use, modification, and redistribution.
  A copy of LGPL 2.1 is included in [COPYING.LESSER](COPYING.LESSER).
- `sha.act` is the Action! adaptation added on 2026-09-13, under GNU LGPL.
- LF-normalized SHA-256 hashes:
  `sha.c`: `e730bd5d5b3e0d67f7a6078a3b64a00da9a82452e15759f23e29ee4c21e4ef81`;
  `sha.h`: `af50a8e0614bcd3bc085f13345f221bd1256a8251fb210a36241321a2613fa10`.

## Adaptations

C's `unsigned long` becomes `LONGCARD`, exactly 32 bits on every host/target.
The C reference also uses `uint32_t` and fixed-width constants, so host `long`
size cannot change its arithmetic. The two-word bit counter is retained; the
port does not require a native 64-bit type. C's bitwise complement becomes XOR
with `$FFFFFFFF`. Rotations remain explicit shift/OR expressions.

The five digest words, two counters, and block buffer have fixed RAM addresses
instead of a C structure. The block buffer has byte and LONGCARD array views.
The schedule is exposed in RAM for validation instead of being a local C array.
Ordinary loops replace memcpy/memset; the four round groups retain the source
expressions and assignment order. The finalizer combines the two identical
zero-fill paths without changing padding or block processing.

The benchmark's file wrapper and 32,743-byte fixed input are replaced by
host-supplied messages of up to 1,025 bytes. As in upstream, each Update starts
at a block boundary: preceding updates must contain complete 64-byte blocks,
and only the last update may be partial. The fixture feeds its message through
one Update call. It does not introduce an arbitrary-chunk streaming API.

The upstream return check only checks the message length stored in the final
block. This test compares the **entire digest, both counters, final block, and
all 80 schedule words** against the C reference instead (412 bytes per case).

## VM contract

| Address | Meaning |
| --- | --- |
| `$0600` | Command: 0 hash message; 1 raw compression; 2 seeded Update/Final |
| `$0602..$0603` | Message length, little-endian, 0..1025 |
| `$06FF` | Completion marker, written as `$A5` |
| `$0701..$0714` | Five digest words |
| `$0715..$0718` | Low word of bit count |
| `$0719..$071C` | High word of bit count |
| `$071D..$075C` | Sixteen block words / 64-byte input block |
| `$07FD..$093C` | Eighty schedule words |
| `$0A01..$0E01` | Input message bytes |

Host-visible words use little-endian byte order; the conventional digest text
prints each of the five words most-significant byte first. Command 1 consumes
the host's digest and block words directly and leaves counters unchanged.
Command 2 retains seeded state; its initial bit count must be a multiple of 512,
consistent with Update's block-boundary contract. These synthetic states check
counter carry/wrap without hashing half a gigabyte in the VM.

The host poisons `$0600..$0EFF` and verifies all bytes outside the declared output
regions, including input, command, length, and gaps. The schedule is unaligned,
crosses two page boundaries, and requires indexes beyond the first 256 bytes.

## Coverage and running

`vectors.txt` contains **155 cases**, executed in four backend/runtime
combinations (**620 VM executions**):

- Two published SHA-0 answers (`abc` and the 56-byte FIPS message), also listed
  in [OpenSSL's SHA-0 test](https://github.com/openssl/openssl/blob/OpenSSL_1_0_2-stable/crypto/sha/shatest.c).
- 75 ramp messages, including every length 0..65 and selected larger lengths
  around block/page boundaries, up to 1,025 bytes.
- 24 repeated-byte messages and 16 deterministic random messages.
- 20 compression cases with arbitrary initial chaining and block words.
- 18 seeded counter cases covering low-word carry, high-word wrap, and
  high-word length encoding in the final block.

The C oracle must match the two published answers before writing vectors.
The Rust tests independently check those answers in the vector file. Both LF
and CRLF source/vector text pass through the actual compiler, parser, and VM
path: cartridge uses LF, standalone uses CRLF. Standalone runs load no ROMs.

```sh
cd tools/vm-runtime-tests
cargo test --locked --test sha
```

## Regenerating the C reference vectors

From the repository root:

```sh
python3 tools/generate_sha_vectors.py
python3 tools/generate_sha_vectors.py --check
```

Regeneration needs Python 3 and a GCC-compatible C compiler (`--cc clang` is
also supported). It verifies the vendored source hashes and compiles a temporary
reference library. The adapter replaces the file/memory glue, makes byte decoding
independent of host endianness, and exposes the final local schedule. It retains
the original compression, initialization, update, and finalization code.
Serialization explicitly writes little-endian words, without copying C structs.

Expected results never come from the Action! port. Normal Rust/VM tests use only
the committed vectors and require neither a C compiler nor network access.
