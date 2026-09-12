# Copy reduction around wide shifts

Implemented in MIR6502 target lowering, 2026-09-12. Baseline: `eb1ca62`.

## Contract

Constant LONGINT/LONGCARD shifts by 4, 8, 12, 16, 20, 24, or 28 bits use byte
projections and independent nibble shifts. Other counts retain their current
word strategies. Zero counts preserve the input, counts at least 32 produce
zero, and dynamic counts retain the wide helpers. RSH remains logical for
both signed and unsigned bit patterns.

For example, with little-endian captured input bytes `b0..b3`,
`CARD(value RSH 12)` needs:

```text
result low  = (b1 >> 4) | (b2 << 4), truncated to one byte
result high = (b2 >> 4) | (b3 << 4), truncated to one byte
```

The former word decomposition computed `(low >> 12) | (high << 4)`. Its zero
high byte still passed through an OR, and intermediate word results added
stores and reloads. The new lowering computes independent result bytes and
assembles them with the existing structured `MirValue::Word` form. Computing
each crossing nibble first lets ordinary materialization keep the main nibble
in A for the OR, eliminating another store/reload pair per merged byte.

Raw MIR still defines the complete 32-bit result. Existing materialization
and liveness proofs eliminate bytes discarded by CARD/INT/BYTE consumers.
There is no matcher for a particular sample or library, source reassociation,
new instruction form, scratch requirement, or helper ABI change. SemIR/NIR
semantics and the classic backend are unchanged. This legalization is valid
with optional peepholes disabled as well.

All explicit input loads and calls are preserved in their original order.
Projection uses captured values, including when stores overlap the source,
a call overwrites it, or other consumers retain the full result. In particular,
a volatile wide input is still read in full even when the result is one byte
or a constant zero.

## Regression coverage

Three compiler tests check raw byte computation without zero merges, full
input capture, both runtimes, default/optimized/disabled-peephole configurations,
direct shift-to-OR materialization, and the dynamic helper fallback.
`fixtures/mir6502/wide_shift_projection.act` adds a raw MIR contract snapshot;
existing snapshots are unchanged. The broad passing fixture count increases
from 348 to 349; the 12 known semantic failures are unchanged.

The VM shift matrix covers every count 0..33 plus 256, 65536, and $FFFFFFFF,
both directions, signed/unsigned full results, byte/word truncation, and captured
calls across all three compiler modes and both runtimes. Additional tests cover
shared results across calls and successor blocks, parameters and returns,
overlapping stores, deterministic random inputs, and the exact order of all
four volatile reads followed by an overlapping byte write. Whole output pages
and explicit completion are checked.

Acceptance passes NIR and MIR fixture snapshots, all 51 NIR sweep fixtures,
all 169 MIR sweep fixtures, and the complete VM suite (305 tests), including
the full Atari/VBXE Mandelbrot image oracles. Existing snapshots do not change;
the new snapshot records the intentional byte-projection lowering contract.

The full compiler suite passes in an isolated checkout containing this patch
(3,122 passed, 24 ignored). Isolation excludes the existing uncommitted
wireframe support files: `samples/vbxe/shared/lines.act` imports `SHARED.SCREEN`,
which the general `parses_all_sample_programs` loader does not resolve. Those
unrelated working-tree changes remain untouched. Compiler and test source
bytes in the isolated checkout match the working tree exactly.

## Matched measurements

Standalone MIR6502 with the normal compiler settings, the same source and
multiplier kernel on both sides, and the existing inliner cost policy.
Atari origin is `$2000`; VBXE and the focused probes use `$3000`. Both square
calls remain inlined and `MulFloor` remains a call in both measured layouts.

| Measure | Before | After |
| --- | ---: | ---: |
| `CARD(input RSH 12)` XEX, including headers | 93 bytes | 79 bytes |
| `CARD(input RSH 12)` CPU cost through the result store | 121 cycles | 101 cycles |
| Full `input RSH 12` XEX | 111 bytes | 97 bytes |
| Full `input RSH 12` CPU cost through the result store | 148 cycles | 128 cycles |
| `CARD(input LSH 12)` XEX | 47 bytes | 47 bytes |
| `CARD(input LSH 12)` CPU cost through the result store | 46 cycles | 46 cycles |
| Atari Mandelbrot XEX | 2,377 bytes | 2,331 bytes |
| VBXE Mandelbrot XEX | 4,366 bytes | 4,320 bytes |
| Either renderer, `Iterate` routine | 546 bytes | 512 bytes |
| Either renderer, retained `MulFloor` routine | 141 bytes | 129 bytes |
| Either renderer, 640-point recurrence (6,167 updates) | 16,047,747 cycles | 15,659,226 cycles |
| Either renderer, 16,000-point recurrence (151,649 updates) | 409,695,879 cycles | 400,141,992 cycles |

The original-grid recurrence uses 9,553,887 fewer cycles (2.33%). Multiplication
is unchanged at 263,652,760 cycles and 480,613 calls; coordinate mapping is also
unchanged. Every escape count is compared with an independent integer oracle,
and every listed instruction byte is checked against the actual XEX.
These are recurrence CPU costs, excluding plotting and display DMA/interrupts;
they are not complete-frame speed measurements.

Rebuild either renderer with:

```sh
cargo run --locked --bin actionc -- --mode mir6502 --runtime standalone --module-path samples/vbxe --origin '$2000' --output build/wide-shift-copy/after-atari.xex --listing build/wide-shift-copy/after-atari.lst samples/graphics/mandelbrot/mbfixed.act
cargo run --locked --bin actionc -- --mode mir6502 --runtime standalone --module-path samples/vbxe --origin '$3000' --output build/wide-shift-copy/after-vbxe.xex --listing build/wide-shift-copy/after-vbxe.lst samples/graphics/mandelbrot/mbfixed-vbxe.act
```

Artifacts and validation logs are under ignored `build/wide-shift-copy/`.
The recurrence profiler is `build/oscar64-mandelbrot/action-profile.rs`; the
focused probe profiler is `build/wide-shift-copy/probe-profile.rs`. Both use
Actionc VM revision `7ec0cc454ebf43b088b7bcd11515533085ea1964`. Each probe checks
its result, surrounding guards, listed bytes, and completion at its terminal
loop. Compiler/VM tests use `CARGO_PROFILE_TEST_OPT_LEVEL=1` to accelerate
the Rust test hosts without changing Action compilation settings or assertions.
