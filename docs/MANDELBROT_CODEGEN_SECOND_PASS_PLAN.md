# Mandelbrot code generation: second pass

Status: all four slices implemented, verified and committed separately.

Baseline: revision 7bea08b, standalone MIR6502. The 640-pixel grid costs
26,877,910 kernel cycles; full Atari and VBXE renders cost 1,464,315,551 and
2,908,153,031 cycles. Ordinary XEX sizes are 2,462 and 4,191 bytes.

## Slices and contracts

1. Narrow Mult32 loops: dispatch using captured multiplier magnitude; shift
   only the live multiplier bytes for byte/word paths. Retain full modulo-2^32
   results, wide fallback, scratch declarations, decimal handling and stack.
2. Direct signed widening and private-local cleanup: generate a sign mask
   without comparison/Boolean/subtraction staging; remove redundant private
   storage only with proven non-escape and use/liveness facts. Preserve calls,
   volatile/absolute memory and pointer aliasing. No sample-specific rules.
3. Four-bit word shifts: select efficient inline carry-linked byte operations
   for small constant counts, guided by code size and runtime cost. Preserve
   overlap/capture semantics and dynamic-shift fallback.
4. Wide addition/subtraction: preserve carry/borrow across all four bytes with
   an explicit MIR contract and verified materialization. Avoid reconstructing
   carry from comparisons; keep all lower-byte contributions to higher lanes.

SemIR/NIR retain language meaning and typed computation. Target sign handling,
shift/carry strategy and arithmetic helper selection belong to MIR6502. Any
private-local optimization must use verified stable storage identities and
conservative effects. No executable strings or SemIR lookbacks are introduced.
Coordinate caching and renderer changes remain outside this task.

## Validation

Each slice runs focused structural and pinned VM regressions, NIR snapshot
checks, the 49-fixture NIR sweep and cargo test. Preserve independent host
oracles, explicit completion, surrounding-memory guards and ABI checks.
After the final slice run all Mandelbrot tests (5,352 executions), compare both
complete images and palettes, re-profile the same 640-pixel grid, rebuild the
installed compiler and sample XEX files. Record actual sizes and cycle counts.

Use the VM harness from tools/vm-runtime-tests with --locked. Build artifacts
are disposable; this task does not add generated listings or XEX files to Git.

## Verified progress

Slice 1: byte/word multiplier loops retain only live right-hand lanes. Kernel
cycles fell to 24,295,785 (9.61% below baseline); VBXE XEX grew to 4,277 bytes
(+86). NIR snapshots, 49/49 sweep, full compiler suite, 17 wide-integer VM
tests, both Q4.12 tests and five Q8.8 tests passed. The expanded direct helper
regression passed 5,376 products with stack, scratch and decimal-mode checks.

Slice 2 contract: verified 32-bit integer scalar homes use the same exact
storage identity, read-before-definition, escape, call and liveness analysis
as byte/word homes. This enables forwarding and dead private-store removal;
initialized persistent homes and externally observable storage retain their
existing barriers. Target sign-mask selection does not change NIR casts.

Slice 2: direct sign masks and verified 32-bit storage forwarding reduce the
kernel to 21,900,337 cycles (18.52% below baseline), with a 3,970-byte VBXE XEX.
The square wrapper no longer stages through its dead local. NIR snapshots and
49/49 sweep are unchanged; compiler checks, 18 wide-integer VM tests and seven
fixed-point VM tests passed, including 3,072 widening executions across all
sign bytes, three compiler modes and both runtimes.

Slice 3: nibble shifts use three independent byte shifts plus OR; three-bit
shifts use the existing bounded carry expansion. Kernel cycles are 21,413,144
(20.33% below baseline), and VBXE XEX size is 4,014 bytes. All 19 wide-integer
VM tests passed, including 6,144 new small-shift executions; compiler tests,
NIR snapshots and 49/49 sweep passed. KALSCOPE's quality assertion intentionally
changes from one shift helper to zero; its existing size ceiling still passes.

Slice 4: wide add/subtract and negation use four captured byte lanes with
explicit carry/borrow dependencies, including discarded low results. The
focused regression covers 2,550 executions across boundaries, random operands,
call captures, compound assignments, negation and partial results. Compatibility
cases stage calls explicitly to respect that profile's expression restrictions.

## Final measurements

| Measurement | Baseline | After four slices | Reduction |
| --- | ---: | ---: | ---: |
| 640-pixel kernel cycles | 26,877,910 | 20,769,965 | 22.72% |
| Full Atari render cycles | 1,464,315,551 | 1,159,571,249 | 20.81% |
| Full VBXE render cycles | 2,908,153,031 | 2,298,447,085 | 20.97% |
| Atari standalone XEX bytes | 2,462 | 2,197 | 265 bytes |
| VBXE standalone XEX bytes | 4,191 | 3,926 | 265 bytes |

Measurements use the same viewport, palette and independent host oracle as
the baseline. All eight Mandelbrot VM tests (5,352 executions), 20 wide-integer
VM tests and seven fixed-point VM tests passed. All 17 saved bitmap/palette
artifacts match the baseline byte-for-byte. Release emission reproduces the
profiled executable. Multiplication remains 69.13% of sampled kernel cycles;
coordinate caching remains outside these slices.

Final compiler verification: 3,087 tests passed, with 22 existing ignored tests;
NIR snapshots and the 49/49 fixture sweep passed without snapshot changes.
