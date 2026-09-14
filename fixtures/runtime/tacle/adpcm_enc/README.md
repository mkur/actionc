# TACLeBench ADPCM encoder

`adpcm_enc.act` ports TACLeBench's two-band ADPCM encoder. Two signed samples
produce one byte codeword through the transmit quadrature mirror filter,
adaptive predictors, quantization and scale-factor updates. It uses ordinary
Action! records, arrays, loops and calls, without recursion or floating point.

## Provenance

- [Pinned upstream source](https://github.com/tacle/tacle-bench/blob/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/sequential/adpcm_enc/adpcm_enc.c),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 2.0.
- Author: Sung-Soo Lim, SNU-RT Benchmark Suite; original algorithm source:
  *C Algorithms for Real-Time DSP* by P. M. Embree.
- The SNU-RT Benchmark Suite permits use, modification and redistribution with
  acknowledgement. `adpcm_enc.c` preserves the unmodified source and permission
  notice. The Action! adaptation was added in 2026.
- LF-normalized SHA-256 of `adpcm_enc.c`:
  `a7993c77e3898040664274127662d2ed94bf18f3e1ec1ee4cac008132b282e88`.

## Arithmetic and adaptation

This upstream revision uses **32-bit int and 64-bit long long**. The signal
values and persistent state use LONGINT. Wide filter and predictor sums use a
`Wide` record containing two LONGCARD halves. `Mac` splits a signed 32-bit value
into an unsigned low 16-bit half and a signed high half, multiplies both by a
signed 16-bit coefficient, and adds the exact product with carry propagation.
`Scaled` extracts the low 32 bits after shifting the 64-bit sum by 14 or 15.
The sums retain their precision until the upstream int conversion.

Every QMF coefficient fits INT. Starting from reset, zero-predictor coefficients
remain within INT: `floor(255*b/256) +/- 128` preserves that range. Pole
coefficients are explicitly clamped. This allows the exact 32-by-16 MAC in all
three accumulation sites. The C generator asserts the predictor coefficient
bounds throughout the streams.

The remaining products fit LONGINT under the scale-factor/coefficient bounds.
Signed shifts use [`MATH.INTEGER.AsrLI`](../../../../docs/INTEGER_SHIFTS.md).
Tests of the sign of a 64-bit product become `SameSign`, which also handles
zero. `Filtep` preserves C's 32-bit doubling of each reconstructed value before
widening it into the accumulator. Array indices and shift counts use BYTE
where their bounds permit it.

`Quantl` retains the 30-level search, early exit and final table entry.
`Sin32` and `Cos32` retain the original integer initializer, including wrapping
32-bit products and division that truncates toward zero. Their generated input
is larger than ordinary PCM samples, so retaining the wide filter matters even
for the original benchmark. Unused five-/six-bit reconstruction tables are
omitted; all tables used by the encoder retain their original values.

Two upstream behaviors are preserved and marked in the source:

- `Upzero` skips delay element 2.
- `Reset` clears only `tqmf(0)` through `tqmf(22)`. Element 23 survives a reset
  during a stream. Scratch values are also left alone, as upstream.

`Main` runs initialization, encodes two sample pairs, and checks the result:

- Initialized samples: **1736745450, -701575096, -701575096, 0, 0, 0**.
- Encoded bytes: **253, 132**; the unused third output remains zero.
- Checksum: **385**; result **0** for that checksum, **1** otherwise.

## Reference vectors

The generator compiles the pinned C with `int32_t`/`int64_t` types and `-fwrapv`.
Signed shifts use 64-bit floor division and potentially oversized conversions
explicitly retain the low 32 bits. A second build retains native signed shifts
and narrowing; every checkpoint and checksum must agree. The last unused
pointer decrement is adjusted to avoid forming a pointer before `tqmf[0]`,
without changing any data access. Host serialization is explicitly little-endian.

Python independently checks the QMF sums against arbitrary-precision arithmetic
and checks the quantizer search at each direct test input. **454 QMF results**
distinguish full accumulation from truncating the sum to 32 bits before shifting.

There are **15 cases**, **1,174 encoded sample pairs**, **364 direct quantizer
inputs**, and **1,561 checkpoints**:

- The complete original initializer and benchmark, plus reset without input.
- Zero, positive/negative constant, alternating, ramp and random PCM streams.
- Full-width random/boundary samples, impulses, and resets during a stream.
- Both signs immediately below, at and above quantizer thresholds at scale
  factors 32 and 16384, including LONGINT minimum/maximum.

The vectors reach **62/62 signed quantizer bins** and **35/38 IF outcomes** in
the C source. The upper `Uppol2` clamp and the two `Uppol1` clamp branches are
not taken by these streams. This is condition coverage, not full path coverage.

## Target separation and validation

The maintained source has no absolute addresses or host protocol. Its storage,
including the two-word records, is compiler-allocated. Root tests verify raw
and optimized NIR and lower both to MIR6502, MIR68k and both MIR65816 layouts.
Execution is currently tested in the 6502 VM.

The adapter checks optimized classic and MIR6502 with cartridge and standalone
runtimes: **60 VM executions** and **6,244 checkpoints**. It checks all **92
32-bit values** at each checkpoint: the returned codeword, every mutable encoder
state value, initialized samples and the original compressed-output array.
Checkpoints follow resets, initialization and each encode/quantizer call.
Each stream runs in one VM; each mode/runtime compiles the adapter once.
Standalone execution loads no ROMs. LF and CRLF reach actual instrumentation,
vector parsing, compilation and execution.

Only the adapter owns these 6502 addresses:

| Address | Meaning |
| --- | --- |
| `$0600` | Command: 0 original benchmark, 1 supplied sample pairs, 2 Quantl |
| `$0601` | CARD pair count, 0 through 128 |
| `$0603` | Checkpoint: 1 reset, 2 result, 3 initialized; host writes zero to resume |
| `$0608` | LONGINT status and checksum for command 0 |
| `$06FF` | Completion marker `$A5` |
| `$07FF` | Up to 256 LONGINT input values, preserved |
| `$0CFF` | Reset-before-pair flags, preserved |
| `$0EFF` | 92 LONGINT state values in vector-header order |

The object starts at `$1200`, leaving room below cartridge ROM for the classic
code and runtime. Every byte of `$0600..$11FF` is compared at every checkpoint
and completion, including page-crossing buffers, guards and unchanged inputs.
The original initializer needs a larger instruction budget because it performs
thousands of integer divisions.

```sh
cargo test --test adpcm_enc
cd tools/vm-runtime-tests
cargo test --locked --test adpcm_enc
```

Generate or verify vectors from the repository root with Python 3 and a
GCC-compatible compiler (`--cc clang` is supported):

```sh
python3 tools/generate_adpcm_enc_vectors.py
python3 tools/generate_adpcm_enc_vectors.py --check
```

Normal tests consume the stored vectors without a host C compiler or network.

## Compiler regression exposed by the port

Classic code generation previously retained cached scratch-byte facts across
writes through differently sized integer views. In an expression such as
`10*Echo(LONGINT(2000)*3141*index)`, that could remove a required zero-extension
load and corrupt the inner argument. Direct memory-byte facts now share physical
byte identity across views, with overlapping dependencies invalidated on writes.
A separate small regression in `classic_long_integers` checks nested calls,
signed extension and guarded stores independently of this benchmark.

## Native MC68000 acceptance

The r68k adapter runs all 15 cases and 1,561 complete state checkpoints in both raw and optimized NIR modes. It uses compiler-emitted symbols and numeric byte-order conversion, with LF and CRLF through actual source instrumentation. The Action! algorithm and C-reference vectors are shared with the 6502 tests.

```sh
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --test adpcm_enc
```
