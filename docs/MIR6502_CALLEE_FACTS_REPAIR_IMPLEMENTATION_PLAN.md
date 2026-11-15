# MIR6502 callee-fact repairs: implementation plan

Prepared 2026-11-15. Status: slices 1–5 completed and validated; resume slices
6–7 in order. The narrower
[private-storage boundary plan](MIR6502_PRIVATE_STORAGE_BOUNDARY_IMPLEMENTATION_PLAN.md)
was implemented separately. Broader ZP placement remains deferred.
Historical baseline: `7ab6f840ba0475c5ddde9194c130ee83d96c83d1` and the
[TN leaf-facts audit](benchmarks/6502-tn-leaf-facts/README.md).

Slice 1 validation: one grouped summary test and one VM group passed. The VM
group compiles default and optimized configurations, loads actual LF/CRLF source,
checks the Position result is 1, and checks valid ALU-result flag reuse. Its
whole-pipeline coverage makes a separate integration-only run unnecessary.

Slice 2 validation: 29 callee-analysis tests (including bounded-machine decoding)
and the independent indexed/offset VM control passed. The corrected TN baseline
is 9,862 bytes; see [evidence](benchmarks/6502-callee-facts-repair/README.md).

Slice 3 validation: all 30 callee-analysis tests and the reader/writer VM group
passed. The group covers default/optimized ActionCart and Standalone, including
page crossing and caller preservation. TN is 9,846 bytes, down 24 from the
post-private-storage baseline; see the directional-write review in the evidence
above. No full suite was run.

Slice 4 validation: the compare CFG and call-contract groups passed, together
with all 11 machine-liveness and six entry-flag tests. The independent VM group
passed default/optimized ActionCart and Standalone with all 16 incoming flag
combinations. The existing entry-flag integration control passed; its optional
MADS case remained ignored. No general compare or word-update expansion was needed.

Slice 5 validation: all seven callee-fact unit groups and all five VM groups
passed. Array references retain their Array class, carry a typed pointer ABI,
and pass planned/actual entry validation; malformed descriptors fail closed.
The direct-memory CMP form needed by Instr is also certified. Existing capture
checks remain unchanged. No NIR contract or array-placement rule changed.

Repair the facts consumed by existing optimizations, then qualify the newly
eligible cases. Implement the slices below in order and commit each after its
focused checks pass. Keep unrelated working-tree changes outside those commits.
Testing remains minimal, using independent execution controls and the existing
shared build target.

The audit baseline is a 9,791-byte TN image. It contains an unsound flag fact;
preserving that image is **not** an acceptance requirement for a correctness fix.
Measure the remaining slices against the current **9,846-byte** slice-3 image.
Retain the 9,862-byte correctness baseline and 9,870-byte private-storage result
as historical evidence, without rebuilding them. Continue
to require strict whole-image savings for each existing home-selection trial,
with complete proof replay and exact rollback. Individual routine growth remains
diagnostic.

## Scope and contracts

| Finding | Resolution | Slice |
| --- | --- | --- |
| Position invents an N/Z result; GetD has the same inference | Known provenance or a proved flag-producing register is required | 1 |
| Indexed machine reads are reduced to an object base; relocation addends disappear | Conservative read footprints and faithful operand identities | 2 |
| Key, Instr and Fnamecmp have unknown writes solely because they read indirectly | Directional write completeness from shared operation effects | 3 |
| Fnamecmp unnecessarily requires incoming C/Z | Certify definite flag definitions for the selected indirect-byte compare recipe | 4 |
| Instr's array-reference parameter fails the scalar entry gate | Carry and validate its existing pointer ABI representation explicitly | 5 |
| Emitted map reports every routine as PROC without a result | Populate diagnostic signatures from verified callable facts | 6 |
| New facts need validation against real consumers | Regenerate the census and measure TN plus independent controls | 7 |

Keep the following invariants throughout:

- Unknown value provenance never proves equality. A store establishes a memory
  home for its source; it does not establish processor flags.
- An Exact memory summary covers every possible access in that direction.
  Unknown reads may coexist with exact writes. Indirect/opaque writes remain
  unknown unless an existing, separate bounded proof establishes their coverage.
- Definite flag definitions are different from possible clobbers. Entry proofs
  preserve every read-before-write dependency and union requirements at CFG joins.
- ABI facts come from typed, verified NIR and are carried into MIR. Analyses do
  not consult SemIR or recover meaning from source text or display names.
- Facts belong to their current program/body snapshot. A change to a callee
  still requires existing preservation replay; recomputing facts alone cannot
  validate caller rewrites that used the old facts.
- Unknown pointee reads remain observable. Better write summaries do not
  authorize removing/reordering those reads or weakening source-ZP exclusions.

Reuse the shared analyses and current transactions. Add no new optimization pass,
per-function exception, subset retry, or whole-program analysis/emission stage.
Correctness fixes are unconditional. Precision extensions use their existing
consumer gates; no extra configuration switches are planned.

This work does not add recursive residence, admit machine/opaque callees to the
ordinary-call selector, recover all missing machine entry facts, or promote
array objects into ZP. Mutable native callee N/Z exit facts remain withheld from
rewriting. The unsupported `enable_peepholes=false` path observed by the audit
is a separate issue and is not a reference implementation for these tests.

## Slice 1 — stop inventing N/Z facts at stores

Primary code: [known_callees.rs](../src/mir6502/analysis/known_callees.rs),
`record_machine_register_store`; existing call transfers in
[machine_values.rs](../src/mir6502/analysis/machine_values.rs).

1. Retain the `zn_register == Some(source)` proof. It is meaningful even when
   the loaded value has unknown memory provenance: a genuine LDA still sets
   N/Z from A.
2. Permit the alternative equality proof only when both provenance facts are
   known and equal. `None == None` must not establish any relationship.
3. Keep rebasing the stored A value to its destination where valid, independently
   of whether N/Z can be rebased. Preserve valid STX/STY flag provenance too.
4. Check the reset paths for unknown calls and clobbers: stale register tags
   must not survive a boundary that loses their flag relationship. Make only
   repairs needed for this invariant.

Minimal validation:

- One grouped summary regression: initial STA; STA after an unknown call;
  unequal known A/NZ; and valid load/ALU/indirect-load then store cases. Check
  both ordinary and rewriting summaries. Preserve valid X/Y and A facts.
- Promote the audit's [Position control](benchmarks/6502-tn-leaf-facts/position-control.act)
  to a normal assertion-based VM regression. `Check()` must return **1**, with
  kx=0, ky=128 and balanced stack, in default and optimized configurations.
  Include a valid flag-reuse control so the repair does not blanket-disable it.
- Add a small synthetic call-then-store summary case for the GetD shape; avoid
  depending on CIO or a real device to establish this analysis invariant.

Document the exit-provenance invariant in
[MIR6502_PSEUDO_MACHINE_CONTRACT.md](MIR6502_PSEUDO_MACHINE_CONTRACT.md).
Commit the fix and its regressions. Do not defer it until size recovery succeeds.

## Slice 2 — make machine memory footprints conservative and address-correct

Primary code: `summarize_machine_instruction_reads`, `expand_machine_operand`
and their consumers in [known_callees.rs](../src/mir6502/analysis/known_callees.rs),
including [bounded_memory.rs](../src/mir6502/analysis/known_callees/bounded_memory.rs).

1. Stop publishing a single Global/Static base byte as the footprint of an
   absolute indexed read. In the context-free summary, make that read set
   Unknown. Preserve independently valid writes, register preservation and
   return facts; do not discard the whole summary merely because reads widen.
2. Apply typed relocation addends when deriving both encoded operand values and
   logical memory identities. Match the emitter's addressing and arithmetic
   rules, including low/high-byte selection and carry into the high byte.
3. Distinguish an address operand from an immediate selected address byte.
   A low/high-byte constant must not become a claim that the full referenced
   object is the memory being accessed. Unsupported symbolic offsets, wrapping
   or truncation must lose the affected proof instead of silently using offset
   zero. Preserve existing conservative exclusions in bounded-machine proofs.
4. Check the small set of consumers that use Exact reads to prove private
   storage unobserved, including terminal cleanup and index-register parameter
   carriers. They must retain their conservative behavior when reads widen.

Use the existing Exact/Unknown representation. Bounded indexed-read range
inference is deferred; Internal's four-byte table is not a reason to introduce
another range analysis in this repair.

Minimal validation:

- One grouped summary test for absolute X/Y indexed Global/Static operands;
  direct reads/writes at nonzero positive and supported negative addends;
  selector/carry boundaries; and an unrepresentable operand that fails closed.
  Ensure an indexed read with fixed writes leaves those writes exact.
- One independent consumer regression where an index or relocation addend
  reaches a byte outside the previously reported base. Check that the consumer's
  observation query refuses to prove that byte unread, and execute a control
  with a needed store and a host-computed expected result. A failing TN runtime
  scenario is not a prerequisite for fixing an incomplete footprint.
- Run the affected bounded-machine operand/footprint tests once, because its
  decoder shares the changed flattening path.

Update the memory-summary contract in the pseudo-machine document. Regenerate
TN once after slices 1–2 and retain a **correctness-repaired baseline**: full
facts, image/map/relocation hashes, configuration, size and routine footprints.
Expected fact changes: Position and GetD no longer claim the unsupported N/Z
relation; CalcAdr/Internal/Ascii have unknown reads and retain their fixed writes.
Investigate any other changed fact or growth; do not restore an unsound fact to
meet the old 9,791-byte result. Commit the repair and baseline evidence.

## Slice 3 — derive write summaries from write effects

Primary code: [effects.rs](../src/mir6502/analysis/effects.rs) and
`summarize_op_writes` in [known_callees.rs](../src/mir6502/analysis/known_callees.rs).

1. Audit every assignment to `has_unknown_effects` and classify which direction
   is unknown. The current classifier already has directional fields:
   `indirect_reads`, `indirect_writes`, `may_write_any`, structured effects,
   direct write ranges, home effects and address-pair writes.
2. Make the write-summary projection use that directional information. A broad
   unknown-effect marker caused only by a read must not invalidate a complete
   write footprint. Retain the marker and compatibility projections for consumers
   that still need conservative read/order information.
3. Remove the isolated `OffsetPointerByIndirectByte` exception in favor of this
   shared rule. Do not replace it with a list of specially permitted opcodes.
   Repair any missing write information in the central classifier, not in a TN
   or call-selector workaround.
4. Include every direct destination byte, physical parameter/local binding,
   return slot and caller-visible scratch/pair write. Unsupported or unresolved
   write homes still produce Unknown. Do not publish Exact for an unexpanded
   recipe whose implicit temporary writes cannot yet be enumerated.
5. Keep unknown/indirect calls, helpers, machine blocks, barriers, indirect
   stores and read/modify/write pointee operations conservative. Structured
   call summaries keep their existing independent read/write handling.

Minimal validation:

- One grouped effects/summary test spanning an indirect load, byte/word compare,
  pointer-offset recipe and indirect-to-fixed-byte copy, with their actual
  scratch/destination footprints. Exercise the representation stages in which
  their summaries are consumed. Pair with indirect-store/compound-write,
  opaque-call and unresolved-home negatives.
- One integration/VM control with a live caller value across a pointer-reading
  leaf. Check the pointee result, caller value and source-memory sentinels,
  including a page crossing; keep an actual pointer writer rejected.
- Confirm Key/Instr/Fnamecmp now have complete write summaries while their
  reads remain Unknown. Next and Value remain controls. Before allowing broader
  selection, verify the emitted writes are covered by the new summaries.

Use the existing combined home-selection batch and cost guard. Fixing the
summary does not promise placement or savings. In particular, Instr and Fnamecmp
still have the entry blockers addressed below. Commit the projection repair.

## Slice 4 — certify the compare that defines Fnamecmp's entry flags

Primary code: `entry_flag_transfer` in
[machine_liveness.rs](../src/mir6502/analysis/machine_liveness.rs), its callers in
[entry_inputs.rs](../src/mir6502/analysis/known_callees/entry_inputs.rs), and the
selected compare recipe in [emit.rs](../src/mir6502/emit.rs).

1. Certify the supported unsigned `CompareIndirectBytes` form with a Flags
   destination. Its successful selected emission executes a real CMP and
   definitely defines C/N/Z, while preserving V. Keep its register/pointer
   inputs and observable memory reads intact.
2. Match the supported operand/consumer forms actually emitted. Unsupported,
   signed, temp-result or potentially omitted comparisons do not inherit the
   certificate merely because their broad effect summary says they write flags.
3. Recompute Fnamecmp's entry requirements. The target is A/X/Y and `$A3`, with
   no incoming C/N/Z/V requirement. Its actual ordinary byte comparisons also
   need consideration: add a further definite-definition case only if the
   selected emission and the failing dependency demonstrate it is necessary.
4. Do not broaden all comparisons or all word updates preemptively. Zero-sized
   shifts, self moves and folded comparisons can emit no flag definition. Word
   INC/DEC need their own all-path proof if a remaining entry dependency requires
   them. Preserve the read-before-write rule for ADC/SBC/rotates using incoming C.

Minimal validation:

- One liveness/entry group: compare before a C/Z branch, compare on only one
  incoming path, a loop, an incoming-flag read before the compare, and V remaining
  live. Keep unsupported/omittable comparison cases conservative.
- One independent comparing-leaf caller with controlled incoming flags and
  equal/less/greater inputs. Assert results and canonical argument delivery in
  the VM. Exercise the existing per-call residence validation as well as the
  entry fact itself.

Extend the definite-definition contract referenced by the
[entry-flag plan](MIR6502_CALLEE_ENTRY_FLAG_SUMMARIES_PLAN.md) and
[parameter-home design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md). Keep mutable
native **exit** flags withheld; this slice refines **entry** obligations only.
Commit the certified forms and tests.

## Slice 5 — recognize pointer-represented array parameters at entry

Primary code: [lower.rs](../src/mir6502/lower.rs), [ir.rs](../src/mir6502/ir.rs),
[abi.rs](../src/mir6502/abi.rs), [param_capture.rs](../src/mir6502/abi/param_capture.rs),
and `direct_parameter_abi_is_complete` in
[entry_inputs.rs](../src/mir6502/analysis/known_callees/entry_inputs.rs).

1. Enrich the existing direct-parameter ABI descriptor with a typed scalar vs
   pointer representation, rather than adding a second, competing eligibility
   table. A small enum such as `Scalar(MirWidth)` / `Pointer` can retain a width
   accessor for existing consumers. Pointer means the supported target pointer
   width/address space proved during lowering, not any two-byte object.
2. Derive that descriptor from the existing verified `NirCallableSignature`
   types. Instr's array-form `s` is passed as a pointer. Preserve its Array storage
   class; do not relabel the source object Scalar to satisfy a backend gate.
3. Validate the descriptor against the actual frame: parameter count/order,
   supported byte widths, zero offset, no initializer, valid parameter identity
   and capture width. Permit Array storage only for the explicitly certified
   pointer representation. Reject aggregates, descriptors with another shape,
   malformed metadata, variadics and unsupported capture conventions.
4. Make PlannedCapture and ActualEntry use the same validated ABI description
   and existing capture builder. Retain the rule that a memory-sourced capture
   cannot be treated as a flag-preserving register store. Snapshot identity and
   stale-fact checks must include the refined descriptor.
5. Restrict the change to certifying callee entry and the necessary ABI plumbing.
   Instr's own parameter homes can remain ordinary memory. General array-home
   selection, array-local promotion and changing the four-byte placement limit
   are outside this slice.

The selected Instr body also exposes an uncertified direct-memory byte compare
once its ABI is recognized. Certify the unsigned Flags form comparing A with a
direct memory operand: emission always performs CMP. Keep omitted/folded and
immediate equality-to-zero forms conservative, and include this form in the
existing slice-4 CFG/flag regression group. This is a shared fact repair, not
an entry exception for Instr.

Minimal validation:

- One lowering/entry group for a CHAR ARRAY reference, an explicit pointer and
  a scalar word; reject a same-size non-pointer aggregate, inconsistent frame
  shape, incomplete capture and unsupported ABI metadata. Check both entry
  stages and a stale descriptor/body.
- One integration/VM caller with a live scalar across an Instr-like search.
  Use empty, found and absent cases plus a page crossing. Check canonical
  A/X/Y inputs, caller preservation, results and stack balance. Keep a malformed
  or extra-input call rejected.

Update the ABI/entry contracts in the pseudo-machine and parameter-home design
documents. This consumes existing NIR facts; no NIR shape, semantic lowering or
printer change is planned. If implementation discovers a missing NIR contract,
separate that migration and run the required shared-contract checks rather than
recovering semantics from names. Commit the MIR fact and entry support.

## Slice 6 — repair callable metadata in emitted maps

Primary code: `mir_routine_signature` in [emit.rs](../src/mir6502/emit.rs) and
output finalization in [mod.rs](../src/mir6502/mod.rs).

Populate callable kind, parameter types/widths and return type/width from the
existing verified callable signature at final output construction, using the
compiler's routine-identity correspondence. Keep these as diagnostic metadata;
no optimizer should read a map to recover a missing MIR fact. Do not infer FUNC
from a final store to `$A0`, or from `scalar_signature` alone: pointer signatures
can legitimately lack that integer-only optimization fact. Generated/runtime
routines with missing source metadata need an explicit unavailable result,
not invented source types.

Use a small metadata integration test containing a procedure, byte function,
word/pointer function and pointer/array parameters. Include TaggedFiles and
Key/Next as audit spot checks. Verify executable bytes and relocations are
unchanged by this slice; map differences are intentional diagnostic repairs.
Keep the shared map structure and other backends unchanged if its existing
fields suffice. Commit independently of optimization changes.

## Slice 7 — qualify the final facts and measure the effect

Retain the original audit and the slice-2 correctness baseline. Write new
qualification artifacts separately; do not overwrite the wrong-code witness or
make a new compiler match its obsolete hashes.

1. Re-run the full leaf-facts generator on current TN. Omit its old-image equality
   argument; the historical image is a measurement reference, not an oracle for
   the repaired compiler. Classify all no-MIR-call
   candidates again, including machine wrappers, computed transfers and menu
   data. If the inventory changes, explain it instead of forcing the old counts.
2. Check summary transitions against emitted code: Position/GetD flag repair;
   machine indexed-read widening; complete Key/Instr/Fnamecmp writes; Fnamecmp
   entry flags; Instr's pointer entry; and accurate callable metadata. Recheck
   entry register/flag/fixed-byte requirements across all audited leaves, and
   explain every change rather than checking only the motivating routines.
3. Record actual eligibility and placement for FindItem, PopUp, Sort and Handle.
   Reaching an entry/write check, passing it and receiving a selected home are
   distinct outcomes. Report both the caller body/eligibility gate and the
   independent callee-contract check. Sort's indirect array writes are outside
   current placement scope even after Fnamecmp's entry flags are repaired. These
   slices do not relax that restriction. Report the first actual selector blocker
   without mistaking a diagnostic call check for evidence of candidate selection.
4. Measure complete image size against the current 9,846-byte baseline, retaining
   comparisons with the old audit and correctness-repaired baseline as historical
   context. Attribute correctness growth separately from
   precision-related savings. Include all changed routine code/ordinary/ZP
   footprints and the Ord/Items/Next controls, not just hoped-for improvements.
5. Preserve exact rollback, per-site argument checks, final-cost acceptance and
   one combined ordinary-call trial. Check the existing rollback control and a
   later-site unknown-write/entry-input rejection on the final implementation.
6. Record selected decisions, image/map/relocation hashes, compiler/source/config
   identity and trial/materialization/emission counts. One elapsed/RSS sample is
   enough; no repeated performance benchmark is required.

Require passing independent result oracles for each changed contract. TN savings
are measured, not required or promised. If a precision change yields no savings,
retain only a justified, tested general fact improvement; do not weaken proof
or cost guards to make the sample smaller. Commit qualification and document
status updates after the checks are complete.

## Minimal testing and build budget

Keep focused summary/effect/entry unit groups beside their existing analyses.
Use the `callee_facts_` prefix for new grouped cases. Extend the existing VM target
`tools/vm-runtime-tests/tests/callee_facts.rs` and its shared helpers. Reuse the
existing entry-flag integration target when its consumer behavior is affected;
do not add a duplicate integration layer for behavior already covered by a VM
control. A small metadata integration target is appropriate for slice 6's maps.

Run only new and directly affected groups per slice, checking nonzero selected-test
counts. Re-run earlier groups only for a changed fact/consumer or unresolved
failure. Commands below are filters, not a requirement to execute all groups at
every slice; apply the shared build settings documented below:

```sh
cargo test --lib callee_facts_
cargo test --test mir6502_callee_entry_flags
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test callee_facts
cargo test --lib later_trials_keep_the_exact_selected_input_and_executable
```

The final qualification includes existing cross-call rejection and rollback
controls. Record which unit, integration or VM group validates each changed
contract. Do not repeat completed slice-1/2 baselines or full backend suites.

Use the shared `build/checked-arena/shared-target`, dev/test opt-level 1, no debug
information, incremental off and two jobs. Keep large images and traces under
an ignored `build/callee-facts-repair/` directory. Do not create another target
tree or rebuild historical compiler revisions.

Batch fixture scenarios. Normalize host text before any newline-sensitive
instrumentation; if adding such instrumentation, run the actual LF and CRLF
loading path in the affected grouped test. Preserve binary/ATASCII bytes. Use
ActionCart `$3000` as the TN measurement, both default/optimized settings for the
flag regression, and a small standalone smoke case for the newly admitted call
path. No origin/runtime Cartesian product is needed.

These are MIR6502 changes. No repository-wide NIR sweep or other backend suites
are planned. Broaden testing for an actual relevant failure or shared-contract
change, and follow the repository's shared-contract requirements if that scope
changes. For this plan-only change, check the document and its links; run no
compiler tests.

Completion means the wrong-code control returns the correct result, Exact facts
have sound coverage, the targeted entry obligations are accurate, unknown and
unsupported cases remain conservative, and the measured final outcomes are
recorded without changing the existing transaction guarantees.
