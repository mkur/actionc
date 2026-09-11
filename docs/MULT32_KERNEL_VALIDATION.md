# Mult32 narrow multiplication kernel

The compiler-owned `Mult32` helper now uses a rotating 16x8 or 16x16 partial
product when the normalized multiplier fits a word and the multiplicand's
upper word is zero or $FFFF. Each set multiplier bit adds two bytes, with the
highest partial-product byte held in A. Signed upper-word correction preserves
the complete product modulo 2^32, including -65536. Wider values retain the
existing general loops.

This is runtime strategy shared by the classic and MIR6502 backends. It adds
no source-language special cases, lookup tables, scratch locations or ABI
changes. The [MIR6502 contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md) specifies
the dispatch proof and signed correction. The implementation is independent
of Mad Pascal's arithmetic runtime.

## Matched measurements

Measured 2026-09-11 against `7956f58`, with the same standalone MIR6502 source,
INLINE requests and origin $2000. Both versions expand the three Q4.12 calls.
The pinned Actionc VM executes the actual linked routines, with every listing
instruction checked against the XEX and each result checked against an
independent integer oracle. Timings exclude coordinate mapping, plotting,
DMA and interrupts; they describe recurrence CPU cycles, not whole-frame
wall-clock speed.

| Measure | Before | After |
| --- | ---: | ---: |
| 640 points, 6,167 updates | 20,276,676 cycles | 16,032,406 cycles |
| Original 16,000 points, 151,649 updates | 520,414,063 cycles | 409,321,262 cycles |
| Multiply work, 640 points | 14,357,820 cycles | 10,113,550 cycles |
| Multiply work, 16,000 points | 374,745,561 cycles | 263,652,760 cycles |
| Linked Mult32 routine | 229 bytes | 336 bytes |
| Atari XEX at $2000 | 2,426 bytes | 2,533 bytes |
| VBXE XEX at its default $3000 | 4,155 bytes | 4,262 bytes |

The recurrence uses 20.93% and 21.35% fewer cycles respectively. Multiplication
uses 29.56% and 29.64% fewer cycles. Helper call counts remain 19,517 and
480,613, and measured work outside the helper is identical. The space cost
is 107 bytes. Both Atari and VBXE still accept all three INLINE expansions.
Profiling the delivered VBXE binary at $3000 over the same original grid gives
the identical before/after cycle totals and escape counts.

A separate direct-helper measurement uses 4,096 deterministic input pairs per
class, including SED, JSR and RTS. Every result is checked against host
wrapping multiplication. This exposes the dispatch cost outside Mandelbrot:

| Input class | Before cycles | After cycles |
| --- | ---: | ---: |
| Zero multiplier | 180,224 | 180,224 |
| Unsigned byte pair | 1,956,847 | 1,337,907 |
| Unsigned word pair | 3,965,502 | 2,647,221 |
| Signed word pair | 4,020,946 | 2,803,433 |
| General 32-bit left, byte right | 1,881,088 | 1,962,368 |
| General 32-bit left, word right | 3,893,332 | 3,975,252 |
| General 32-bit left, 24-bit right | 6,716,872 | 6,713,112 |
| General 32-bit pair | 8,929,437 | 8,929,408 |

Failed narrow-left checks add up to 20 cycles per call in these layouts,
about 4.3% for the wide-by-byte workload and 2.1% for wide-by-word. Tiny
differences in the last two rows reflect width dispatch and code layout;
their multiplication loops retain the previous algorithm. These are measured
input classes, not universal timing bounds for all addresses and values.

## Regression coverage

The direct linked-helper test checks all 65,536 byte pairs. For every one of
65,536 word patterns it checks three unsigned products and the corresponding
three signed products: squares, deterministic partners and a dense partner.
That is 458,752 executions, with result, scratch, decimal-mode and stack checks
on every call. These sweeps do not exhaust all word-pair combinations. Cycle
ceilings prevent a return to the former four-byte narrow loop.

Additional boundary and random tests cover arbitrary 32-bit products, all
dispatch widths, unsigned high-bit inputs, MIN, -65536 and nearby values,
and upper words that must fail the narrow check. Language-level tests exercise
both signed and unsigned multiplication in all three compiler modes and both
runtimes. The separate exhaustive Q4.12 audit checks every signed square and
deterministic floor-product pairs, with inlining disabled, costed and explicitly
expanded.

The larger helper also exposed an unsafe inliner placement between opaque
runtime entries connected by literal relative branches. Placement alternatives
now stop before the first machine-block routine, preserving the runtime tail's
adjacency. A focused regression protects that boundary; the existing six-mode
partial-row graphics test exercises the affected startup path.

Generated before/after XEX files, listings, profiles and logs are under ignored
`build/multiply-kernel/`. The recurrence profiler is
`build/oscar64-mandelbrot/action-profile.rs`; the direct helper profiler is
`build/multiply-kernel/multiply-profile.rs`. Both use Actionc VM revision
`7ec0cc454ebf43b088b7bcd11515533085ea1964`.

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test
cargo test --lib q4_exhaustive -- --ignored
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked
```

The validation runs use `CARGO_PROFILE_TEST_OPT_LEVEL=1` to accelerate host
compiler and VM execution while preserving test-profile assertions. This
does not change the Action compilation mode or generated arithmetic.

Final checks pass: 3,110 compiler tests (24 opt-in tests ignored), 294 VM tests,
NIR snapshots, the 51-fixture NIR sweep and the 167-fixture MIR6502 sweep.
The VM checks include complete 160x192 Atari and 320x192 VBXE images and
palettes against independent host oracles. The exhaustive Q4.12 audit also
passes explicitly. Existing fixture snapshots are unchanged.
