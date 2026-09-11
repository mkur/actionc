# Oscar64 Mandelbrot code generation comparison

Measured 2026-09-11 against Actionc `b9d74f9` (standalone MIR6502) and Oscar64
1.32.273, upstream revision `f38a1f20acd0280ed65588cad5d71fcca52fa38d`, built
with the upstream makefile on macOS. Target compilation uses `-O3 -g`.
The sample's own makefile selects `-O3`. Native 6502 code is Oscar64's default.

This records the baseline before Q4.12 wrapper inlining and the rotating
multiplication kernel. See [INLINE validation](INLINE_Q4_12_VALIDATION.md) and
[Mult32 kernel validation](MULT32_KERNEL_VALIDATION.md) for subsequent results.

Oscar64 produces smaller and faster arithmetic code in the validated comparison:
22.24% fewer CPU cycles for exactly the same coordinates and iteration counts.
The unmodified upstream sample also exposes a correctness problem, so its timings
are excluded from that comparison.

The generated [C64 binaries, listings and measurement sources](../build/oscar64-mandelbrot/)
are in the ignored build directory. The original C64 program is
[mbfixed-original.prg](../build/oscar64-mandelbrot/mbfixed-original.prg).
The verified C64 program using ordinary LONG arithmetic is
[mbfixed-generic.prg](../build/oscar64-mandelbrot/mbfixed-generic.prg).
These are C64 PRG files, not Atari XEX files.

## Workload normalization

| Program | Independently calculated points | Display |
| --- | ---: | --- |
| Original Oscar64 sample | 160 × 100 = 16,000 | 160 × 200, two dither rows per point |
| Actionc Atari sample | 160 × 192 = 30,720 | Every displayed pixel calculated |
| Actionc VBXE sample | 320 × 192 = 61,440 | Every displayed pixel calculated |

The Actionc images therefore calculate 1.92 and 3.84 times as many points.
Dividing whole-frame time by these factors would still mix different iteration
counts, coordinate mappings, graphics runtimes and plotting strategies.

The primary measurement runs both compiled kernels at the original 16,000
coordinates, using the original truncated coefficients 22937 and 25165 and the
same integer oracle. Both perform exactly 151,649 recurrence updates. Coordinate
calculation and plotting are excluded. Actionc executes its real `Iterate`
routine, including its parameter capture, Q4.12 wrapper calls and return.
Oscar64 executes the real inlined iteration region from its compiled program.
Inlining and ABI differences are therefore part of the measured code quality.

| Validated kernel workload | Actionc MIR6502 | Oscar64 LONG control |
| --- | ---: | ---: |
| Original 16,000 coordinates | 532,548,330 cycles | 414,134,362 cycles |
| Average per calculated point | 33,284.27 cycles | 25,883.40 cycles |
| Previous 640-point grid, 6,167 updates | 20,769,965 cycles | 15,915,243 cycles |
| Arithmetic code and linked helpers | 899 bytes | 420 bytes |

Oscar64 uses 22.24% fewer cycles on the complete matched workload; equivalently,
Actionc needs 28.59% more cycles. The 640-point grid gives a similar 23.37%
reduction. The code-size comparison includes all arithmetic helper paths, even
paths the grid does not exercise, but excludes coordinates, rendering, storage
data and startup. Actionc's 899 bytes comprise `Iterate` (427), `MulFloor` (181),
`SqrWide` (62), and `Mult32` (229). Oscar64's 420 comprise the two inlined loop
regions (258) and `mul32`, its proxy and `mul32by8` (162).

Whole files are 1,041 bytes for the unmodified Oscar64 PRG, 1,012 for the verified
LONG control, 2,197 for Actionc Atari and 3,926 for Actionc VBXE. These totals
include different graphics/runtime support and are not a pure compiler comparison.

## Why the LONG control is necessary

The [upstream sample](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/samples/fractals/mbfixed.c)
uses the [fixed-point library](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/include/fixmath.c).
Its unsigned square multiplication, signed coordinate multiplication and Q4.12
cross multiplication use handwritten 6502 assembly. In particular, `lmul4f12s`
computes the required scaled result directly. Actionc's Q4.12 library uses
ordinary LONGINT intermediates and a general 32-bit multiplication helper.

`mbfixed-generic.c` replaces the three fixed-point entry points with inline C
expressions using `(long)x * (long)y`, then shifts/casts the result. It preserves
the original coordinates, separate square shifts, floor-rounded cross product,
32-step cap, pre-update radius test, palette and rendering loop. This control
matches Actionc's arithmetic formulation more closely and passes every original
coordinate and every one of the 8,000 expected bitmap bytes.

## Remaining Actionc opportunities

1. **Reduce wrapper and storage traffic.** On the 640-point grid Actionc spends
   2,908,672 cycles inside the Q4.12 wrappers, excluding `Mult32`, and another
   3,503,473 in `Iterate`. Oscar64's non-multiply loop code costs 3,116,855 cycles.
   The difference outside multiplication is 3,295,290 cycles, about 68% of the
   total gap. This does not isolate inlining from register allocation: both
   wrapper calls and their argument/result/local transfers contribute.

2. **Place hot arithmetic state in zero page.** Oscar64 keeps `x`, `y`, the
   square intermediates, coordinates and iteration counter in zero-page homes.
   Actionc repeatedly accesses absolute parameter/local homes and spills. The
   Actionc listing at `$3303–$333A` shows square results moving through return
   slots into absolute local storage. Private-home placement and better return
   forwarding are useful general compiler improvements.

3. **Improve general multiplication further, with measured priorities.** The
   valid C control spends 12,798,388 grid cycles in its multiply helpers and
   proxy, versus Actionc's 14,357,820. That accounts for the remaining 32% of the
   measured gap. A general 16×16→32 strategy, selected from verified operand
   facts, remains attractive. The original assembly library demonstrates a
   separate opportunity for specialized fixed-point helpers.

4. **Fuse comparisons directly into branches.** Both compilers now use an
   efficient four-byte carry chain and compare the radius's top byte with 4.
   Oscar64 branches immediately. Actionc at `$3356–$3363` still materializes a
   Boolean with `LDA #0/#1`, then compares it with zero and branches again.

5. **Reduce staging around truncated wide shifts.** Oscar64 uses compact
   three-byte shift loops for the separately shifted squares. Actionc's inline
   nibble shifts avoid helper calls but still create more staging and combining
   instructions, including `LDA #0; ORA ...` sequences at `$33B0` and `$33F2`.
   Fusing a wide shift with its narrow consumer is more promising than merely
   unrolling additional shift counts.

The new Actionc carry chains and significant-byte radius test compare well.
The main remaining issue is getting values to and from arithmetic with less code
and fewer memory accesses.

## Correctness issue in the unmodified Oscar64 binary

An untouched full run of `mbfixed-original.prg` computes 678 escape counts that
*differ* from the independent oracle, out of 16,000 points. Its final bitmap
differs in 669 of 8,000 bytes. For example, point `(110,1)` should escape at
iteration 4, but returns 3. This is not a resolution comparison: these checks
use the original program's own coordinates, which are independently verified.

The original `-O3` listing saves bytes 0, 2 and 3 of the first square after the
call at `$095B`, but omits byte 1. Later code reads its expected home at `$43`
for both the radius carry chain and the shifted square. A diagnostic emulator
hook copying `$1C` (returned square byte 1) into `$43` at `$095E` eliminates all
678 disagreements. This diagnostic is an untimed memory intervention and is
not used for performance scoring or included in the delivered PRGs.

The pinned source/compiler revision `8deb94c4d762bab3aa60c9565412691f01021bbb`
produces an identical original PRG to current revision `f38a1f20...`; the sample
and fixed-point library sources are unchanged between them. The generated `-O1`
and `-O2` listings also omit the same byte store; those alternatives are not
presented as validated workarounds.

Both original and LONG-control bitmaps were independently reproduced with
Oscar64's own emulator and match the corresponding pinned Actionc-VM runs
byte-for-byte. That second emulator check replaces only the first post-render
instruction with `RTS` to bypass keyboard input and display restoration; the
rendering code executes unchanged. The LONG control also matches the independent
host bitmap exactly.

## Reproduction and artifacts

The artifact directory contains the source variants, PRGs, assembly, maps,
SHA-256 manifest, raw profiles, independent oracle bitmaps, Rust profiling
sources, and a small adapter for Oscar64's own emulator. No compiler source or
VM test-suite files were changed for this analysis.

Build Oscar64 from the recorded revision with:

```sh
make -f make/makefile compiler -j8
bin/oscar64 -O3 -g -o=/path/to/mbfixed-original.prg /path/to/mbfixed-original.c
bin/oscar64 -O3 -g -o=/path/to/mbfixed-generic.prg /path/to/mbfixed-generic.c
```

Run `build/oscar64-mandelbrot/recheck.sh` from this checkout to rebuild the
measurement tools and replay the saved binaries. The artifact manifest detects
changed inputs; profiler entry/exit addresses refer to those exact binaries.

Cycle measurements use the same pinned 6502 core, Actionc VM revision
`7ec0cc454ebf43b088b7bcd11515533085ea1964`, for both compilers. They measure CPU
instructions without VIC/ANTIC DMA or interrupt contention, and are not measured
wall-clock times on C64/Atari hardware. Cross-emulator validation concerns bitmap
correctness; performance figures consistently use the pinned core.
