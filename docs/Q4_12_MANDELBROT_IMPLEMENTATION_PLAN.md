# Signed Q4.12 and Oscar64 Mandelbrot

Status: all three slices complete and accepted, 2026-09-10.
Commit each tested implementation slice.

## Contract

Add `MATH.Q4_12` as ordinary embedded Action source, using signed INT raw
values divided by 4096 and existing LONGINT intermediates. Range is -8 through
7.999755859375; the sign is included in the four integer bits. Export One=4096,
Half=2048, Epsilon=1, MinValue=-32768, MaxValue=32767. FromInt, Trunc,
FromRatio, Mul, and Div follow the Q8.8 API: truncate toward zero, wrap only
after rescaling, and reuse non-returning Error(101) for division by zero.

Two explicit operations support the Mandelbrot recurrence: MulFloor returns
`floor(left*right/4096)` narrowed to INT, and SqrWide returns the exact raw
square as LONGCARD (Q8.24, with no rescaling). For example, Mul(-1,1)=0 but
MulFloor(-1,1)=-1; SqrWide(-32768)=1073741824. All intermediate products and
scaled INT numerators fit LONGINT. Preserve Q8.8 behavior. Add no types, IR
forms, ABI changes, or special compiler rules for the sample.

Port Oscar64's `samples/fractals/mbfixed.c` at the existing port revision
`8deb94c4d762bab3aa60c9565412691f01021bbb`. Retain its 160x100 coordinate grid,
32-iteration limit, pre-update escape test at raw squared radius 0x04000000,
separately rescaled squares, floor-rounded cross product, and update order.
Use coefficients 22937 and 25165 and offsets 10240 and 4915, preserving the
original constant truncation. Q8.8 Mul is valid for these nonnegative coordinate
products; the signed recurrence explicitly uses Q4.12 MulFloor. Keep the
unscaled squares wide until after the escape test.

The numerical module and Atari display share one implementation of the
recurrence. Replace C64 VIC/memory-map setup with Atari Graphics(31), retaining
the original eight pairs of 2-bit dither patterns and adapting the palette.
Map 100 logical rows to 192 display rows using the interval
`[floor(py*192/100), floor((py+1)*192/100))`: 92 rows use both original dither
halves, eight use only the upper half. Preserve every logical row and column.
Display adaptation is explicit; do not claim a byte-identical C64 image.

## Slices

1. **Q4.12 library and arithmetic tests.** Add the module, API guide, public
   compilation/constant checks and host-i64 VM oracles in all six mode/runtime
   lanes. Exercise extrema, half-step remainders, wrapping, both rounding
   policies, full-width squares, repeated calls, and both division fault APIs.
   Reuse the Q8.8 VM setup/guard/fault harness without changing its oracles.
   Register new named-module runtime fixtures in the existing corpus ledger.
2. **Mandelbrot numerical port.** Put the shared kernel in the sample project,
   add an Oscar64 runtime fixture and dedicated VM target with independently
   computed coordinate and iteration expectations. Cover boundary/inside/outside
   points, every coordinate axis position, deterministic random pixels, and
   inputs where floor versus truncation changes the result, in all six lanes.
   Record pinned provenance, license, adaptations, and actual coverage.
3. **Atari graphics sample and acceptance.** Add the runnable sample using the
   shared kernel, build-catalog entries and documentation. Verify representative
   rendered rows and untouched bitmap regions in all six lanes; verify a full
   160x100 render in standalone MIR6502 against an independent packed-bitmap
   oracle. Update indexes and coverage records after the checks pass.

## Validation

Use the public compile_file path; pure numerical standalone runs load no ROMs.
Expected arithmetic uses host i64 with explicit floor/truncation and signed
16-bit wrapping, independent of compiler constant evaluation. Inspect complete
guarded outputs and actual error delivery, not just checksums or watchdogs.
Mandelbrot expectations must reproduce the pinned integer algorithm, including
the separate square shifts, rather than using floating point as the oracle.
The pinned VM records CIO graphics pixels rather than emulating ANTIC scanout
or OS screen-memory packing. Compare all observed 160x192 pixels after packing
them into a linear two-bit image; also check the palette, graphics channel
parameters, untouched regions, and pixels outside the viewport. This validates
the rendered pixel image through the existing graphics-call model.

Run relevant root compilation tests and focused locked VM tests per slice.
Before final acceptance run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
cargo check --all-targets
```

From tools/vm-runtime-tests, run `cargo test --locked --no-fail-fast`.
If a compiler repair is needed, keep it general, retain the exposing expression,
and run required compiler/NIR checks before its separate commit. Existing IR
snapshots should remain unchanged; catalog additions are fixture coverage
updates, not IR contract changes. Report runtime/coverage limits precisely.

## Progress

Slice 1 is complete. The public compiler checks and corpus ledger pass; the
two Q4.12 VM tests pass 3,774 executions (617 arithmetic pairs across six
lanes plus 72 faults). The Q8.8 harness setup was shared without changing its
oracles; all five existing tests and 3,810 executions still pass. The new
library uses ordinary signed LONGINT operations, including explicit correction
for floor rounding and complete four-byte square results. No compiler changes
or IR snapshot changes were needed.

Slice 2 is complete. The shared sample-project kernel preserves the original
integer recurrence, coefficients, radius check, and explicit floor rounding.
Its numerical test passes 2,424 VM executions (404 pixel cases per lane, also
checking repeated calls and direct raw coordinates); the probe adds six
documented-output runs. The corpus and sample-role catalogs pass. The host
model counts 3,167 capped pixels, 151,649 updates, and 180 floor/truncation
differences in the full viewport; these are integer-model facts, not claims
of executing an Oscar64 binary. Pinned provenance and GPL attribution are
recorded with the shared source and the fixture.

Slice 3 implements the Atari Graphics(31) sample using the same kernel and
the original dither patterns at the documented row intervals. Both samples
build in every advertised mode/runtime configuration. The dedicated VM target
passes four tests / 2,437 executions: the numerical and printing cases above,
selected logical rows 0, 24, 50, 75, and 99 in all six lanes, and the complete
160x100 numerical viewport rendered into 160x192 pixels in standalone MIR6502.
Observed pixels, untouched regions, palette registers, and graphics channel
parameters match the independent host oracle. The public CLI also builds the
uninstrumented standalone MIR6502 sample (3,198-byte XEX). The sample README
records the graphics-model boundary, artifact format, and measured test costs.

Final acceptance passes: 3,077 root tests, with 22 existing ignored tests;
279 locked VM tests, none ignored; the dedicated NIR snapshot check; all 49
NIR sweep fixtures; and cargo check --all-targets. The full sample build
catalog passes. The source-only runtime corpus retains 345 normal entries
plus 12 expected named-module semantic rejections, covered through the public
module-aware compilation path. NIR snapshots and compiler code are unchanged.

## Sources

- [Pinned Mandelbrot source](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/samples/fractals/mbfixed.c)
- [Pinned arithmetic helpers](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/include/fixmath.c)
- [Pinned arithmetic tests](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/autotest/fixmathtest.c)
- [Oscar64 GPL-3.0 license](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/LICENSE)
- [Q8.8 guide](FIXED_POINT_Q8_8.md)

Retain GPL-3.0 attribution in adapted kernel/display sources. Native fixed
point types, additional formats, specialized arithmetic kernels, and performance
optimization are separate follow-ups.
