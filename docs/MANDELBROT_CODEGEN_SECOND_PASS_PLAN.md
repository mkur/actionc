# Mandelbrot code generation: second pass

Status: implementation in progress. Commit each verified slice.

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
