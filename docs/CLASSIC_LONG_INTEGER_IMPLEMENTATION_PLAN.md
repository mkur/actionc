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

1. **Typed computation and storage.** Use resolved storage types, casts and
   typed literals from the classic SemIR projection. Implement four-byte constants, loads,
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

## Classic legalization contract

The existing SemIR projection and shared ScalarType arithmetic contract supply
computation widths and signedness. Aggregate identity takes precedence over
byte extent: a four-byte record or union is not a LONG scalar. Every completed
integer subexpression is truncated to its own width and then extended according
to its own signedness before an enclosing computation or store converts it.

The classic wide evaluator uses $C4..$C7 for a captured value, $82..$85 and
$C0..$C3 for binary operands, and the existing compiler-owned 32-bit kernels.
Earlier operands are pushed before evaluating later operands. Direct and typed
indirect calls pack complete argument values on the stack before writing the
public ABI homes; four-byte results are captured before outgoing arguments can
overwrite $A3. Assignment captures its effective destination address before
RHS effects. Loads capture the complete source before overwriting result bytes.
Helpers are linked once when used and report their Error dependency through
the existing runtime linker. These rules apply in both classic profiles.


Wide FOR direction and magnitude come from SemIR's resolved step control.
Classic emits four-byte limit comparisons and wrap guards at signed/unsigned
boundaries. Compounds capture the destination before RHS effects and read the
old value afterwards, applying the SemIR computation type before store
conversion. Condition-shaped AND/OR branches preserve guarded CASE evaluation.
Wide scalar and aggregate initializer writes use the typed static image;
array backing storage is bound after all arithmetic helper bodies, so buffers
cannot overlap executable code.


## Progress

Slice 1 is complete. Both classic profiles execute four-byte computation,
conversion, storage and nested call results in both runtimes. The public guard
remains until control-flow integration is complete. Validation: 3,073 compiler
tests passed (22 existing ignored), 49 NIR fixtures passed, and 62 focused VM
tests passed, including 592 host-fed classic LONG cases. No IR snapshots changed.

Slice 2 is complete. The compiler facade and inspection CLI enable classic LONG
execution through the typed projection. Calls, compounds, signed/unsigned FOR
limits and steps, IF/CASE values, variant payloads, initializer images and
volatile captures preserve full width. The array/helper overlap found by the
cartridge input oracle is fixed. The refreshed compiler suite passes all 3,073
tests (22 existing ignored), all 49 NIR fixtures pass, and all-targets checking
passes. Focused wide execution and I/O oracles pass; no IR snapshots changed.
