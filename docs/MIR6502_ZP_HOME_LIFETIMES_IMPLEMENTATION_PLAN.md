# MIR6502 direct ZP homes and value lifetimes: implementation plan

Prepared 2026-11-16. Status: slices 1–3 complete; slice 4 pending.

First delivery of the
[ZP lifetime design](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#zp-home-lifetimes-and-persistent-storage).
Implement the four slices in order, with focused checks and a commit after each
slice. Preserve unrelated working-tree changes. Working copies are not part of
this delivery; dedicated persistent reservations have the follow-on boundary below.

Slice 1 validation: all 53 parameter-home unit tests passed, including the new
logical call-site/entry-dependence control. The existing local initialization,
physical clobber, handoff, observation and rollback controls remain green.

Slice 2 validation: all 54 placement unit tests and the new mixed-home VM
control passed. The VM covers gate off/on, both runtimes, repeated calls and
LF/CRLF through the source-loading path. The new discovery gate remains off
by default pending qualification. Direct Param/Local address formation is the
only added body shape; addressed objects retain inline homes.

Slice 3 validation: all 55 placement unit tests, the new final-output integration
control and the alternating-subtree VM control passed. The VM covers both
runtimes, gate off/on, word boundaries, repeated calls and LF/CRLF input. Calls
validate flattened ABI lanes after word expansion. Calls outside a value
lifetime still exclude result slots: result/home handoff is not part of this
slice. The same uncertified target is accepted at a dead boundary and rejected
when a value must survive it; malformed inputs and either-lane clobbers reject.

## Outcome and scope

Give eligible parameters and locals direct ZP homes even when an unrelated
parameter is exposed or a call cannot preserve a value that is already dead.
Reuse ZP between independent invocations and call subtrees when their values
do not interfere. Keep one authoritative home per object, the existing pool,
the public ABI and the existing final-size transaction.

This delivery extends invocation placement. It does not make entry-dependent
locals invocation-local. `PopUp.menu` still needs retained-value preservation;
its `Push(menu)`/`menu = Pull()` sequence stays unchanged. `PopUp.item` is a useful
short-lived candidate, while `@c` should affect only `c`'s placement. Neither
routine names nor a promised TN saving are eligibility rules.

Keep existing scalar widths, at-most-four incoming argument bytes, supported
native operations, source-address protection and physical observation checks.
Arrays, aliases, fixed or exposed homes and initializer-backed locals keep their
existing policy. Do not introduce new register/argument/result coalescing,
within-routine home coloring, save/restore insertion or backing copies.

Reuse currently qualified call contracts. The separate
[acyclic call-chain design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-zp-placement-through-acyclic-call-chains)
owns nested entry-contract composition and coordinated descendant proposals.
This plan neither implements that analysis nor treats its proposed behavior as
already available. Unknown calls may bound a dead value's lifetime; they cannot
certify a live value's preservation.

## Current implementation and baseline

| Boundary | Current behavior | Change in this plan |
| --- | --- | --- |
| `eligible_with_calls` | Every parameter must pass address-control and scalar eligibility | Separate routine/ABI legality from per-object placement eligibility |
| `zero_page::bindings_avoiding` | Receives one forbidden-byte set for all parameters | Keep complete mixed bindings; use object-specific call exclusions |
| `cross_call::propose` | Unions every call's writes; one unsupported call rejects the caller | Require preservation only at calls crossed by each candidate value |
| `lifetimes::verify_with_address_control` | Requests every nonterminal call's residence contract before solving liveness | Derive liveness first and check physical clobbers against surviving values |
| Local entry dependencies | A read of a previous invocation's value rejects invocation placement | Retain this restriction and protect persistent siblings |
| Allocation and selection | Per-routine reservations; one combined cross-call trial | Retain sharing, one continuation and exact rollback |

Start from `56526393`, including the existing call-preserved spill work. The
[retained TN receipt](benchmarks/6502-cross-call-spills/README.md) records optimized
MIR6502 ActionCart at `$3000`: **9,815 bytes**, Sort 238 instruction bytes and no
inline scalar/spill home, with `gap` at `$EC`. Reuse that evidence without
rebuilding the historical compiler. At qualification, compare gate-off/on with
the same compiler and source; record any baseline drift explicitly.

## Slice 1 — shared logical liveness before physical placement

Primary code: [lifetimes.rs](../src/mir6502/parameter_homes/lifetimes.rs),
[locals.rs](../src/mir6502/parameter_homes/locals.rs),
[home_liveness.rs](../src/mir6502/analysis/home_liveness.rs) and
[effects.rs](../src/mir6502/analysis/effects.rs).

1. Extract the existing logical read/definition projection into one reusable
   query for ordinary parameter/local bytes. Key facts by owning routine,
   typed object identity, byte lane and CFG site; do not use physical ZP bytes
   as the only identity before placement or confuse equal IDs in two routines.
2. Report entry dependencies and values live across each invocation. Reuse the
   existing CFG/liveness solver. A source definition kills the old value;
   callee may-writes, argument setup and scratch writes do not initialize it.
   Treat explicit caller result definitions separately from callee clobbers.
3. Preserve both lanes of words and conservative joins/backedges. Values used
   to prepare arguments are read before invocation; a value first defined after
   a call has no obligation to survive that call. Include existing explicit
   physical-read observations when projecting a bound executable.
4. Reuse the query for local entry-dependence checks and expanded residence
   verification. Tie results to the immutable routine/program snapshot; rerun
   after expansion or rewrites. Missing CFG/effect evidence grants no proof.
5. Keep selection and existing call admission unchanged in this slice.

Focused checks: one grouped unit control covering byte/word lanes, conditional
definition, a loop backedge, last argument use, definition after a call, an
entry-dependent local and equal local IDs in different routines. Run affected
existing local/lifetime tests once. Do not add a second liveness engine.

Exit: discovery can ask which logical values cross a call without first binding
them to ZP; current accepted bindings and fallback output remain unchanged.
Commit the shared query and tests.

## Slice 2 — mixed eligibility without a whole-routine veto

Primary code: [parameter_homes.rs](../src/mir6502/parameter_homes.rs),
[zero_page.rs](../src/mir6502/parameter_homes/zero_page.rs),
[materialize/abi.rs](../src/mir6502/materialize/abi.rs) and existing mixed-binding
verification/tests.

1. Separate routine-level requirements (supported body, ABI and signature)
   from each parameter/local's address control and supported storage shape.
   Keep unsupported objects inline while considering eligible siblings.
2. Produce the existing complete binding tables. An exposed parameter remains
   inline even when it looks unused; it must not acquire `Absent`, register or
   ZP storage through a different path. Preserve capture for all inline lanes.
3. Compute outgoing/incoming offsets from the complete original ABI signature,
   including skipped objects. Skipping one parameter must not shift a later
   parameter's lane or weaken argument setup interference.
4. Retain local persistence, initialization, alias and fixed-address exclusions.
   Keep register-only/handoff strategies under their current eligibility unless
   their mixed-object behavior is independently covered; this slice broadens
   direct memory placement only. Direct `LeaAddr` of a Param/Local is admitted
   for this mixed case; its addressed object remains inline and its existing
   structured address-reference effects remain authoritative.
5. Retain the current whole-call exclusion rule until slice 3. Use existing
   reservations, physical layout and final acceptance; no new allocator or trial.

Focused checks: a mixed exposed/eligible parameter pair, an eligible local beside
an exposed parameter, and an entry-dependent local beside an initialized local.
Verify ABI offsets/captures, the unchanged address of the exposed home, both
word lanes and complete binding tables. Add one VM control with independent
expected results and sentinel memory, reusing existing fixture-loading helpers.

Exit: one exposed parameter no longer rejects every home in an otherwise
supported routine. Persistent and exposed objects retain their original storage.
Commit separately.

## Slice 3 — require preservation only across live calls

Primary code: [cross_call.rs](../src/mir6502/parameter_homes/cross_call.rs),
[calls.rs](../src/mir6502/parameter_homes/calls.rs),
[lifetimes.rs](../src/mir6502/parameter_homes/lifetimes.rs),
[zero_page.rs](../src/mir6502/parameter_homes/zero_page.rs),
[observations.rs](../src/mir6502/parameter_homes/observations.rs) and
[preparation.rs](../src/mir6502/preparation.rs).

1. For each candidate object, union the writes of only those invocations across
   which its value is live. Cache existing callee facts by target but check site
   inputs separately. An unknown preservation contract rejects a candidate
   crossing that call, not unrelated candidates whose values are dead there.
2. For an otherwise supported ordinary call with no live candidate value,
   conservatively model unknown invocation writes as clobbering every candidate
   byte. Do not request an unnecessary residence-only preservation certificate.
   Keep call legality, ABI setup, required entry inputs, explicit observations
   and ordering checks; do not infer any narrower public read/write summary.
3. Solve expanded logical liveness before enforcing residence call obligations.
   Check setup instructions independently, read call inputs before invocation
   effects, and validate newly defined results afterwards. A physical write
   cannot kill its own liveness proof. Unsupported machine/helper operations
   remain outside routine admission; dead storage is not permission to lower
   an unsupported operation.
4. Give the existing byte picker each object's forbidden set alongside shared
   source, scratch, ABI and already-selected home reservations. Keep distinct
   source homes within a routine conservatively disjoint. Do not reserve an
   invocation home globally: independent eligible routines may reuse it.
5. Keep the existing coordinated proposal boundary. Where qualified call-chain
   support exists, include proposed descendant writes using its existing rules;
   do not add a call-graph fixed point or per-subtree trials. A caller live across
   a descendant write must still avoid those bytes.
6. Rebuild candidate facts after the core continuation. Replay physical layout,
   source observations and earlier preservation proofs with terminal frames
   fixed. Update `InvocationHomes` only through the same valid residence facts;
   do not classify persistent storage as dead merely because a sibling is dead.
7. Keep the existing spill promotion policy unless a shared verifier change
   requires adapting it. Preserve Sort's selected spill and ordinary homes.

Focused checks: a value dead before a call with unknown preservation; a value
defined afterwards; a live value rejected at that same call; repeated sites
where only one is crossed; a setup clobber; and a conditional/loop-carried word.
Use two independent eligible caller/leaf subtrees to prove actual byte reuse,
plus a parent/child conflict control. Test a changed candidate callee clobber
and exact rollback, not just the discovery proposal.

Exit: short-lived direct homes can coexist with unrelated unsupported residence
contracts, while every live byte remains protected. Final acceptance still
requires a strictly smaller packaged image; equality/failure restores the exact
selected artifact and all reservations. Commit separately.

## Slice 4 — focused execution and TN qualification

Primary targets: [cross-call integration](../tests/mir6502_cross_call_homes.rs),
[parameter-home integration](../tests/mir6502_parameter_homes.rs),
[cross-call VM controls](../tools/vm-runtime-tests/tests/cross_call_homes.rs) and
the existing TN controls whose routines actually change.

1. Group the new controls around mixed homes and value lifetimes. Execute
   repeated calls and alternating sibling subtrees; compare results, retained
   inline state, source-owned sentinels, call arguments and stack balance with
   independent oracles. Include a pointer/word boundary case.
2. Run placement off/on in ActionCart and Standalone on the small controls.
   Exercise LF and CRLF through the actual loading/instrumentation path when
   adding newline-sensitive fixtures. Reuse existing runtime/origin coverage;
   do not multiply every semantic case across a redundant matrix.
3. Measure TN once at the final boundary, optimized ActionCart `$3000`, with
   identical sources/configuration except the discovery change. If an isolation
   gate is needed, make it subordinate to the existing home/cross-call gates;
   disabled behavior must remain byte-identical. Qualify before default enablement.
4. Record total image size, routine instruction/storage changes, selected homes,
   peak ZP use, blockers and continuation/emission counts. Verify there is still
   zero or one combined cross-call trial, with no per-object or per-depth replay.
   Do not impose a per-routine growth veto or promise that every candidate wins.
5. Inspect PopUp's selected homes and report the concrete remaining reasons for
   `menu`, `c`, `item` and other candidates. Compare Ord/Items/FindItem,
   Strcpy/Fnamecmp and Sort with their retained baseline; report individual
   changes under the whole-image criterion without adding subset retries.
6. Store a compact receipt, relevant assembly and source/configuration hashes
   under `docs/benchmarks/6502-zp-home-lifetimes/`; keep binaries and full dumps
   under ignored `build/`. Remove temporary diagnostic hooks before committing.

During development use filtered `cargo test --lib mir6502::parameter_homes`
cases and explicit integration filters. At this final boundary run the affected
`mir6502_cross_call_homes` and `mir6502_parameter_homes` integration targets and
the VM `cross_call_homes` target through `tools/vm-runtime-tests/Cargo.toml`, plus
affected ABI/parameter VM cases and changed TN paths. Reuse passing checks unless
new edits or failures justify repeating them. Do not run repository-wide suites
for MIR6502-only work; broaden within this backend if behavior or emission changes
require it. Any unexpected NIR/shared-contract change requires a separate slice
and the checks mandated by AGENTS.md.

Exit: qualification distinguishes actual savings, safe rejected candidates and
unchanged TN code. Commit the qualification and documentation separately.

## Follow-on: dedicated persistent homes

Dedicated reservations are the second strategy in the design, not a dependency
of slices 1–4. Schedule them as a separate implementation after direct placement
has been measured. They need all of the following before selection can open:

- Explicit typed reservation lifetime/ownership for a direct persistent binding,
  consumed by every allocator, scratch/helper user, verifier and final layout.
  A frame-local fixed-byte reservation alone cannot protect an inactive owner.
- An initialization mechanism matching the existing image lifecycle. Currently
  `emit_storage_init(None, ...)` emits zero-filled bytes for ordinary inline scalar
  storage, while `format_load_file` emits a contiguous program segment and RUNAD.
  Relocating the binding alone neither loads nor initializes its ZP bytes.
  Specify initialization on fresh load, retained values on repeated routine or
  program entry, and runtime/loader ordering before choosing an implementation.
  Do not silently zero the home on every routine/program entry.
- Physical preservation throughout retained lifetimes, including other call
  subtrees, runtime/assembly calls and supported asynchronous behavior. Unknown
  obligations keep the home inline; no invented contract for TN's Getchar.
- Full cost of initialization, dedicated pool pressure and affected routines in
  the existing whole-image transaction, with rollback of all reservation state.
- Independent initial-value and repeated-entry controls, alternating persistent
  owners, an inactive-owner clobber control and runtime/loader qualification.

Until that extension is implemented, an entry-dependent object stays inline.
Working copies remain optional future work requiring a measured case; this plan
adds no duplicate homes, dirty state, cache invalidation or copy synchronization.
