# MIR6502 parameter-to-argument handoff implementation plan

Prepared 2026-11-14. Status: slices 0–2 complete; slices 3–6 pending.
Starting compiler: `471ef1c8`, with qualified leaf parameter placement enabled.

Implements the [parameter handoff design extension](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-parameter-handoff-to-call-arguments).
The [leaf placement plan](MIR6502_PARAMETER_HOME_SELECTION_IMPLEMENTATION_PLAN.md)
is complete and remains the baseline. Execute the slices below in order,
committing each after its checks pass. Keep unrelated working-tree changes
outside those commits.

## Objective and fixed scope

Use one parameter placement and availability model from entry capture through
outgoing call setup. Retain bytes already in useful registers or ABI-window
storage, and emit only the transfers needed to establish the callee's inputs.
The motivating target is compiler-generated TN `Strcpy` at 20 code bytes with
no ordinary parameter storage, versus the current 41 code + 4 storage bytes.
Independent wrappers must demonstrate the same general behavior.

Initial candidates are ordinary Action wrappers with read-only, unexposed
BYTE/WORD parameters occupying at most four incoming bytes. Admit a straight-line
setup region followed by one known direct call and a void return; outgoing
canonical arguments occupy at most five bytes. The call consumes the last uses
of all relocated parameters. Existing tail-call selection decides whether the
final call/return becomes a jump.

Keep loops/joins in the wrapper, mutable parameters, earlier calls/helpers,
setup indirect writes, opaque barriers, wrapper machine blocks, indirect or
variadic calls, aggregates, SArgs, observable wrapper entry conventions, and
post-call parameter/result uses outside this scope. Existing leaf support is
unchanged. A callee may contain assembly, loops, or indirect writes if its entry
contract supplies the required proofs; do not apply the wrapper restrictions
to the callee's implementation. `MovePage` must qualify through shared facts.

No SemIR/NIR contract or public ABI change is intended. No special cases for
routine names, TN addresses, or particular byte values. This work adds neither
general residence across calls nor arbitrary live-range splitting.

## Code boundaries

| Responsibility | Existing code to extend |
| --- | --- |
| Physical lane vocabulary and canonical argument positions | [ir.rs](../src/mir6502/ir.rs), [abi.rs](../src/mir6502/abi.rs), [call_plan.rs](../src/mir6502/call_plan.rs) |
| Placement policy, reservations, and lifetime proof | [parameter_homes.rs](../src/mir6502/parameter_homes.rs), [zero_page.rs](../src/mir6502/parameter_homes/zero_page.rs), [lifetimes.rs](../src/mir6502/parameter_homes/lifetimes.rs) |
| Selected input, continuation, and exact fallback | [preparation.rs](../src/mir6502/preparation.rs), [mod.rs](../src/mir6502/mod.rs), [materialize.rs](../src/mir6502/materialize.rs) |
| Entry capture, parameter copies, and argument setup | [materialize/abi.rs](../src/mir6502/materialize/abi.rs), [param_availability.rs](../src/mir6502/analysis/param_availability.rs), [materialize/calls.rs](../src/mir6502/materialize/calls.rs) |
| Definition identity, physical availability, and liveness | Existing [rewrite contexts](../src/mir6502/rewrite/context.rs), [machine values](../src/mir6502/analysis/machine_values.rs), [home liveness](../src/mir6502/analysis/home_liveness.rs), and [machine liveness](../src/mir6502/analysis/machine_liveness.rs) |
| Additional callee entry observations and wrapper preservation | [known_callees.rs](../src/mir6502/analysis/known_callees.rs), [entry_flags.rs](../src/mir6502/analysis/entry_flags.rs), [preservation.rs](../src/mir6502/late_cleanup/preservation.rs) |
| Address consumers, layout, verification, and diagnostics | [indexes.rs](../src/mir6502/materialize/indexes.rs), [layout.rs](../src/mir6502/materialize/layout.rs), [emit.rs](../src/mir6502/emit.rs), [verify.rs](../src/mir6502/verify.rs), [printer.rs](../src/mir6502/printer.rs), [trace/homes.rs](../src/mir6502/trace/homes.rs) |

Put focused wrapper candidate/plan code under the existing parameter-home
module. Reuse shared analysis and call materialization; do not add a second
ABI mapper, register-content tracker, alias scanner, or late home-remapping pass.

## Slice 0 — freeze the leaf-qualified baseline and independent controls

Add compact runtime fixtures for direct BYTE/WORD forwarding, a computed fifth
byte, and an independent indirect-byte producer. Retain the wrappers through
real callers; disable inlining only for isolated probes, and also measure
normal optimized callers. Add negative controls for post-call uses (`Strcat`
included), parameter mutation, extra calls, source ZP ownership, exposure, and
extra machine-callee entry observations.

Add a focused root integration target and VM target named
`parameter_argument_handoff` (root target `mir6502_parameter_argument_handoff`).
Reuse the existing VM call/entry observation support. Baseline execution must
check canonical arguments before callee capture, public memory, and stack
balance, independently of agreement between two compiler configurations.

Extend the parameter-home measurement harness with explicit configuration for
this scope when its gate is introduced. Publish receipts under
`docs/benchmarks/6502-parameter-handoff/`; keep executables, listings, and large
traces under ignored `build/`. Record source hashes including TN's LIB.ACT,
compiler revision/diff/library hash, configuration, commands, routine sizes,
ordinary homes, ZP usage, image hashes, and compiler work counts.

Recheck these starting measurements rather than copying expectations blindly:

| Subject | Current leaf-qualified baseline |
| --- | --- |
| TN, cart / standalone | 9,878 / 9,985 packaged bytes |
| Fnamecmp | 105 code + 0 ordinary parameter bytes |
| Strcpy | 41 code + 4 ordinary parameter bytes |
| Strcat | 113 code + 4 ordinary parameter bytes |

Use both runtimes and origins `$3000` / `$41C7`. Preserve the existing leaf
receipts as independent nonregression controls. Treat the handwritten 20-byte
wrapper as a reference, never as compiler-generated evidence.

**Checks and exit:** the new baseline integration/VM targets pass; LF and CRLF
inputs exercise the actual source-loading path. Record any baseline difference
with its cause. Commit fixtures, harness support, and frozen receipts before
changing compiler behavior.

## Slice 1 — retain the selected leaf input and add a bounded extension gate

Extend the selected placement artifact to own both its finalized executable
and the exact pre-capture input that produced it. Preserve the original selected
core configuration, helper requirements, upstream obligations, and resume data.
For an accepted leaf proposal, retain its complete input, including absent-home
store removal and fixed reservations; copying only its binding table is not
sufficient. Rejected proposals retain the prior input and executable together.

Refactor the current selector's early returns so that no leaf candidate or a
rejected leaf trial still leaves a usable baseline for wrapper discovery. Never
turn a rejected proposal into the next trial's input. Share retained immutable
state where practical rather than repeatedly cloning whole programs.

Add `enable_parameter_argument_handoff`, initially false in both conservative
and optimized configurations, subordinate to `enable_peepholes` and
`enable_parameter_home_selection`. Keep the existing leaf default unchanged.
The new stage accepts at most one wrapper batch after current leaf selection.
Reuse the existing continuation and final output; do not recursively invoke
the upstream optimizer or the leaf selector inside the trial.

**Checks and exit:** gate-off artifacts and work counts match slice 0. Test leaf
accept/reject/no-candidate paths followed by wrapper no-op, failure, equal cost,
and growth using bounded test proposals. Wrapper rejection restores the exact
leaf-qualified image, maps, relocations, costs, and trace selection. Trace-on/off
agree. No candidate adds no materialization; a trial adds at most one relative
to today's optimized pipeline, with unchanged upstream choices/counts. Run
preparation/parameter-home unit tests and phase-trace/layout integration targets.
Commit the continuation ownership change with production candidates disabled.

## Slice 2 — mixed parameter lanes through the complete storage boundary

Add a typed mixed-lane working binding capable of representing
`[Zp($A0), Reg(X)]`. Define one checked normalization used by parameter bindings
and flattened call-argument requirements. Retain routine-qualified ParamId,
byte offsets, widths, and low/high order. Uniform register/ZP bindings must
resolve identically to their previous implementation.

Teach capture generation, parameter availability, scalar consumers, reservation
queries, layout, effects, printer, and emitter to consume the same mapping.
Seed retained register lanes from the canonical entry ABI; memory lanes become
available only after their actual capture or a valid incoming fixed-slot fact.
Keep one working location per lane and use existing availability for extra
copies. Do not fabricate a capture store to teach the analysis a register fact.

Resolve mixed words lane by lane. Register lanes have no address; a complete
pointer dereference requires a coherent pair. Reserve only actual memory lanes.
Report real memory fragments without assigning a fictitious contiguous word
address. Prefer existing map facilities; any necessary shared map/API change
must inventory its consumers and broaden validation accordingly.

Tighten verification for incomplete/duplicate mappings, wrong widths/order,
register-address operands, conflicting legacy metadata, unreserved or
source-owned bytes, overlapping live owners, and unsupported whole-word
address consumers. Keep existing parameter-remapping passes off bound frames.
Update the pseudo-machine contract and design with the exact representation.

**Checks and exit:** forced mixed bindings in small call-free routines survive
materialization and emission, with independently checked values and maps.
Malformed bindings fail at the appropriate phase. Existing uniform bindings
and feature-off outputs stay identical. Run affected binding/layout/effect/unit
tests and parameter-home, ABI, materialization, and pointer-setup integration
targets, including their cross-platform fixture paths. Commit this vertical
representation slice with wrapper selection still disabled.

## Slice 3 — prove terminal call consumption and callee entry obligations

Add a bounded typed wrapper fact using existing CFG/use-def and availability
contexts. Identify the unique direct call, original source-read order, complete
parameter lane definitions, outgoing ABI destinations, and last uses. Apply a
named setup-operation budget, reusing the existing 64-operation staging bound
where applicable. Unsupported shape or budget exhaustion produces a reason
and fallback, not a compilation error.

Extend lifetime verification to distinguish setup, call inputs, and invocation
clobbers. Calls read their arguments before their effects invalidate locations.
A physical scratch/outgoing write cannot kill its own parameter safety proof.
Reject any protected value needed after the call. Keep wrapper bodies without
this explicit proof under the existing conservative call rejection.

Audit shared callee-entry queries for extra registers, flags, and fixed-memory
reads before overwrite. If an obligation is missing, add the smallest general
fact to the shared analysis with independent tests. Machine callees, including
MovePage-shaped loops, must qualify without procedure-name matching or requiring
an exact bound on every destination write. Unknown entry observations still
block selection even when all formal parameters die at the call.

Record analysis generation/site ownership. Reconstruct proofs after expansion
and transformations that change sites or definitions; an old instruction index
is not a stable call identity. Preserve the call target, public effects, stack
behavior, and wrapper contracts consumed by optimized callers.

**Checks and exit:** focused analysis/verifier tests distinguish a call's last
argument read from a clobber, a post-call use, and a setup overwrite. Cover extra
entry flags/fixed bytes, a callee overwriting a slot before use, mixed lanes,
stale plans, unknown callees, and source exposure. Run affected entry-summary
and preservation tests as well as the new target. No production wrapper is
selected yet. Commit the proof boundary.

## Slice 4 — select direct forwarding wrappers and share call transfers

Build one deterministic placement proposal from the typed facts. Start with
forwarded scalar parameters, constants, and already-supported native direct
computations; defer indirect producers to slice 5. Prefer surviving incoming
registers/fixed lanes, then reserved ZP for lanes whose setup requires a save.
Preserve all previously accepted leaf bindings.

Integrate the plan into existing entry and call materialization. Omit a transfer
only when the required value definition is already available in its destination.
Schedule required transfers using existing dependency/overlap handling. Cover
changed argument positions and repeated arguments; retain safe staging or reject
unsupported cycles. Do not reorder observable producers or globally change
argument evaluation order. Expanded effects must prove every retained lane
survives until its last outgoing use.

Keep `Call; Return` and the existing tail-call conversion intact. A later generic
staging path must not recreate removed homes or ignore reservations. Rebuild
candidate summaries and replay consumed wrapper contracts. Selection stays
opt-in through the new gate and uses final code/home/image acceptance.

**Checks and exit:** independent wrappers gain final bytes with correct entry
arguments, memory results, and stack behavior in both runtimes/origins. Negative
shapes retain exact fallback. Test gate combinations and normal optimized
callers, not just forced bindings. Run the new integration/VM targets plus
affected argument staging, A-to-X forwarding/bitwise, callee-entry, ABI, and
parameter-home consumers. Keep strict per-pass oracles isolated and add combined
mode coverage where new placement legitimately changes private scratch traffic.
Commit the first productive wrapper slice.

## Slice 5 — resident pointer producer and TN Strcpy

Admit the existing indirect-byte consumer in a straight-line setup when its
pointer can reside in a coherent pair throughout the read. Use the same binding
resolution in pointer preparation, argument-expression selection, effects,
scratch allocation, and final proof. Preserve the source read exactly; its
pointer's current location does not authorize eliminating or delaying it.

For the general two-word-forwarding plus byte-producer shape, the placement
can retain the first word's high byte in X, capture its low byte in ZP, keep the
second word coherent in `$A2/$A3`, and compute the fifth byte into `$A4`. Respect
every explicit source reservation and uncaptured input. Check actual expansion
for A/X/Y and carry use; reject any schedule requiring unsupported spills or
overwriting a live pointer component.

Retain better existing pointer/arithmetic strategies and all prior leaf savings.
Do not relax the data-dependent pointer-arithmetic or indexed-load exclusions
for unrelated routines. Strcat remains a post-call-lifetime fallback, and Key,
Next, Fnamecmp, and independent leaf controls must not regress.

**Checks and exit:** compile the unchanged TN source and independent renamed/
reordered controls through the full pipeline. Target Strcpy at 20 code + 0
ordinary parameter bytes; account for every remaining instruction if the target
is not reached, and keep the unmet target explicit rather than substituting
handwritten output. Require actual compiler savings in both TN and controls.
VM checks observe canonical arguments at callee entry, all 256 length bytes,
carry/wrap boundaries, different pointer low bytes, page crossings, identical
and overlapping buffers, ordered public reads/writes, and stack balance. Run
actual LF/CRLF paths and affected pointer/call/preservation targets. Commit the
indirect-producer slice with measured receipts.

## Slice 6 — full MIR6502 qualification and default decision

Measure with leaf selection enabled in both configurations and handoff toggled
alone. Repeat the independent leaf controls and full TN matrix with normal
inlining; add isolated controls with inlining disabled. Compare final artifacts,
maps/relocations, routine code and ordinary homes, actual reserved ZP bytes,
source/image hashes, and trace-on/off equality. Inspect accepted and rejected
trial traces so speculative output cannot be reported as selected output.

Prove at most one additional core continuation over the current optimized path
and no additional upstream search. Report all downstream emissions, preservation
replays, elapsed compilation time, and peak memory. The previous leaf feature
already added substantial compilation work; a bounded trial is not a claim of
negligible overhead. No-candidate builds must avoid the extra continuation.

Run the complete affected MIR6502 unit and integration scope, including CLI
configuration, MADS reassembly, storage/maps, phase tracing, and required
cross-platform fixture paths. Run relevant VM targets for parameters, call
staging/forwarding, pointer consumers, entry flags, preservation, terminal
cleanup, and TN reproducers alongside the new wrapper target. Reuse successful
earlier runs when no subsequent behavior change affects them; save exact target
names, commands, outcomes, and the reason for any focused rerun.

Enable the scope in optimized configuration only after correctness, size,
nonregression, and compilation-cost qualification. Conservative configuration
remains off and the master/parameter gates continue to dominate. Record an
explicit opt-in policy if overhead or unresolved coverage prevents enablement;
do not describe an unqualified default change as complete. Update the design,
phase tracing documentation where needed, and final benchmark receipts.

**Checks and exit:** final accepted images are smaller with no individual routine
growth relative to the leaf-qualified baseline. Rejected trials retain the
complete baseline, including TN's existing 84-byte leaf saving. All required
backend checks pass, deviations from the 20-byte reference are explained, and
the default decision and measured overhead are documented. Commit qualification
and policy separately from implementation.

## Validation discipline and completion record

Use the existing shared build directory and low-debug build configuration;
avoid duplicate target trees and unbounded saved traces. During development,
run the changed analysis/materialization filters and explicit affected targets.
Final qualification broadens within MIR6502, not automatically to 65816/68k.
If implementation changes shared frontend/NIR contracts or shared output APIs,
apply AGENTS.md's corresponding shared-contract/consumer checks instead.

Host-text normalization belongs before newline-sensitive instrumentation.
Exercise LF and CRLF through actual loading or an isolated CRLF checkout for
embedded fixtures; never normalize binary or ATASCII outputs to make them agree.
Keep semantic and ordered-access oracles independent of incidental private
scratch traffic, while retaining isolated exact-traffic tests for their passes.

For each slice, record its commit, tests and scope, final measurements, and any
remaining restriction. Completion requires one authoritative mixed-lane mapping,
proved entry-to-call consumption, strict final-cost acceptance, exact rollback
to existing leaf output, independent positive/fallback coverage, and a reported
compiler-generated Strcpy result. This plan does not authorize separate work
on general cross-call allocation, shared Y induction, or arbitrary call-result
forwarding.

### Slice 0 completion

The frozen [baseline receipt](benchmarks/6502-parameter-handoff/baseline.json)
records the current leaf-qualified compiler and independent forwarding/copy
controls. The root integration target passes two tests; the VM target passes
its entry, ordered-memory, and stack oracle across both runtimes, both origins,
and actual LF/CRLF source loading. Cross-newline executable/storage comparison
intentionally excludes source spans, whose byte offsets differ with CRLF.
Tracing versus untraced compilation still compares the entire output.
Compiler source is unchanged in this slice.

### Slice 1 completion

Slice 0 is `c3307cf7`. Selected placement now retains its exact input alongside
its executable; continuation state is borrowed and immutable. The extension gate
is off in both configurations and subordinate to the existing two gates.
Production discovery remains disabled pending the lane/consumption proof slices.

Validation: 12 parameter-home and 3 preparation unit tests, plus 14 integration
checks across handoff, phase tracing and layout census pass. Forced second trials
cover unchanged/equal-cost, invalid and growing proposals after original,
accepted-leaf and rejected-leaf selection; rollback retains input identity,
program, full output, costs and trace selection. No-candidate gate combinations
add no work. The [gate-off receipt](benchmarks/6502-parameter-handoff/slice1-off.json)
matches all 12 baseline image hashes, routine sizes, work counts and upstream
decisions. The initial unit attempt used an overbroad assertion that also banned
required downstream continuation work; the corrected upstream-only assertion and
all artifact assertions pass. No compiler behavior changed to satisfy that test.

### Slice 2 completion

Slice 1 is `31916958`. Mixed physical lanes now cross capture, availability,
planning layout, pointer/effect projection, emission, ownership, allocation
certificates and diagnostics. Register lanes have no address; maps publish only
real memory fragments. Canonical argument and parameter normalization share the
same lane vocabulary. Verification rejects malformed/reservation-conflicting
bindings and memory accesses spanning non-memory lanes. The public map API and
SemIR/NIR contracts are unchanged; production wrapper selection remains disabled.

Validation: 75 focused unit cases pass across parameter homes, availability,
layout, ABI/binding consumers and rewrite drivers. The eight affected integration
targets pass 100 tests (call ABI, materialization gap, parameter homes/pointers,
handoff, phase trace, layout census and pointer setup). Mixed bindings execute
2,048 independently checked word cases across both runtimes/origins and LF/CRLF
inputs; clobber, malformed-lane, source overlap and memory-fragment checks pass.
A dead constant clobber was legitimately optimized away, so that negative oracle
now uses an observable hardware read. Two older synthetic pointer-effect frames
were supplied with the declarations/reservations required by checked normalization.
These are test-fixture corrections, not a weaker production proof.

The [slice 2 receipt](benchmarks/6502-parameter-handoff/slice2-off.json) matches all
12 baseline image hashes, routine sizes, upstream choices and work counts.
Checks used the existing shared opt-level-1, no-debug build directory. A filter
for `materialize::abi` matched no tests; the explicit ABI/binding-consumer cases
and call-ABI integration target provide that coverage instead. The final unit
rerun changed only those synthetic test frames; passing production checks were
retained rather than rebuilding the measurement matrix again.
