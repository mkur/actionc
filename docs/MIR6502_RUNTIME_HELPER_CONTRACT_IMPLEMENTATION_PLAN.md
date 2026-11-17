# MIR6502 runtime and standard library contract implementation plan

Status: in progress, 2026-11-17. Slice 1 complete.

Implement the [contract design](MIR6502_RUNTIME_HELPER_CONTRACT_DESIGN.md) in
the slices below, committing each slice after its focused checks pass. Deliver
SArgs support first through the existing typed capture operation. Reuse that
work for arithmetic helpers and selected standard-library implementations.
Preserve unrelated working-tree changes.

## Outcome and scope

Allow existing entry analysis, callee summaries and ZP residence to understand
supported runtime boundaries. A caller can keep a home across a call when the
complete normal-return footprint preserves it. Inputs and guaranteed results
remain independent of possible clobbers and full invocation observations.

This delivery covers:

- SArgs capture into the existing contiguous parameter frame.
- Word/byte multiply, word shifts and wide multiply/shifts.
- Compiler-owned division/remainder families and their terminal fault paths.
- Ordinary Action library wrappers through existing analysis, plus validated
  `SYS.SCompare`, `SCopy`, `Zero`, `SetBlock` and `MoveBlock` bindings.
- A coverage inventory identifying the remaining library contracts.

Keep source syntax, SemIR/NIR contracts, the public ABI, helper instruction
bodies and emitted capture descriptors unchanged. Keep the ZP pool, ownership
rules, placement transaction, final-cost acceptance and rollback. The four-byte
limit on a routine's own home-selection eligibility is separate from removing
that limit on understanding a callee's argument capture.

Do not fold the other Huffman repairs into this work: public LONGCARD result
support, parameter/result ABI separation, caller-body eligibility, local-only
leaf discovery and ordinary-global overlap precision remain separate. Broader
conversion, I/O and device contracts follow the inventory, after their returning
dependencies are supported. No blanket library purity or preservation claim.

## Working baseline

Before the first behavior change, record the current revision, configuration
and source hashes with the existing
[Huffman audit](benchmarks/6502-huffman-facts/README.md) and TN comparison path.
Reuse matching existing receipts where possible; do not regenerate large
artifacts after every slice. The audited Huffman baselines are 3,851 packaged
bytes for portable storage and 3,781 for the fixed-address driver, both with
zero named ZP homes. Compare each program against itself at the same origin and
runtime. Keep raw images, traces and listings under ignored `build/`.

Commit the completed design/plan and their relevant cross-links separately
from implementation. Do not sweep other documentation or audit files into that
commit without checking their scope.

## Slice 1 — Describe SArgs capture once

- Add one shared capture-summary builder using `SArgsCapture`, its owning
  frame and the supported helper binding. Reuse current home, effect and
  transfer types; introduce no new executable IR form or descriptor parser.
- Enumerate all incoming ABI byte lanes, including `$A3` onward. Describe each
  destination parameter byte and its guaranteed definition on normal return.
  Keep scratch writes and register/flag clobbers separate from those definitions.
- Record supported binding provenance at the existing resolver boundary where
  needed. Use a small local witness; helper names or matching addresses alone
  cannot certify a custom replacement. Keep facts unavailable until binding.
- Retain the Break state observation and may-stop behavior. Describe the normal
  copy and scratch footprint without requiring an Error-handler clobber map.
  Keep full invocation effects for deletion/reordering consumers.
- Reuse the existing frame/count checks, validate representable ABI lanes and
  preserve the emitted call/descriptor. This slice produces facts; entry and
  residence admission remain unchanged until slice 2.

Check a mixed five-byte scalar/pointer capture, a four-byte direct-capture
control, malformed frame/count, an unsupported binding and every captured lane.
Run the affected capture-summary tests and existing
[capture emission tests](../src/mir6502/materialize/sargs_capture_tests.rs).
Commit the slice.

## Slice 2 — Admit SArgs callees to existing analyses

- Feed the capture summary into actual-entry home/machine transfers and callee
  memory summaries. Reads precede guaranteed definitions. Capture initializes
  callee parameter bytes; it does not logically initialize an overlapping
  caller object.
- Remove the four-byte capture-strategy threshold from actual-entry ABI
  completeness for validated SArgs frames. Keep type/frame consistency and
  canonical checks at every call, including repeated sites. Preserve current
  supported ABI extents and planned-capture restrictions.
- Replace blanket SArgs rejection in acyclic entry composition and residence
  call checks with the shared supported-capture query. Compose normal-return
  writes through the whole callee, including its frame, scratch, temporaries
  and descendants. General full-effect queries keep their current meaning.
- Keep structural checks before helper resolution, and run mandatory
  binding-dependent residence checks immediately after resolution in baseline
  preparation and every replay path. No pending proof may reach dependent
  cleanup or final acceptance. Do not disable unrelated verifier checks.
- Extend existing freshness/compatibility checks to the capture binding and
  normal-return facts used by callers. A changed descriptor, frame, binding or
  descendant invalidates those facts and uses existing rejection/replay.
- Retain the callee's current contiguous parameter homes. Leave its own
  placement eligibility, terminal handoff and unrelated spill-discovery scope
  unchanged. This slice enables caller preservation across SArgs callees.

Check a live caller ZP home, scratch collision, missing/reordered later argument
lane, nested wrapper and stale binding/frame. Run affected entry/residence tests,
the SArgs cases in [call ABI tests](../tests/mir6502_call_abi.rs), and
`sargs_capture_executes_all_lanes_and_returns_past_descriptor` in the
[cross-call VM target](../tools/vm-runtime-tests/tests/cross_call_homes.rs).
Extend that existing VM control to observe caller preservation in both runtimes.
Commit the completed SArgs slice before proceeding to the wider catalog.

## Slice 3 — Consolidate helper signatures and result layouts

- Put existing argument, result and effect descriptions behind one small
  MIR6502 contract module keyed by `MirRuntimeHelper`. Retain thin accessors
  from [materialization](../src/mir6502/materialize/runtime.rs).
- Make the primary and additional result widths explicit. Provide one byte
  projection for both, including all of `$C4–$C7` for wide helpers. Describe
  supported entry and normal-return facts independently of may-effects.
- Reuse the binding and capture machinery from slices 1–2. Avoid a second
  signature catalog, analysis cache or helper whitelist in each consumer.
- Strengthen declaration/invocation checks for canonical inputs, primary and
  additional result homes/widths and effect consistency before stronger facts
  are consumed. Retain conservative behavior for unsupported contracts.

Run focused helper signature, effect-projection and verifier tests. Check
missing high result bytes and inconsistent primary results explicitly. Use
existing emission controls to confirm unchanged helper selection and bytes;
do not update executable fixtures solely to hide a metadata discrepancy.
Commit the slice.

## Slice 4 — Compose multiply and shift contracts

- Validate contracts for `MulByte`, `Mul`, `Lsh`, `Rsh`, `Mul32`, `Lsh32` and
  `Rsh32` against their selected generated, linked or resident implementations.
  Cover private calls/fallthrough where present. Reviewed manual contracts are
  sufficient for supported assembly; no general disassembler expansion is required.
- Adapt validated helper inputs and guaranteed results to the existing entry
  and machine/home liveness transfers. Keep argument reads even for a pass-through
  result. Do not infer entry requirements from the full scratch read footprint.
- Allow these helpers in native call composition and direct residence checks.
  Reuse the normal-return query and binding freshness from the SArgs slices.
  Source caller-body restrictions remain independent.
- Keep conservative register/flag clobbers and publish no new flag provenance.
  Extend the fact report with helper sites and unavailable-contract reasons;
  ordinary Action-call counts alone must not classify helper-bearing routines.

Check zero/boundary shifts, result lanes, initialized internal scratch versus
real inputs, direct and nested preservation, and an unsupported override.
Run relevant MIR6502 cases from the existing integer-shift and wide-integer VM
targets, selecting cases that exercise the affected helper families. Include
an ordinary Action arithmetic library wrapper as the analyzed-library control.
Commit the slice.

## Slice 5 — Use normal-return facts for division

- Complete contracts for signed/unsigned word, narrow, wide and shared-divmod
  helpers using their existing ABI/result layouts and `helper_return_effects`.
- Preserve full invocation effects and the divide-by-zero stop. Normal-return
  entry/result and preservation queries must not be rejected merely because a
  terminal Error path has unknown effects or calls the runtime/OS.
- Retain generated non-return guards, including when Error itself returns.
  Do not export an output from `Fault` or obtain preservation vacuously from
  an invocation with no normal return.
- Compose the normal-return projection through native wrappers without changing
  general read/write summaries used for deletion, motion or observable stores.

Run the affected helper-contract tests and MIR6502 cases from
[modern arithmetic](../tools/vm-runtime-tests/tests/modern_arithmetic.rs) and
[wide integers](../tools/vm-runtime-tests/tests/wide_integers.rs). Cover a live
home outside successful scratch, unused-result division by zero, prior observable
state and the existing returning-Error-handler case. Reuse these controls rather
than adding a new fault-test matrix. Commit the slice.

## Slice 6 — Retain library identity and certify a reader

- Extend [interface resolution](../src/mir6502/interfaces.rs) to retain a typed
  association between the public interface/result signature and its selected
  binding before declaration-only routines are removed. Keep per-binding
  evidence for both absolute and linked targets.
- Feed library contracts into the same query/transfer vocabulary as helpers,
  while retaining the public Action ABI. Ordinary Action implementations use
  existing analysis; embedded assembly/resident entries use reviewed metadata
  attached to implementation/binding identity.
- Certify `SYS.SCompare` in each supported provider, with complete scratch,
  entry and public result facts. Retain string read uncertainty independently
  of finite writes. Include its internal machine control flow in the contract.
- Preserve qualified/prelude identity and user-name shadowing. Preserve
  interface-specific result widths even for shared-address bindings such as
  `Peek`/`PeekC`; this identity regression need not grant those routines new
  optimization eligibility.
- Reuse the existing pre-materialization SYS resolution boundary and freshness
  checks after body transformations. Do not make the optimizer consult SemIR
  or change public declarations to carry physical register clobbers.

Run focused interface/binding tests and a small string-reader VM control for
both providers, observing its signed result and live caller state. Check a changed
implementation/dependency invalidates its certificate. Commit the slice.

## Slice 7 — Describe library buffer effects

- Add only the shared effect representation needed to retain argument-relative
  buffer accesses alongside fixed scratch. Use captured typed arguments and
  existing ownership/overlap queries; unknown extent remains unknown in that
  direction. Do not add a library-specific alias analysis.
- Certify `SCopy` as the first buffer writer. Retain both source reads and
  destination writes. Project its effect at each invocation, including repeated
  calls with different actual buffers.
- A consumer unable to represent the combined footprint keeps its conservative
  answer. The new residence query may exclude unexposed private homes only
  through the existing ownership contract. Exposed objects, explicit numeric
  addresses and unsupported machine effects retain their observations.
- Keep observable calls and reads ordered. A finite scratch set or unused
  result cannot make the operation removable. Preserve full physical checks
  and final-cost acceptance.

Check a live private home, exposed destination, unknown destination/extent and
valid alias/overlap cases under the routine's semantics. Use focused effect and
residence tests plus one cartridge/standalone copy VM control. Commit the slice.

## Slice 8 — Certify the memory library group

- Apply the same buffer-effect machinery to `Zero`, `SetBlock` and `MoveBlock`,
  using separate evidence for each selected implementation.
- Include Zero's fallthrough into SetBlock and each routine's complete scratch
  and result/entry behavior. Read dynamic sizes from actual argument lanes;
  known counts do not prove arbitrary destinations private or disjoint.
- Validate larger public argument lists independently of capture strategy.
  Standard-library assembly entries that consume ABI lanes directly must not
  receive an invented SArgs prologue. Keep any remaining unsupported boundary
  explicit in the coverage report.
- Start the library inventory with all selected contracts and their proof
  components. Record conversion/I/O/device gaps and returning dependencies;
  terminal Break/Error alone is not an unsupported dependency.

Run grouped memory-operation controls in both providers, covering zero/nonzero
counts, a page boundary, valid overlap behavior, scratch collisions and live
caller state. Reuse existing runtime-link closure tests for fallthrough, adding
only a missing contract assertion where needed. Commit the slice.

## Slice 9 — Measure the completed coverage

- Repeat the Huffman facts/home audit for both layouts and the no-inline control
  with matching origins/configurations. Run its MIR6502 cartridge/standalone
  VM tests and retain the existing LF/CRLF loader coverage.
- Compare TN against the recorded baseline: actual packaged size, named homes,
  temporary ZP, changed routines and remaining blockers. Exercise the affected
  TN/runtime controls if its selected code changes. No size saving is promised.
- Save a compact receipt with source/image hashes, contract availability,
  helper/library sites, normal versus full effects, placement outcomes and any
  fallback. Report compiler work/time only from comparable configurations.
- Verify trace/report generation does not change the image, remove temporary
  probes, update design/plan status and document the remaining library inventory.
  Keep large raw artifacts under `build/`. Commit the qualification slice.

## Validation discipline

Use filtered library tests and explicit affected integration/VM targets. Do not
run the full repository suite after each slice or repeat passing cases after
documentation-only changes. Batch distinct boundary cases; use existing VM
controls for runtime behavior and units for the contract matrix. If affected
host fixture processing is newline-sensitive, exercise LF and CRLF through
the actual loader/instrumentation path.

Changes are intended to stay within MIR6502 contracts and their consumers.
If implementation changes shared frontend/NIR/verifier/printer contracts, run
the required shared-contract checks from AGENTS.md. Broaden backend coverage
when the actual change or a failure requires it. Record exact commands, filters
and results in each slice's receipt; never label filtered runs a full suite.

The accepted result is accurate, current contracts used by existing analyses,
with retained execution behavior and exact rollback on failed proofs/costs.
Contract availability and ZP placement are separate outcomes. Unchanged TN or
Huffman size is acceptable when independent restrictions still block placement.

## Implementation checks

All commands use `CARGO_TARGET_DIR=build/checked-arena/shared-target`, dev/test
optimization level 1, debug info disabled and two build jobs.

- Slice 1: `cargo test --lib sargs`: 11 passed. Includes mixed five-byte
  capture metadata in both runtimes, stale/custom binding rejection and the
  existing descriptor/relocation emission oracle at two origins. No admission
  or emitted-code change. A test with unused call arguments encountered an
  existing pre-materialization temp-cleanup loop; the metadata control now
  constructs the callee without an unnecessary caller. Runtime calls remain
  covered by the existing emission and upcoming VM controls.
