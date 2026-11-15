# MIR6502 unexposed object storage: implementation plan

Prepared 2026-11-15. Status: slices 1–2 complete; continue with slices 3–4.
Implements the [design](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md).

Complete the four slices in order and commit after each slice's focused checks
pass. This plan does not implement the separate acyclic-call-chain extension.
Preserve unrelated working-tree edits and historical qualification artifacts.

Slice 1 validation: the grouped address-control regression, five existing local
binding/lifetime tests, fifteen capture/handoff tests and five scoped-observation
tests passed. The shared borrowed inventory is reused across each selector or
verifier batch. Source roles and indirect-write admission remain unchanged.

Slice 2 validation: both grouped object-storage tests, five existing local
binding/lifetime controls and five scoped-observation controls passed. Native
byte/word stores coexist with explicit resident bindings; both live lanes,
stale facts, numeric aliases and persistent locals remain protected. Public
pointer-write summaries remain Unknown. Final emission now retains its physical
inventory for placement validation; no additional emission was introduced.

## Outcome and baseline

Eligible unexposed source locals/parameters have one address-control fact before
placement. Ordinary native pointer writes no longer reject every home in their
routine. Logical initialization and physical scratch/call conflicts remain
independent requirements. No blanket weakening of Unknown effects is permitted.

Use `7ea9e7ab` and the existing
[final callee-facts evidence](benchmarks/6502-callee-facts-repair/final/facts.compact.json)
as the measurement baseline. Optimized MIR6502 TN, ActionCart at `$3000`, is
**9,846 bytes**. Its XEX SHA-256 is
`2419293a359d4b20b4d8af576b5b7efc7d2b84a0e4f682e1ef137679d92167ae`.
Sort has 250 code bytes and three mapped ordinary-home bytes; Fnamecmp's call
contract passes. Reuse these artifacts without rebuilding the old compiler.

## Slice 1 — shared address-control facts, independent of binding

Primary code: [storage_ownership.rs](../src/mir6502/analysis/storage_ownership.rs),
[parameter_homes.rs](../src/mir6502/parameter_homes.rs),
[locals.rs](../src/mir6502/parameter_homes/locals.rs), and
[observations.rs](../src/mir6502/parameter_homes/observations.rs).

1. Extend shared ownership/exposure analysis with the design's object-specific
   address-control query. Retain source roles and declaration IDs; do not
   reclassify Param/Local as Temporary or infer privacy from a ZP binding.
2. Consolidate existing object-exposure checks behind it. Include address uses
   in executable operations/terminators, data initializers and relocations,
   aliases and existing machine observations. Preserve conservative unknown
   or undecodable exposure and the restrictions from retained input programs.
3. Separate logical address control from layout-dependent physical observations.
   Reuse the existing complete declaration/observation inventories. Keep pending
   numeric overlap checks for baseline and candidate layout; do not erase a
   source range to make a candidate qualify.
4. Have placement and InvocationHomes consume the shared exposure authority.
   Keep lifetime, ABI and physical-safety checks independent, and keep current
   source-ZP exclusions. Build/reuse facts per immutable snapshot, not per object.
5. Keep the indirect-write admission guards closed in this slice. Document the
   query's limited permission in the ownership and placement contracts. Where
   incomplete evidence is discovered, retain fallback rather than guessing.

Focused checks: one grouped unit regression for the same object's fact while
inline and ZP-bound; another routine with the same LocalId; address-taking versus
passing a pointer value; local aliases, fixed/ABI storage, data/machine exposure,
numeric observation obligations, historical exposure and stale evidence.
Run the existing affected exposure/capture controls only where their path changed.

Exit: a binding cannot manufacture address control, explicit observations remain
protected, and the selector still preserves its pre-slice admission scope.
Commit the analysis consolidation and contract tests.

## Slice 2 — verify pointee effects separately from physical clobbers

Primary code: [effects.rs](../src/mir6502/analysis/effects.rs),
[lifetimes.rs](../src/mir6502/parameter_homes/lifetimes.rs),
[locals.rs](../src/mir6502/parameter_homes/locals.rs),
[observations.rs](../src/mir6502/parameter_homes/observations.rs),
[verify.rs](../src/mir6502/verify.rs), and the existing physical observation
queries in [late_cleanup](../src/mir6502/late_cleanup/mod.rs).

1. Add the shared contextual test for whether an ordinary native pointee
   read/write can access the candidate object. Reuse typed operation effects;
   do not add a second general memory-summary system or change Unknown globally.
2. Let residence verification consume the current address-control evidence and
   distinguish pointee writes from explicit home, address-pair and scratch
   writes. Every operation's hidden resource effects must remain accounted for.
   Unsupported/opaque effects and unknown call contracts still reject residence.
3. Preserve logical byte-definition liveness. An indirect write or a callee
   may-clobber cannot initialize a source local or kill its live-range proof.
   Check both lanes of words, pointer setup and live fixed-slot conflicts.
4. Apply the same rules to manually supplied bindings and automatic proposals.
   Update InvocationHomes to combine shared exposure with current lifetime/ABI
   proofs without recursive dependence on its own cleanup certificate.
5. Reuse final physical observation checks for explicit aliases in the old and
   proposed layouts, including retained witnesses. Missing placement or an
   unresolved explicit observation cannot certify a home. Keep the scoped
   cleanup permission, storage inventory and pruning/movement exclusions intact.

Focused checks: a grouped verifier test using explicit inline/ZP bindings beside
native byte/word indexed reads/writes, with positive disjoint scratch and negative
low/high-lane conflicts. Include opaque effects, unknown callee writes, partial
or persistent local initialization, explicit physical observations and stale
facts. Run the directly affected local-lifetime and scoped-observation controls.
The selector remains closed to new candidates until slice 3.

Exit: a legitimate native pointee write can coexist with a qualified resident
object; a real scratch/call/physical alias still rejects it. No public Exact
write summary is synthesized from this object-specific result. Commit separately.

## Slice 3 — admit native indexed bodies and constant byte shifts

Primary code: [parameter_homes.rs](../src/mir6502/parameter_homes.rs),
[cross_call.rs](../src/mir6502/parameter_homes/cross_call.rs),
[zero_page.rs](../src/mir6502/parameter_homes/zero_page.rs), and existing
materialization/effect consumers where a concrete missing resource fact is found.

1. Replace the caller's blanket indirect-write rejection with the shared object
   and operation requirements. Admit native byte/word indexed reads and writes
   whose expanded scratch effects satisfy slice 2. An indexed write must not
   disqualify every otherwise eligible scalar local merely by being present.
2. Admit constant BYTE Lsh/Rsh counts 0 through 8. Preserve zero-count flag
   behavior and carry dependencies; do not publish new definite flag definitions.
   Keep variable shifts, helper-producing arithmetic and unsupported word-shift
   expansions outside this slice. Inspect actual retained MIR forms, not source
   names, when deciding which addressing recipes require support.
3. Preserve source-ZP exclusion, complete home bindings, reservations and the
   existing ordinary-call entry/write requirements. Record eligibility, address
   exposure, lifetime, resource, replay and cost rejection distinctly through
   existing trace events. A passed independent call check is not selected placement.
4. Add candidates to the current combined proposal, using the selected input
   and existing core continuation. Keep strict complete-image acceptance, routine
   inventory compatibility and exact rollback. Add no retry batch, per-depth
   search, per-object materialization or new optimization switch.
5. Run final candidate verification and preservation replay through the normal
   path. A lost fact or failed physical obligation rejects the proposal; do not
   weaken the proof to recover a TN saving.

Focused checks: one grouped whole-pipeline case in
[mir6502_cross_call_homes.rs](../tests/mir6502_cross_call_homes.rs) for actual
selection, source-ZP avoidance, bounded trial count and inline fallback.
Extend the existing [cross_call_homes VM target](../tools/vm-runtime-tests/tests/cross_call_homes.rs)
with one small group using native indexed byte/word writes and a live scalar
across an ordinary leaf call. Include an array-swap/constant-shift shape, a page
crossing and repeated invocations. Check expected values, array contents,
sentinels, call inputs and stack balance with placement disabled/enabled for
ActionCart and Standalone. Ensure the positive case actually exercises a selected
ZP home; do not count a passing fallback as coverage of the new path.

Use focused negative cases for exposed objects, real scratch collisions,
unknown/unsupported calls and persistence. Reuse existing tests for boundaries
already exercised; do not replicate their full configuration matrix.
Commit admission and its independent execution controls.

## Slice 4 — qualify TN and record remaining restrictions

1. Run the existing [facts generator](../tools/compare6502/tn/leaf_facts.py) once
   for current TN. Preserve the selected MIR, final image/map/relocation hashes
   and compiler/source/config identity. Its temporary hook must be removed.
2. Compare with the 9,846-byte baseline and record every changed routine's code,
   mapped ordinary homes and mapped ZP homes. Inspect Sort, its Fnamecmp call,
   and Ord/Items/FindItem/Next controls. Check changes to summaries and explicit
   observations, not just the hoped-for local placement.
3. Report the actual outcome for Sort's eligible locals: rejected before
   proposal, no available slot, expanded proof failure, cost rejection or
   accepted binding. Account for remaining gates using retained MIR; do not
   promise savings or add a sample-specific admission rule.
4. Check the existing exact-rollback control and the later-call argument/write
   rejection controls on the final implementation. Preserve one combined
   ordinary-call trial and no per-object compilation. Use the trace to record
   work counts and one elapsed/resident-memory sample, with its measurement scope.
5. Save compact evidence separately under a new benchmark directory, update
   design/plan status and link the result from related ownership/placement notes.
   Keep historical baseline files unchanged. Commit qualification and status.

## Local validation and build budget

Keep testing focused. Use the `object_storage_` prefix for the new grouped cases
so they can be selected without executing unrelated tests. Extend existing
library/integration/VM targets rather than creating another test crate. Commands
below select proposed groups after they exist; verify nonzero selected counts:

```sh
cargo test --lib object_storage_
cargo test --test mir6502_cross_call_homes object_storage_
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes object_storage_
cargo test --lib later_trials_keep_the_exact_selected_input_and_executable
cargo test --lib mir6502::parameter_homes::cross_call::tests
```

Run each group in the slice that changes its contract. Re-run a passing group
only after a relevant change or failure; broader shared exposure changes may
justify the existing ownership/observation controls. Keep a short per-slice
record of executed checks. A generic compile does not require running all
backends. No full suite or historical compiler rebuild is planned.

Use the existing `build/checked-arena/shared-target`, dev/test opt-level 1, debug
information off, incremental off and two build jobs. Keep large artifacts under
ignored `build/`. Batch related scenarios to avoid repeated compiler builds.
No implementation performance target or repeated benchmark loop is introduced.

Normalize host fixture text before newline-sensitive instrumentation and check
LF/CRLF through that actual path when it changes. Preserve binary/ATASCII bytes.
Use independent memory/result oracles; a guessed hidden-home address is not a
supported aliasing test. Explicit numeric/typed exposure is a negative proof
case and must retain its observed behavior.

These changes are intended to stay within MIR6502 and its existing contracts.
If implementation requires semantic/NIR/verifier/printer changes, separate that
shared migration and run the repository's mandatory shared-contract checks.
Broaden backend checks for actual emission changes or relevant failures as
required by AGENTS.md. Preparing these documents requires content/link checks
only.

Completion requires location-independent address-control evidence, correct
invocation and physical-clobber checks, independent execution coverage, preserved
transaction guarantees and honest TN measurements. A smaller TN image is a
measured result, not a condition for accepting a justified general contract.
