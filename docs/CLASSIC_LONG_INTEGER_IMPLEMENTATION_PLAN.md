# LONGINT/LONGCARD in the classic backend

Status: implementation started, 2026-09-10.

## Contract

Compatibility and Optimized classic must implement the same signed LONGINT
and unsigned LONGCARD semantics as MIR6502, with both ActionCart and Standalone.
INT/CARD remain 16-bit. Operand types determine computation width; assignment
and explicit casts convert the completed value. Narrow-before-widen, logical
right shifts, wrapping arithmetic, signed division/remainder and Error(101)
retain their existing language contracts.

SemIR supplies resolved integer types and operations to the classic projection.
Classic owns byte expansion, temporary storage, helper calls and ABI placement.
Reuse compiler-owned integer6502 kernels. Do not route a classic request through
MIR6502 or add executable string summaries to NIR. Four-byte scalar arguments
and results use the existing Action ABI described in
[the wide-integer ABI contract](LONG_INTEGER_INTEGRATION_AND_MIR6502_PLAN.md#mir6502-legalization-and-abi).

Capture source values and effective addresses once, preserving call order,
volatile accesses, pointer-cell overlap, selected IF/CASE arms and aggregate
payload widths. Calls and arithmetic helpers clobber their declared scratch;
live values must survive nested calls and outgoing argument placement at $A3.

## Slices

1. **Typed computation and storage.** Carry resolved wide expression facts
   through the classic projection. Implement four-byte constants, loads,
   stores, integer conversions, unary/binary arithmetic and comparisons with
   conservative capture. Extend internal result-byte accounting. Keep the
   public capability guard until the complete surface passes execution tests.
2. **Calls and control flow.** Complete direct/indirect argument staging,
   returns, compound assignments, wide loop limits/steps, CASE and selection
   values. Check pointer/index/record/variant consumers and faults. Remove the
   classic rejection only when these paths are supported; preserve specific
   diagnostics for independently unsupported conversions or language features.
3. **Integration and acceptance.** Expand existing wide-integer execution
   oracles, LONG library/sample checks and the Oscar64 LONGCARD rotation port
   to the supported classic profiles. Replace obsolete backend-rejection
   expectations and update capability documentation and coverage counts.

Commit each implementation slice after its focused checks. Compiler repairs
must be general; retain regression inputs and independent expected results.

## Validation

Use externally supplied values with nonzero upper words, carry/borrow across
all four bytes, signed/unsigned limits, counts at and above 32, narrow operands,
changed arguments, nested direct/indirect calls, overlapping pointer cells,
odd/page-crossing arrays, volatile captures and guarded destinations. Validate
wide labels differing only above bit 15 and loops at numeric boundaries.

Run focused classic execution tests in both profiles and runtimes, then root
`cargo test nir_fixtures_match_snapshots`,
`cargo run --bin actionc-nir-sweep -- fixtures/nir`, `cargo test`, and
`cargo check --all-targets`. Run the focused and full locked VM suites from
`tools/vm-runtime-tests`. Explain any changed IR snapshots; unchanged source
fixtures must retain their observable behavior.
