# MIR68K code quality milestone

Status: all four implementation slices complete, with local validation below.

This milestone adds native DCT and ADPCM acceptance, records a reproducible
code-generation baseline, reduces temporary stack traffic within basic blocks,
and improves measured instruction selection. Each major slice is committed
separately. Public CLI output and Amiga startup remain the following milestone.

Native benchmark acceptance is implemented. It exposed missing native array
descriptor loads and named record-pointer dereferences, now explicit in NIR.
Six native snapshots change for that contract fix; the Atari representation is
preserved. DCT passes 181 cases per mode, the decoder 2,713 checkpoints and the
encoder 1,561 checkpoints. Reference generator checks pass unchanged.

## Delivery and acceptance

1. Run every existing jfdctint, adpcm_dec and adpcm_enc C-reference vector in
   raw and optimized native modes. Decode reference integers before serializing
   them in target byte order. Address state through compiler symbols and compare
   intermediate states as well as final results. Exercise LF and CRLF through
   actual source instrumentation and compilation.
2. Record code bytes, executed instructions and maximum individual frame size
   for the native benchmarks. Measure their ordinary entry points, excluding
   test instrumentation, with a repeatable command and explicit compiler modes.
3. Retain temporary values in registers within individual physical blocks.
   Track actual instruction clobbers and widths; preserve flags, calls, volatile
   accesses, scratch storage, and control-flow boundaries. Retain a conservative
   materialization path for differential execution and measurement.
4. Improve instruction selection where baseline listings show avoidable cost.
   Qualify new original-MC68000 encodings independently, preserve flags and
   full-width semantics, and measure the result before expanding the scope.

Backend changes run focused compiler checks and the complete native suite.
Shared compiler contracts require the checks in AGENTS.md. Benchmark adapters
alone do not require repeating unaffected 6502 tests; the original algorithms
and their existing adapters remain shared validation inputs. Full CI continues
to cover all workspaces on Linux, Windows and macOS.

Register allocation across blocks, multidimensional arrays and platform output
formats remain separate work. NIR continues to own typed computation and
effects; instruction selection and physical register decisions belong to MIR68K.

## Baseline

The [baseline CSV](mir68k-code-quality-baseline.csv) records the corrected
compiler at `aefccef`, with conservative stack homes. The reproducible command
is documented in the [runner README](../tools/vm68k-runtime-tests/README.md).
Optimized NIR examples before target optimization:

| Benchmark | Executable bytes | Executed instructions | Largest frame |
| --- | ---: | ---: | ---: |
| Matrix1 | 2,524 | 185,892 | 166 |
| SHA (`abc`) | 9,010 | 36,010 | 786 |
| DCT | 10,192 | 57,837 | 1,304 |
| ADPCM encoder | 17,742 | 2,851,208 | 802 |

The encoder includes its original integer sine initialization. These are
deterministic instruction counts, not host wall-clock timings or whole-machine
performance estimates. Frame measurements exclude dynamic call nesting.

## Temporary forwarding

Block-local forwarding and removal of unread private temporary stores reduce
optimized Matrix1 from 185,892 to 170,264 instructions and DCT from 10,192 to
9,034 executable bytes. All benchmark results remain unchanged; frame sizes
are unchanged. The differential regression checks branch/call composition,
mixed widths, ABI completion and the exact volatile access trace. The
`--no-codegen-opt` measurement reproduces the baseline CSV byte for byte.

## Instruction selection

MOVEQ, immediate arithmetic and comparisons, constant shifts, and simpler
constant/power-of-two index calculations reduce optimized SHA to 6,124 bytes
and 28,620 instructions. Independent literal MC68000 programs qualify the new
instruction encodings and flags. A host oracle checks all five integer types,
constant shift boundaries, and immediate arithmetic with target selection on
and off. The conservative option still reproduces the original baseline.

## Branch relaxation and final measurements

Checked word-displacement Bcc/BRA instructions replace nearby absolute jumps.
Relaxation repeats as code shrinks; distant destinations retain their original
forms. Independent encodings cover displacement limits and the PC base, and
compiled loops exercise both branch arms and backedges at multiple origins.

The [final CSV](mir68k-code-quality-current.csv) records all seven benchmarks
with raw and optimized NIR. Optimized NIR comparisons, with all target
optimizations enabled:

| Benchmark | Code bytes before → after | Instructions before → after | Largest frame |
| --- | ---: | ---: | ---: |
| Insertion sort | 2,990 → 2,242 | 12,700 → 10,617 | 182 |
| Matrix1 | 2,524 → 1,790 | 185,892 → 162,876 | 166 |
| Binary search | 1,720 → 1,222 | 11,557 → 9,156 | 126 |
| SHA (`abc`) | 9,010 → 5,926 | 36,010 → 28,606 | 786 |
| DCT | 10,192 → 7,402 | 57,837 → 47,129 | 1,304 |
| ADPCM decoder | 13,850 → 10,258 | 26,447 → 22,924 | 794 |
| ADPCM encoder | 17,742 → 13,032 | 2,851,208 → 2,304,120 | 802 |

Frame reservations remain unchanged. Disabling target optimizations reproduces
the baseline CSV byte for byte. These counts describe the documented entry
inputs, rather than every reference-vector execution.

## Validation

The complete native r68k suite passes, including raw/optimized DSP reference
states, LF/CRLF instrumentation, arithmetic oracles, volatile traces, ABI
preservation and differential target-optimization checks. Focused MIR68K unit,
native ABI and native type checks pass. The NIR snapshots and 51-fixture sweep
pass; six native snapshots intentionally record the pointer contract bug fix.

The full local compiler suite passes except for a pre-existing untracked
`samples/vbxe/shared/lines.act` whose `SHARED.SCREEN` dependency is absent.
That unrelated sample and the user's other worktree changes are outside this
milestone.
