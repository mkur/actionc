# MIR65816 direct BYTE arithmetic operands

## Objective and baseline

Implement priority 5 from the frozen Exec output audit in two independent
slices: immediate right operands, then captured private right operands. Replace
the generic `RIGHT` scratch round trip for materialized BYTE Add/Sub/AND/OR/XOR
with the existing native immediate or stack-relative instruction.

Baseline compiler: `13220e50` (implementation at `f5da2404`). Frozen Exec:
`154bcf5a45605aeee57de5420ec4604e4ff7b30c`, 166 loaded source files and 899
routines. Compiler code sizes are 324,615 bytes optimized unchecked, 437,062
optimized guarded and 372,460 raw unchecked; initialized data is 1,385 bytes.
Platform assembly and hosted Exec boot qualification are outside these totals.

All 236 original candidates still contain the scratch round trip. Their current
spans occupy 2,685 bytes; six spans already lost mode changes during earlier work.

| Slice | Sites | Modeled saving |
| --- | ---: | ---: |
| Immediate RHS | 143 | 572 bytes |
| Captured private RHS | 93 | 372 bytes |
| Total | 236 | 944 bytes |

This is a four-byte-per-site instruction model, not a measured promise. Preserve
the frozen source/layout hashes and remeasure each implementation slice.

## Contract and ownership

This is MIR65816 instruction selection, using existing typed operations, widths
and allocated homes. Do not change NIR, source semantics, allocation, ABI padding,
stack bounds or forwarding windows. Keep earlier accumulator-expression,
top-bit, fusion, DP/X and other selector ownership intact.

Admit only one-byte results of Add/Sub/AND/OR/XOR, with a complete writable stack
result and exact BYTE immediate/private operands supported by checked operand
selection. Slice one requires a U8 right operand; slice two adds captured stack
and current parameter homes. Refuse unsupported widths, DP homes, symbolic
operands and unhandled borrowed bindings before emitting anything. Check both
sources and the result, including transient stack reach, before changing tracked
state. Never write through a read-only operand binding.

Reuse existing typed byte loads and arithmetic selection. Request A8 explicitly;
preserve hidden B, X/Y and the native environment. Add establishes clear carry,
Sub establishes set carry, and logical operations preserve C/V. Operand roles
remain unchanged, especially for subtraction. Read the captured left operand,
consume the direct right operand and store exactly one result byte. Equal source
and destination homes are safe because both inputs precede the final store.
Private operand read order may differ from generic scratch staging; source loads,
volatile/absolute accesses, calls and machine blocks keep their original order.

Use conservative barriers and retain typed effects/replay. Do not create new
retained-A or flag witnesses, omit captures, fold external loads into arithmetic,
or extend residence across stores/calls. Wider operations, shifts and unsupported
forms retain established paths. No Exec-specific matching is permitted.

## Implementation commits

1. Commit this plan before compiler changes.
2. Select immediate RHS BYTE arithmetic, sharing existing checked instruction
   forms. Add focused encoding/preflight and runtime tests, update the emission
   contract, measure the frozen optimized unchecked build and commit the slice.
3. Add exact captured private RHS operands. Cover operand roles, identical homes,
   mutable parameter values, alias/call capture behavior and refusal boundaries.
   Measure and commit separately.
4. Run final backend qualification, rebuild all three frozen Exec profiles and
   commit compact results/provenance and completion notes.

## Validation

Unit tests cover all five operations, both entry accumulator widths, exact byte
encodings, stack endpoint/transient reach, unsupported operands and atomic
refusal. Native tests use independent ca65 templates, poisoned hidden B, complete
register/flag comparisons, exhaustive byte arithmetic where practical, exact
private accesses/canaries, unchanged external capture traces and replay equality.
Cover raw/optimized builds, guards on/off, two o65 placements and IRQ/NMI after
nested calls. Exercise LF and CRLF through changed host-text preparation paths.

Use affected targets while developing. After the final compiler change run the
MIR65816 library tests, its root integration targets, and the full native release
runner: `python3 -B tools/native65816-runtime-tests/qualify.py --release --no-fail-fast`.
Keep qualified compiler/runtime/test inputs stable throughout each invocation;
exclude no baseline failures. Existing opt-in external comparisons may stay
ignored. Shared-contract checks are needed only if the implementation changes
shared frontend/NIR contracts.

Use nonincremental builds with reduced debug information and monitor disk space.
Keep unrelated working-tree changes intact. Store bulky measurements under
`target/`; commit compact results under `docs/benchmarks/65816-byte-arithmetic/`.
Compare all 166 source hashes, layouts, signatures, arguments/results, frames,
homes, calls, stack bounds, initialized data and zero-fill against baseline.
Distinguish actual total code savings, candidate span savings and modeled savings.

## Immediate slice

Implemented direct U8 RHS selection using the existing checked BYTE classifier
and typed expression arithmetic. The result keeps its allocated stack home.
All 143 audited immediate spans shrink by four bytes (572 bytes total); the 93
private-RHS spans are unchanged. Optimized unchecked Exec is 324,037 bytes,
saving 578 bytes including neighboring code changes. Source/layout hashes and
all routine/ABI/frame/data facts match baseline. See
[results.json](benchmarks/65816-byte-arithmetic/results.json).

Two selector unit tests and three new native tests pass, as do the existing
BYTE-consumer test and four top-bit branch tests. Native checks include all 256
left values against five immediate boundaries for each operation, independent
ca65/register/flag checks, exact private traffic, guards, LF/CRLF, capture order,
two o65 placements and IRQ/NMI after earlier calls. Boundary constants are
substituted into verified MIR so source constant folding cannot remove the
selector cases being tested.

## Private-operand slice

The same selection now consumes exact BYTE stack captures and current parameter
homes directly. Both inputs precede the store, including identical homes;
mutable parameters use their authoritative frame location. All 93 audited
private-RHS sites save four bytes each. Optimized unchecked Exec is 323,664 bytes,
373 below the immediate slice and 951 below baseline. All 236 audited sites are
selected: 944 bytes saved in their spans plus seven bytes in neighboring code.

Five unit tests and fourteen focused native tests pass (six new arithmetic tests,
the existing arithmetic target and seven storage-demand tests). Private arithmetic
checks all 65,536 operand pairs for all five operations, with both incoming C/V
settings, host flag/value oracles and ca65 checks at boundary values. Other mode
and guard combinations check every left value against right-hand boundaries.
Tests also cover mutable parameters, local captures, exact external access order,
relocation and IRQ/NMI after calls. Full backend qualification is the final step.
