# Mandelbrot arithmetic code generation

Status: implementation in progress. Commit each tested slice.

## Contract and baseline

Preserve all LONGINT/LONGCARD arithmetic, Q4.12 rounding and wrapping, and the
complete Atari/VBXE images. Keep the original Oscar64 conformance API unchanged.
Use general target lowering and compiler-owned arithmetic helpers; no renderer
names or source-text patterns belong in compiler decisions. Explicit loads,
volatile operations and ordering barriers retain their contracts.

The baseline standalone MIR6502 renders take 3,641,588,504 VM cycles at 160x192
and 7,263,913,365 at 320x192. A 640-pixel kernel profile attributes 56.26% to
Mult32, 22.99% to DivI32 and 6.08% to RShift32. X-coordinate recomputation costs
about 294 million cycles per VBXE frame. These measurements guide priorities;
they are not correctness or golden performance assertions.

## Slices

1. **Constant wide shifts and Q4.12 floor rescaling.** Legalize constant 32-bit
   logical shifts as word shifts/projections and bitwise combinations. Preserve
   zero counts and the zero result for counts at least 32; retain helpers for
   dynamic counts. Existing materialization owns byte selection and dead lanes.
   Express MulFloor's wrapped INT result as product bits 12..27, eliminating
   signed division and its floor correction without changing any public API.
2. **Narrow multiplication paths.** Improve the shared compiler-owned Mult32
   kernel so widened narrow operands do not pay for 32 rounds. Preserve the
   full modulo-2^32 result for arbitrary signed/unsigned bit patterns, existing
   scratch/clobber declarations, decimal-mode handling and balanced stack.
   Select paths from captured input bits, without depending on alias facts or
   consulting SemIR. Both Atari backends benefit.
3. **Aligned wide comparisons and final acceptance.** Simplify comparisons
   against word-aligned constants using the significant word, retaining signed
   high-word ordering. Validate boundary conditions and all comparison forms.
   Rebuild the installed compiler and both XEX files; repeat full image oracles
   and the same kernel profile, and record actual size/cycle changes.

Coordinate caching changes renderer data flow rather than code generation and
is a separate follow-up. General constant signed division and new source types
are not required for these slices.

## Checks

For compiler changes run NIR fixture snapshots, the NIR sweep and cargo test.
Use focused generated-code regressions and pinned VM tests from
tools/vm-runtime-tests with --locked: constant/dynamic wide shifts, multiplication
boundaries/random operands, Q4.12, and Mandelbrot. Verify whole guarded output
regions and explicit completion. Final acceptance includes all 5,352 Mandelbrot
VM executions, both complete images and the sample build catalog. Explain any
intentional snapshot/baseline change; do not weaken semantic checks.

## Verified progress

Slice 1 is complete. Constant wide shifts avoid the 32-bit helper; MulFloor
extracts the wrapped signed result from product bits 12..27. The same 640-pixel
kernel profile drops from 72,669,353 to 51,588,550 cycles (29.01%), with identical
iteration counts. The standalone VBXE XEX shrinks from 4,556 to 4,234 bytes.

Validation: 3,081 root tests passed (22 existing ignored), NIR snapshots and all
49 sweep fixtures passed, all wide-integer and Q4.12 VM tests passed, and the six
Mandelbrot tests excluding the two full renders passed (5,350 VM executions).
The new constant-shift matrix covers 1,056 executions across all modes/runtimes,
including full/narrow results, overshifts and preserved input calls. No snapshot
contract changed. Full images are checked again after the remaining slices.
