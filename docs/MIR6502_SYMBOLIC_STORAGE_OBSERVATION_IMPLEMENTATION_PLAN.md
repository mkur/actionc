# MIR6502 symbolic storage observations: implementation plan

2026-11-16. Status: all four slices implemented and qualified.
Continues the completed [fixed-address observation plan](MIR6502_FIXED_ADDRESS_OBSERVATION_IMPLEMENTATION_PLAN.md)
under the [unexposed-object contract](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md)
and [storage-observation design](MIR6502_STORAGE_OBSERVATION_DESIGN.md).

## Outcome and baseline

Separate compiler-generated SArgs descriptor data from executable machine code,
and retain decodable symbolic storage observations until authoritative layout
can resolve them. An unknown pointee remains distinct from an unknown explicit
operand. Completing an observation inventory does not establish a complete
callee read/write summary.

Use `96e91785` and its [recorded qualification](benchmarks/6502-fixed-address-observations/README.md):
TN is **9,838 bytes**, Sort has 250 code bytes, three inline-home bytes and no ZP
homes. Reuse its image, facts, maps and receipt; do not rebuild an old compiler.
The four remaining unresolved blocks have these causes:

| Owner | Recorded block | Cause | Planned correction |
| --- | --- | --- | --- |
| Range | m34 | Generated SArgs frame-address/count bytes are scanned as instructions | Typed ABI capture operation |
| _Cio | m11 | `JMP (ErrorHandler)` needs the final pointer-cell address | Deferred pointer-fetch observation |
| _StrNam | m13 | `fnam-1,Y` is rejected as a negative object offset | Preserve signed encoded-base expression |
| MakeJmp | m29 | `commandTargets-2,X` and `commandTargets-1,X` have the same problem | Same expression handling |

These names and IDs identify qualification evidence, never implementation rules.
After representation changes, locate blocks by owner and typed operation rather
than assuming machine IDs remain unchanged.

## Contracts to record before implementation

Extend the existing unexposed-object design in the first implementation slice
with the following boundaries. Keep the extension focused on invariants.

- **MIR6502 owns ABI capture.** The SArgs call and its three-byte inline descriptor
  form one typed operation. Its frame target uses parameter identity in the owning
  routine, and its count is explicit. Emission owns the resolved address and byte
  encoding. This is neither source DATA nor a new NIR operation.
- **Decoding precedes address resolution.** An explicit observation retains its
  owner, typed target, signed relocation addend, access direction and addressing
  behavior. Distinguish complete fixed observations, decodable observations
  awaiting layout, and unsupported/invalid operands. Pending is not proof of
  safety; opaque executable bytes remain a discovery rejection.
- **Encoded bases are not object accesses.** An indexed operand may encode an
  address before its named object. Validate the referenced identity, retain the
  signed addend, then model the instruction's physical extent. Do not turn a
  negative addend into an unsigned object offset or infer an index bound from
  the declaration's size.
- **Physical evidence belongs to a layout.** Resolve against the final physical
  map for the relevant executable. Keep retained source exposure witnesses and
  check old and proposed homes. Missing identities, incompatible maps or any
  unresolved final obligation reject the trial.
- **Existing safety and cost rules remain authoritative.** Preserve helper
  effects, source-storage protection, private-scratch residence checks, public
  callee contracts, preservation replay, the combined placement proposal,
  strictly smaller complete-image acceptance and exact rollback. Add no subset
  retry, extra materialization or per-candidate emission.

No new assembly admission, index-range analysis, SArgs home-placement strategy,
source syntax, runtime implementation, general alias framework or NIR contract
change is in scope. Keep existing conservative handling of unsupported byte
selectors and raw mixed code/data blocks.

## Slice 1 — represent SArgs capture explicitly

Primary code: [MIR types](../src/mir6502/ir.rs),
[ABI materialization](../src/mir6502/materialize/abi.rs),
[helper contracts](../src/mir6502/materialize/runtime.rs),
[verification](../src/mir6502/verify.rs), and
[emission](../src/mir6502/emit.rs).

1. Introduce a dedicated typed capture operation, for example `SArgsCapture`,
   carrying the first parameter's ID and total capture byte count. Derive its
   helper arguments and effects from the existing SArgs contract. Replace the
   generated `RuntimeHelper` plus `MachineBlock` pair with that operation.
   Keep helper and descriptor inseparable; a jump/tail-call substitution must
   not bypass the descriptor ABI.
2. Verify the owning routine, parameter target, count encoding, contiguous ABI
   frame and allowed phase. Reject missing/malformed capture metadata. Preserve
   the existing policy for frame sizes outside SArgs support. Remove the old
   generator; do not recognize descriptors by byte pattern, name, source-span
   absence or proximity to an arbitrary helper call.
3. Update helper discovery, effects, liveness, storage exposure, materialization,
   printing, sizing and emission consumers. Keep the existing Unknown memory
   effects and register/flag/stack contract. Retain protection of the entire
   captured parameter frame, not just its first byte; removing the machine-block
   owner guard must not silently authorize movement or deletion of that frame.
4. Emit the same helper call, little-endian frame address and count-minus-one
   bytes through typed relocation handling. Preserve helper overrides, runtime
   linkage and readable descriptor/parameter listing annotations. The operation
   must remain an ABI barrier and must not enter the instruction scanner.
5. Add a grouped verifier/materialization control and reuse existing SArgs ABI,
   final-frame-address and listing tests. Execute a mixed five-byte capture,
   checking every parameter and return/stack behavior in both runtimes. Prove
   the descriptor no longer creates an unresolved machine observation while
   genuinely opaque source machine code still does.

Check byte/relocation equivalence on the small controls with placement fixed.
Changing representation is not itself a size optimization. Commit this slice
with the design extension and its focused checks.

## Slice 2 — retain typed symbolic observation expressions

Primary code: [machine observations](../src/mir6502/analysis/known_callees/observations.rs)
and the operand decoding helpers in
[known_callees.rs](../src/mir6502/analysis/known_callees.rs).

1. Extend the explicit-observation representation with typed base expressions
   and addressing behavior. Keep routine-qualified local/parameter identities,
   signed addends and relocation width/ZP requirements. Reuse opcode and operand
   decoding machinery; do not introduce a second instruction semantics table.
2. For supported full-address relocations, stop requiring the encoded base to
   fit inside the named object. Validate the identity itself and preserve its
   addend separately. Apply alias offsets and addends exactly once. Keep the
   stricter object-access checks used by exact callee summaries unchanged.
3. Describe absolute indexed accesses with the full 0..255 index extent; retain
   read, write and read/modify/write distinctions. Describe indirect accesses
   as pointer-cell fetch obligations, with unknown pointees still unknown.
   Keep full-ZP observations for ZP indexing and indexed-indirect addressing.
4. Retain the existing fixed-address fast path. Do not truncate invalid
   relocation arithmetic into a valid operand. At final resolution, validate
   the encoded base using emission's relocation rules, then apply the ISA's
   address wrapping to indexed accesses and pointer fetches.
5. Adapt all current observation consumers and the diagnostic probe to the new
   result shape. Until slice 3 supplies layout resolution, pending observations
   must still cause conservative rejection or an unknown physical observer;
   they must never disappear from cleanup. Successful decoding alone must not
   improve public callee summaries or approve a placement.

Use one proposed `symbolic_observation_` unit group: negative indexed addends,
typed local/global bases, alias/addend composition, reads and writes, invalid
IDs/widths/selectors, fixed and pending pointers, and opaque/truncated code.
Check emitted operand agreement on small source-derived cases. Commit once the
new representation is usable and all intermediate consumers remain conservative.

## Slice 3 — discharge obligations using authoritative layouts

Primary code: [physical checks](../src/mir6502/parameter_homes/physical.rs),
[placement transaction](../src/mir6502/parameter_homes.rs),
[cross-call discovery](../src/mir6502/parameter_homes/cross_call.rs),
[physical memory](../src/mir6502/analysis/record_bases.rs), and
[terminal cleanup](../src/mir6502/late_cleanup/mod.rs).

1. Add one resolver for the explicit observation expressions using existing
   physical maps and source-storage records. Resolve the typed base first and
   then the signed addend. Emit concrete ranges or separate pointer-fetch bytes;
   support 16-bit indexed wrap, ZP `$FF -> $00`, and NMOS indirect-JMP
   `$12FF -> $1200`. Recheck ZP operand requirements after layout.
2. Make discovery reject opaque/invalid observations but allow decodable pending
   obligations into the existing trial. Finalization must discharge every
   required obligation before accepting any resident native writer. Report
   owner/block, target, addressing form and failure stage in diagnostics.
3. Pair retained inputs with their executables explicitly: selected input with
   the selected executable's layout, candidate input with the candidate layout,
   and each executable program with its own map. Use stable source identities
   to resolve pre-expansion references; an absent mapping is a failed proof.
   Retain the union of resolved observations for the existing old/new overlap
   checks and retain all logical exposure witnesses. Never resolve every input
   against whichever map happens to be current.
4. Use the same resolver in terminal cleanup. Preserve typed logical exposure
   and concrete physical observations, including portions of indexed ranges
   outside the named object and wrapped pointer bytes. An unresolved obligation
   remains a conservative observer; it cannot authorize deleting a producer or
   store. Replay preservation obligations against current candidate evidence.
5. Keep frame/binding rules, scratch exclusions, trial count, whole-image guard
   and rollback unchanged. A concrete overlap can still reject the combined
   trial; splitting candidates or retrying subsets is outside this plan.

Focused controls must cover both layouts: a changed pointer page, a negative
indexed base whose range reaches a protected byte, overlap only at an old home,
overlap only at a proposed home, a retained input reference removed by cleanup,
and a missing map entry. Check both pointer lanes, boundary wrapping and stale
evidence. Include a disjoint positive placement and an opaque rejection that
preserves independent candidates. Reuse the exact selected-input/executable
rollback test and the affected machine-observation cleanup controls. Commit.

## Slice 4 — execute controls and qualify TN

1. Extend the existing [object-storage fixture](../fixtures/runtime/object_storage.act),
   [pipeline controls](../tests/mir6502_cross_call_homes.rs) and
   [VM group](../tools/vm-runtime-tests/tests/cross_call_homes.rs) with ordinary
   symbolic pointer cells and negative indexed operands. Use small independent
   readers/writers and a dispatch table, not copies of TN's routines.
2. Exercise placement off/on in ActionCart and Standalone. Assert actual selected
   homes, arguments, results, writes, neighboring declared sentinels and stack
   balance. Use deliberate declarations and emitted addresses for physical
   overlap controls; do not guess a hidden home and treat that as a supported
   source program. Normalize host text and exercise LF/CRLF through any changed
   fixture instrumentation; compare binary bytes exactly.
3. Run one [TN facts collection](../tools/compare6502/tn/leaf_facts.py). Record
   SArgs capture separately from executable machine observations, and distinguish
   decoded/pending/resolved/failed observations. Confirm all four recorded causes
   are addressed. Any further rejection must identify its actual operation and
   obligation rather than being folded into a generic unknown-read explanation.
4. Report Sort's discovery, proposal, selected homes and final cost or concrete
   remaining blocker. Compare all routine code/home footprints and complete-image
   size against the recorded baseline. Inspect Ord/Items/FindItem/Copy savings
   and Fnamecmp/Next contracts. Separate representation-only effects from accepted
   placement/cleanup changes using existing trial evidence.
5. Retain MIR, assembly, maps, relocation/image hashes, observation records,
   decisions and work counts under `docs/benchmarks/6502-symbolic-storage-observations/`.
   Extend receipt hashing to changed ABI/observation implementation files. Keep
   large reproducible build artifacts under `build/`. If TN code or homes change,
   run its existing `tn_dispatch` VM target once. Update design and plan status,
   then commit qualification.

Completion requires correct representation, complete resolution of the supported
cases and preserved rejection/rollback behavior. A smaller TN image is measured,
not promised. Report any newly exposed scratch, lifetime or cost limitation as
a follow-up rather than broadening this implementation.

## Validation and execution discipline

Run focused groups during slices, checking that filters select nonzero tests.
Relevant existing targets are `mir6502_call_abi`, the `sargs_` cases in
`mir6502_materialization_gap`, `mads_sargs_listing`, MIR6502 SArgs linkage/override
cases in `actionc_cli`, `mir6502_cross_call_homes`, and the VM SArgs/object-storage
controls. Add proposed `sargs_capture_` and `symbolic_observation_` groups for new
boundaries. Reuse `fixed_observation_`, `object_storage_`,
`private_storage_machine_observations_survive_unknown_effects`, and
`later_trials_keep_the_exact_selected_input_and_executable` where behavior changes.

Batch related cases; do not rerun passing groups without relevant changes.
For final backend qualification, follow AGENTS.md's affected MIR6502 suite
requirement once, including its modes, runtime/ABI and cross-platform coverage.
Do not run other backend suites by default. A shared frontend/NIR contract change
requires a separate slice and the mandated shared-contract checks; none is
planned here. Preserve the separately recorded
[pre-existing failures](benchmarks/6502-labeled-data/final/validation.json)
and distinguish them from regressions.

Reuse `build/checked-arena/shared-target`, dev/test opt-level 1, debug information
off, incremental off and two build jobs. Commit after each completed slice's
relevant checks. Preserve unrelated working-tree edits, avoid concurrent builds
while the TN facts probe installs its temporary hook, and remove that hook after
collection.

## Execution record

Slice 1: typed `SArgsCapture` replaces the generated helper/machine-data pair.
Its verifier and emitter check the target, ABI frame/count and canonical effects;
helper barriers and full-frame address protection remain conservative. Two unit
controls pass, including byte/relocation equivalence with the old representation
at two origins and malformed-frame rejection. All 11 call-ABI tests, three
SArgs final-address tests, all three SArgs listing tests (including the real
LF/CRLF path), and six SArgs CLI linkage/override tests pass. The new VM control
passes in ActionCart and Standalone, checking all five argument bytes, sentinels,
repeated calls and balanced return past the descriptor. `cargo check --lib` and
edited-document link/whitespace checks pass. No TN run or full suite in this slice.

Slice 2: the shared instruction-boundary walker now supports the explicit
observation decoder without changing strict callee-summary operand expansion.
Typed targets retain signed addends, required-ZP encoding and access extents;
invalid backing chains remain failures. Fifteen focused `observation_` unit
controls pass, including the three new symbolic cases, fixed-pointer wrapping,
physical exclusions and existing ownership controls. Fixed alias/addend encoding
matches emitted bytes. A temporary compatibility projection keeps production
placement/cleanup behavior conservative until slice 3 installs layout resolution;
that projection is removed in slice 3. The diagnostic probe records decoded
expressions and counts SArgs capture as a helper call. Local symbolic operands
also receive source-derived execution coverage in slice 4.

Slice 3: placement and cleanup consume the same typed observations and final-map
resolver; the compatibility projection is removed. Source-storage records resolve
assembler data addresses, including descriptor backing and local identities.
Retained input/executable pairs use their own maps, and unresolved final operands
report their owner, block, stage and expression. Eighteen `observation_` unit
controls pass, including local indexed emission agreement, ZP validation, pointer
page changes, missing maps and retained old/new observations. Four existing
`object_storage_` units, the machine-observation cleanup control, exact rollback,
and both object-storage pipeline controls also pass. The pipeline exercises
LF/CRLF and preserves actual positive ZP selection plus opaque/independent fallback.
No extra materialization, emission stage or subset retry was introduced.

Slice 4 extends the independent fixture with symbolic pointer cells, signed
global/local indexed operands and a dispatch table. Three pipeline controls
cover positive selection, opaque fallback and exact rollback for an explicit
`$A0/$A1` conflict. The VM group checks placement off/on in both runtimes,
including declared sentinels and stack balance. The SArgs fixture expectation
now distinguishes typed capture from source machine data.

Final qualification exposed a missing post-home ABI read: wide calls can use
`$AC/$AD` as argument homes despite their pointer-scratch role elsewhere in the
caller. The previous opaque descriptor inventory had masked this dead-store
proof defect. Home liveness and definition-use analysis now read populated call
homes, preserving pre-materialization effects. The grouped unit control checks
both live argument definitions and genuinely dead surrounding stores. All five
ADPCM decoder tests and all six cross-call VM tests pass after correction; both
TN dispatch tests pass too.

The broad runs also exposed existing failures. A detached `96e91785` checkout
was used only for targeted failure comparison, extending the original reuse-only
validation scope. TN size comparison continues to use the recorded baseline
artifact. The final [qualification report](benchmarks/6502-symbolic-storage-observations/README.md)
records exact checks, baseline failures, the corrected ADPCM regression and
compiler work counts.

The final TN collection after the ABI correction retains the exact **9,824-byte**
image: **14 bytes saved**, six from Sort's `h` at `$E2/$E3` and eight from four
staging stores. All 34 explicit machine observations resolve, and all 34 leaf
entry/exit summaries remain unchanged. Materialization and emission counts are
unchanged. Final receipts include the home-liveness implementation; the temporary
diagnostic hook and detached baseline checkout were removed.
