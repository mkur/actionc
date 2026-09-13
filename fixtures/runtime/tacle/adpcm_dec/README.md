# TACLeBench ADPCM decoder

`adpcm_dec.act` ports TACLeBench's two-band ADPCM decoder. Each byte codeword
produces two signed samples through inverse quantization, adaptive zero/pole
predictors, and a receive quadrature mirror filter. It exercises signed 32-bit
multiplication, arithmetic right shifts, table lookups, array parameters,
clamping, and persistent delay lines, without recursion or floating point.

## Provenance

- [Pinned upstream source](https://github.com/tacle/tacle-bench/blob/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/sequential/adpcm_dec/adpcm_dec.c),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 1.x.
- Author: Sung-Soo Lim, SNU-RT Benchmark Suite; original algorithm source:
  *C Algorithms for Real-Time DSP* by P. M. Embree.
- The SNU-RT Benchmark Suite permits use, modification and redistribution with
  acknowledgement. `adpcm_dec.c` retains the unmodified upstream source and its
  attribution and permission notice. The Action! adaptation was added in 2026.
- LF-normalized SHA-256 of `adpcm_dec.c`:
  `162af092c94771911ba58acc08b73ddb30dcb0d25ac48f7aa72a45060ce90514`.

## Action! adaptation

C's signal values, coefficients, `int` and `long` calculations use LONGINT with
32-bit wrapping arithmetic. Signed shifts use
[`MATH.INTEGER.AsrLI`](../../../../docs/INTEGER_SHIFTS.md), whose count is BYTE.
Codewords and small loop counters use BYTE; table indices are explicitly
narrowed to BYTE after their bounded calculations. Flat array indexing replaces
C pointer walks. The predictor functions retain their call structure and
operation order.

`Reset` clears the persistent predictor/filter history and restores scale
factors 32 and 8. As upstream, it leaves scratch values and previous output
samples alone; subsequent decoding overwrites them. `Decode(codeword)` updates
that state and writes `xout1` and `xout2`. `Run` decodes the first two entries of
the original `[0,253,32]` input. `Main` calls `Reset`, `Run`, and `CheckResult`:

- Samples: **0, 0, -1, -1**.
- Checksum: **-2**.
- Result: **0** for that checksum, **1** otherwise.

The unused encoder state and sine/cosine input generator are omitted. The
latter fills `test_data`, which the decoder never reads; its actual input is
`compressed`.

Outputs follow the upstream implementation, including two preserved quirks:

- The six-bit reconstruction table is indexed by the encoder's `il`, which
  stays zero in this decoder benchmark, rather than the received `ilr`.
- `Upzero` does not assign delay element 2. It remains zero after a reset.

Both are marked in the Action! source. They are part of the reference behavior
being tested, rather than corrections to the codec algorithm.

## Reference vectors and coverage

The generator compiles the pinned C decoder with explicit `int32_t` types and
`-fwrapv`, including removal of host-width `L` suffixes. Potentially signed right
shifts use mathematical floor division in `int64_t`. A second C build retains
native signed right shifts; every checkpoint and original benchmark result must
agree. Serialization is explicitly little-endian and independent of host byte
order. The generator also avoids forming an unused pointer before the start of
a delay array during the last post-decrement; data accesses remain unchanged.

The **15 cases** decode **2,434 codewords** and check **2,713 state checkpoints**:

- Original benchmark and a reset with no input.
- Every codeword from 0 through 255, resetting before each one.
- Ascending, descending, constant, alternating and deterministic random streams.
- Resets during a stream, including consecutive resets and filter/page boundaries.

Each reset and decode checks all **83 mutable 32-bit state values**: both output
samples, scale factors, predictor coefficients, scratch values, and every
predictor/QMF delay element. The vector header specifies their order. The C
instrumentation exercises **25/26 IF outcomes**; the true branch of
`apl2 > 12288` in `Uppol2` is not reached by these streams. This is measured
condition coverage, not complete path coverage.

## Target separation and VM checks

The maintained Action! source uses compiler-allocated storage and has no absolute
addresses or host protocol. Root tests verify raw and optimized NIR and lower
both to MIR6502, MIR68k and both MIR65816 layouts. Native execution is currently
covered only by the 6502 VM.

The VM adapter adds checkpoint capture and a driver, compiling once per
mode/runtime and running each stream in one VM. It checks optimized classic and
MIR6502 with cartridge and standalone runtimes: **60 VM executions**, **9,736
codewords**, and **10,852 checkpoints**. Standalone execution loads no ROMs.
Both LF and CRLF reach source instrumentation, vector parsing and compilation.

Only the adapter owns this 6502 memory map:

| Address | Meaning |
| --- | --- |
| `$0600` | Original-benchmark flag |
| `$0601` | CARD codeword count, 0 through 256 |
| `$0603` | Checkpoint: 1 reset, 2 decode; host writes zero to continue |
| `$0608` | Six LONGINT values: status, checksum, four original samples |
| `$06FF` | Completion marker `$A5` |
| `$07FF` | Up to 256 input bytes, preserved |
| `$09FF` | Reset-before-codeword flags, preserved |
| `$0BFF` | 83 LONGINT state values |

The object starts at `$2000`. Every byte of `$0600..$0FFF` is compared at each
checkpoint and completion, including guards and unchanged inputs. State and
input arrays cross page boundaries deliberately. Expected state comes from the
committed C vectors; tests require no host C compiler or network access.

```sh
cargo test --test adpcm_dec
cd tools/vm-runtime-tests
cargo test --locked --test adpcm_dec
```

Regenerate or verify vectors from the repository root with Python 3 and a
GCC-compatible C compiler (`--cc clang` is also supported):

```sh
python3 tools/generate_adpcm_dec_vectors.py
python3 tools/generate_adpcm_dec_vectors.py --check
```
