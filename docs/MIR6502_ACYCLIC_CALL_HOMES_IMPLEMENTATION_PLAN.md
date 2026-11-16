# MIR6502 acyclic call homes: implementation plan

Prepared 2026-11-16. Status: slice 1 complete; slices 2–4 pending.

Implement the [acyclic call-chain design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-zp-placement-through-acyclic-call-chains)
in four slices. Commit each slice after its focused checks pass. Preserve
unrelated working-tree changes and keep large measurement artifacts under
ignored `build/`.

Slice 1 adds snapshot-scoped acyclic entry composition and transitive freshness.
Call requirements reuse the existing machine/home liveness solvers. Definite
return bytes require a typed scalar result and actual writes on every returning
path. Three new grouped analysis tests, 18 existing entry/handoff tests and 11
machine-liveness tests pass. Production placement is unchanged.

## Outcome and scope

Allow eligible parameters and locals to keep ZP homes across supported native
call chains such as `Caller -> Ord -> Next`. Use the existing per-object value
lifetimes: a value live across Ord must avoid the complete invocation's writes;
a value dead at that call does not need invocation preservation. Both cases
still require valid argument setup and the existing observation checks.

Include ordinary and native `=*` callees, several callees, diamonds, repeated
sites and loops inside routines. A callee can provide a contract while its own
homes remain inline. Keep `ActionObservable`, direct-entry behavior and the
current policy for a `=*` routine's own storage.

Retain the current ZP pool, mixed inline/ZP bindings, four-byte incoming-argument
limit, caller operation restrictions and source ownership rules. Entry-dependent
or exposed objects stay inline while eligible siblings can move. Do not add
recursive preservation, assembly/OS contracts, helper admission, result-slot
coalescing, save/restore, a new allocator or a frontend/NIR change. This delivery
extends source-home discovery; broader spill discovery remains a follow-up.

The reference [native-current-location qualification](benchmarks/6502-native-current-location-callees/README.md)
records TN at **9,811 packaged bytes**, optimized ActionCart at `$3000`. Sort
already has resident `h` and `gap`; its former Fnamecmp flag blocker is resolved.
Copy's operation guard and PopUp's SArgs/exposure/retained-value restrictions
remain separate. No particular TN saving is assumed.

## Implementation boundaries

| Existing component | Change |
| --- | --- |
| [Shared entry facts](../src/mir6502/analysis/known_callees/entry_inputs.rs) | Compose actual native entries through supported direct calls, with transitive freshness. |
| [Machine liveness](../src/mir6502/analysis/machine_liveness.rs) and [home liveness](../src/mir6502/analysis/home_liveness.rs) | Reuse their solvers with certified call transfers; keep may-writes separate from definitions. |
| [Known-callee summaries](../src/mir6502/analysis/known_callees.rs) | Reuse existing transitive writes and physical-byte preservation queries. |
| [Residence calls](../src/mir6502/parameter_homes/calls.rs) | Add explicit acyclic admission without changing existing leaf-only discovery clients. |
| [Cross-call discovery](../src/mir6502/parameter_homes/cross_call.rs) | Coordinate proposed descendant homes in one callee-before-caller traversal. |
| [Placement transaction](../src/mir6502/parameter_homes.rs) and [preparation](../src/mir6502/preparation.rs) | Reuse one continuation, candidate verification, final cost and exact rollback. |

Add `enable_acyclic_call_homes` to the existing MIR6502 configuration. Initially
leave it off in both configurations. Discovery requires it together with the
peephole, parameter-home, cross-call and value-lifetime gates; the existing
multiple-call gate still controls the number of call sites. With the new gate
off, retain current discovery order, behavior and artifacts. Do not rename the
existing gates as part of this work.

## Slice 1 — compose actual-entry facts

1. Add an opt-in shared analysis context borrowing one immutable `MirProgram`.
   Index direct native dependencies by `RoutineId`, analyze callees before
   callers and memoize success or failure once per routine. Use stable ID order
   between unrelated routines. Detect recursive components and dependencies on
   them; a failed closure is unavailable, never an empty contract. Intraprocedural
   loops continue through the existing CFG solver.
2. Limit the new composition to `ActualEntry`. Preserve the existing leaf and
   bounded-machine query behavior and the entire `PlannedCapture` scope. Accept
   supported ordinary and native `ActionObservable` entries; reject unsupported
   descendants, including indirect/unresolved calls, machine bodies, OS calls,
   SArgs, barriers and helpers outside the current contract.
3. Validate canonical argument width and physical lanes at every nested site,
   even when the target is cached. Factor the existing site-shape check into a
   shared ABI helper if necessary; shared analysis must not depend on a private
   selector approximation. Extra callee register, individual-flag and fixed-byte
   requirements become observations at that site, propagated backwards through
   real setup and all CFG paths. A requirement locally established on every
   relevant path must not become an extra outer entry input.
4. Extend the existing liveness transfer interfaces narrowly. Calls contribute
   their entry observations, but possible writes/clobbers do not kill incoming
   dependencies. Use only certified definite definitions, including guaranteed
   typed ABI result delivery where already available. Otherwise retain the
   dependency or reject the proof. An incidental `$A0` store or a may-write
   summary proves no result definition; do not introduce native exit N/Z facts.
5. Keep caches local to the borrowed program. If composed `MirCalleeEntryFact`
   values leave that context, retain shared dependency witnesses and make
   `is_current` check them transitively. A changed grandchild body, binding, ABI
   or backing must invalidate the outer fact even when the outer body is equal.
   Never share these facts between selected and candidate snapshots.

Focused checks: one grouped entry-analysis regression covering a three-level
chain and diamond; ordinary/`=*` middle entries; fixed-byte and carry inputs
established on all paths versus one; may-write versus definite/result definition;
an invalid later call to a cached target; recursion/unsupported descendants; and
stale grandchild facts. Retain actual-versus-planned and bounded-machine controls.
Run the affected entry/handoff tests and machine-liveness tests if their transfer
interface changes. Selection remains unchanged in this slice.

Exit: composed facts have correct requirements and provenance, without widening
production placement. Commit the analysis and focused tests.

## Slice 2 — certify residence through the chain

Primary code: residence calls, [lifetime verification](../src/mir6502/parameter_homes/lifetimes.rs),
[MIR verification](../src/mir6502/verify.rs) and
[invocation certificates](../src/mir6502/parameter_homes/observations.rs).

1. Give residence queries an explicit leaf-only versus acyclic discovery scope.
   In the new scope, replace the blanket nonleaf veto with slice 1's supported
   actual-entry closure. Validate the outer call site's inputs as before and
   obtain writes from `MirKnownCalleeSummaries::analyze_for_rewriting`.
2. Preserve the distinct dead/live boundary policies. An ordinary dead boundary
   retains its current fallback when a composed fact is unavailable, including
   the existing recursive-call controls. A live boundary requires both complete
   entry facts and known writes. Native `=*` calls require complete actual-entry
   facts at dead as well as live boundaries. Unsupported preservation excludes
   the affected object's placement, not every otherwise eligible caller object.
3. Check that transitive physical exclusions include capture, scratch, nested
   arguments/results, allocated virtual storage and descendant writes, including
   writes restored before return. Resolve each bound object in its owning
   routine's frame. Reuse `preserves_fixed_byte`; keep the conservative Global
   alias rule and Unknown read footprints unchanged.
4. Separate permission to discover new placements from verification of supplied
   bindings. The MIR verifier must validate acyclic bindings from their actual
   program, independently of whether discovery is enabled. Use the same contract
   in expanded lifetime checks and invocation certificates; rebuild facts after
   expansion and for every changed terminal snapshot. Preserve the existing
   ordinary dead-boundary policy during verification too.
5. Audit all `ResidenceCalls` consumers. Keep
   [spill discovery](../src/mir6502/parameter_homes/spills.rs) explicitly leaf-only
   and terminal handoff at its existing scope. Any extended cleanup certificate
   must be explicitly requested by the new path; do not silently widen the
   default constructor and change gate-off cleanup. Retain source exposure,
   entry-dependence and physical-observation checks.

Focused checks: manually bound parent/child homes across three levels, a
grandchild clobber of either word lane, fixed/absolute aliases, an inline middle
callee, native `=*` middle entries and unknown-write fallback. Include ordinary
dead/defined-after versus live values and unchanged spill/handoff controls. Run
the affected parameter-home unit tests and relevant ABI/ownership-certificate
regressions. Production nested discovery is still disabled.

Exit: actual acyclic bindings can be proved or rejected through the shared
contracts, while existing discovery permissions remain intact. Commit separately.

## Slice 3 — coordinate one placement proposal

1. Wire the new gate into `cross_call::propose`. When enabled, process supported
   dependencies callee before caller, with deterministic `RoutineId` ordering.
   Join retained input and selected executable by IDs. Compute object lifetimes
   and sites in the retained routine, and callee entry/write facts from the
   selected actual executable; never reuse operation indices across these
   representations. Account conservatively for retained call edges that can
   reappear on resumption. Unsupported closures must not remove existing
   ordinary dead-call candidates.
2. Maintain a small proposed-resident-byte set per routine, propagated upwards
   through all relevant descendants, including intermediate routines that receive
   no new homes. Keep already selected bindings fixed. Add only eligible remaining
   inline objects through the existing parameter/local binding functions.
3. For each object and call site, reuse `LogicalHomes::live_after`. If either
   lane of a word survives, exclude both the selected callee's full transitive
   writes and its proposed subtree's resident bytes from the whole word home.
   Keep selected writes even when the proposal may remove some. At a dead call,
   omit invocation-preservation exclusions but retain current return-slot,
   argument-setup, source-owned, scratch and observation restrictions.
4. Permit sibling reuse and ancestor/descendant sharing at proved dead boundaries.
   Keep one authoritative home per object and disjoint source homes within a
   routine. Proposed byte sets are planning constraints, not published effects
   or final preservation evidence.
5. Merge these candidates into the existing ordinary-call batch. Reuse
   `try_proposal` and the single core continuation; no per-depth compilation,
   second batch, subset retry or placement fixed point. After expansion, rebuild
   contracts and verify every resident caller, including callers whose bindings
   did not change. Include any newly selected spills and scratch in this proof.
6. Retain final physical alias checks, preservation replay and strict reduction
   of the complete packaged image. Individual routine growth is diagnostic only.
   Unknown/equal/larger cost or any failed proof restores the entire selected
   artifact entering this trial. Extend existing trace events with the blocking
   dependency/site/object and proposed descendant exclusions; keep counters for
   zero or one `cross_call_home_trials`.

Focused checks: one grouped discovery control with parent and child both proposed,
a diamond/sibling reuse, mixed eligible/exposed/entry-dependent objects, a
loop-carried value and repeated target with only one crossed site. Assert actual
dead-boundary ancestor/descendant sharing and live-boundary separation. Add an
expansion-time conflict that invalidates an unchanged caller and checks complete
rollback. Cover setup/result conflicts, empty proposals and parent gates.

Run changed placement tests and the cross-call integration target. Preserve the
existing leaf-only nested-call negative case with the new gate off and add its
positive gate-on counterpart. Check gate-off artifact equality, whole-image
acceptance and one-trial behavior. Keep the new default off pending slice 4.

Exit: a real nested-call control selects homes and strictly reduces packaged
bytes through one accepted transaction. Commit the selector and focused tests.

## Slice 4 — execute, measure and qualify the default

Add one grouped `acyclic_call_homes` execution control to the existing
[cross-call VM target](../tools/vm-runtime-tests/tests/cross_call_homes.rs), using
a small fixture under `fixtures/runtime/`. Keep calls real by disabling inlining
in the isolated placement comparison. Use supported fixed-address local aliases
for output/sentinels where needed so the control does not accidentally measure
the unrelated all-ZP Global-write exclusion.

Check live bytes and words/pointers across nested calls, alternating subtrees,
byte/page boundaries, received arguments, host-computed results, unrelated
sentinels and stack balance. Exercise optimized ActionCart with just the new
gate off/on, a default ActionCart control and a small Standalone control. Load
both LF and CRLF fixture variants through the real parser/instrumentation path;
normalize host text before newline-sensitive edits and preserve binary bytes.
Reuse the existing VM harness; do not create a broad Cartesian test matrix.

After focused qualification, enable the gate in `Mir6502Config::optimized()`;
leave the conservative default off. Check optimized-default equality with
explicit enablement and all parent gates. Run only newly affected default/gate
regressions after this toggle, not every previously passing suite.

Make one final TN gate-off/on comparison with identical compiler, sources,
runtime, origin and other configuration. Use ActionCart at `$3000`; record any
drift from the 9,811-byte reference separately. Retain Ord, Items, FindItem,
Strcpy, Instr, Fnamecmp, Next, Sort and PrintNonzeroDigit as controls. Inspect
callers of Ord and other supported nonleaf routines, plus the remaining blockers
in larger routines; avoid adjusting TN source to force a result.

Save a compact receipt under `docs/benchmarks/6502-acyclic-call-homes/` containing:

- Compiler/source/configuration identity and artifact hashes.
- Independent control size saving, selected homes and execution results.
- TN packaged size and every changed routine's code/inline-storage footprint;
  report ZP occupancy separately, including shared addresses.
- Concrete blockers for promising routines still inline: body, entry, writes,
  exposure/lifetime, capacity or final cost.
- Placement trials, materializations, emissions, inner probes and preservation
  replays, plus one compile-time sample per A/B build; no repeated timing study.

An unchanged TN image is acceptable if the independent nested control executes
correctly and saves final bytes. A failed safety or profitability qualification
leaves the gate off with its actual blocker recorded. Update the design and this
plan to match the implemented scope and commit the tests, default and receipt.

## Local validation budget

Use filtered MIR6502 library tests during slices 1–3, the affected cross-call
integration target once the selector changes, and the new VM group in slice 4.
Reuse passing coverage; broaden only for changed consumers or observed failures.
No full compiler suite, NIR sweep, other-backend suite or repeated TN compilation
is planned for this backend analysis/selection change. Substantial emission or
shared frontend/NIR contract changes would require revisiting this scope and
running the repository's corresponding mandatory checks.

This planning change itself requires content, relative-link and whitespace
checks only; it changes no executable behavior.
