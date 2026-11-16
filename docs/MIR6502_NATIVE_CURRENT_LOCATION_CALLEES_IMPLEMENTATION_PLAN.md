# MIR6502 native current-location callees: implementation plan

Prepared 2026-11-16. Status: slice 1 complete; slices 2–3 pending.

Slice 1: native observable leaves now support actual-entry analysis while
planned capture remains excluded. All 18 shared-entry/capture/handoff unit
tests passed, including the new ABI/staleness group and observable conditional
capture coverage. Residence admission is unchanged in this slice.

Implement in three slices, committing each after its focused checks pass.
Preserve unrelated working-tree changes, including the existing call-chain
extension in the parameter-home design.

## Objective and boundary

Allow an ordinary caller to keep eligible values in ZP across a supported
high-level `=*` callee. Current-location placement must not, by itself, make
that callee's entry requirements or writes unknowable.

The [storage contract](ACTION_STORAGE_LAYOUT.md#routine-storage) already states
that `=*` affects entry address and patchability, not argument placement. TN
uses it in high-level routines to avoid historical entry trampolines. Today
[lowering](../src/mir6502/lower.rs) classifies these routines as
`ActionObservable`; both
[residence calls](../src/mir6502/parameter_homes/calls.rs) and native
[entry analysis](../src/mir6502/analysis/known_callees/entry_inputs.rs) reject
that classification before using the available body.

Fix callee-contract eligibility, not the global ABI classification. Keep
`ActionObservable`, entry labels, argument/return lanes, capture generation and
the callee's own storage policy intact. In particular, do not relabel all `=*`
routines as `Action`: that would also widen inlining, storage removal, handoff
and other consumers that this fix does not qualify. No new NIR form, parser
change, allocator, production feature switch or TN source rewrite is needed.

The first admitted case is a direct call to an emitted native `=*` leaf with a
complete supported ABI, known actual-entry requirements and complete writes.
Use existing typed targets and body/effect facts; do not identify routines by
name or recover source syntax in MIR6502. A routine may qualify as a callee
while its own parameter and local homes remain inline.

Separate follow-ups remain:

- Placement of the `=*` routine's own source homes.
- Nonleaf entry composition and coordinated call-chain placement, covered by
  the [existing design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-zp-placement-through-acyclic-call-chains).
- Assembly/OS contracts, SArgs, unsupported helpers and indirect calls.
- `Copy`'s pointer-read/variable-word-arithmetic body restriction.

This correction therefore does not promise ZP homes or a size saving for `Copy`.
Its current body rejection remains first; `SwapWin` remains a nonleaf callee.

## Required invariants

1. Preserve the public Action ABI and direct entry. Entry address observation
   does not imply arbitrary memory effects, and an analyzable body does not
   authorize changing its public boundary.
2. Certify the actual expanded entry, including its real capture operations.
   For newly admitted observable native entries, do not invent a planned
   prologue. Keep their `PlannedCapture` query unsupported in this delivery.
3. Validate every call site's canonical arguments and extra register, flag and
   fixed-byte inputs. Unknown entry requirements never become an empty set.
   Even a dead caller value can affect the callee's inputs through ZP reuse.
4. Require preservation only for values live across the invocation, using the
   existing per-object liveness. Include capture, return delivery and scratch
   in the actual write footprint; protect both lanes of a live word.
5. Keep source ownership, exposure, persistent-value and physical-observation
   checks unchanged. Complete writes do not turn unknown reads into known reads.
6. Reuse one placement transaction, strict whole-image cost acceptance,
   candidate-specific facts, final preservation replay and exact rollback.

## Slice 1 — certify native observable actual entries

Primary code: [entry_inputs.rs](../src/mir6502/analysis/known_callees/entry_inputs.rs)
and the existing [entry tests](../src/mir6502/parameter_homes/handoff_tests.rs).

1. Separate finite ABI declaration validation from permission to synthesize
   capture. Today `direct_parameter_abi_is_complete` mixes the two through its
   `Action | ProgramEntry` check.
2. Admit `ActionObservable` to native analysis at `ActualEntry` when its
   existing body, CFG, signature and effect requirements are satisfied. Retain
   the current four-byte direct-capture boundary and typed pointer support.
   Keep program-entry and external-interface admission unchanged.
3. Analyze existing operations once. Preserve the distinction between definite
   initialization and possible writes, partial capture, incoming flags, fixed
   inputs and the ordinary balanced-stack dependency. Machine bodies continue
   through their existing bounded analysis, without new permissions.
4. Keep fact freshness tied to the body, ABI, bindings, stage and backing facts.
   Do not cache a newly accepted fact across materialization or mutation.
5. Add a short amendment to the existing parameter-home design describing
   actual-entry analysis of native `=*` callees. Preserve its pending call-chain
   extension and state that planned observable capture remains outside scope.

Focused checks: one grouped regression for ordinary/current-location native
entries with zero, byte, word/pointer and four-byte arguments; missing or
conditional capture; extra flag/fixed inputs; malformed declarations; and
stage/body staleness. Update the existing observable-ABI negative case to
distinguish planned from actual entry. Run the affected capture and shared-entry
tests, including the existing machine-entry control. Placement admission stays
unchanged in this slice.

Exit: native `=*` leaf facts describe the actual entry without changing ABI,
capture or storage. Commit the analysis correction and contract amendment.

## Slice 2 — use those facts for caller residence

Primary code: [calls.rs](../src/mir6502/parameter_homes/calls.rs), its tests,
[cross_call/tests.rs](../src/mir6502/parameter_homes/cross_call/tests.rs) and
[cross-call integration tests](../tests/mir6502_cross_call_homes.rs).

1. Replace the blanket observable-ABI veto in residence call admission with
   the narrow supported native case from slice 1. Require a direct routine
   target, a complete actual-entry fact and the existing canonical ABI checks.
   Absolute/system, external and indirect targets stay rejected.
2. For the newly admitted class, require the actual-entry fact at dead as well
   as live boundaries. Preserve the existing ordinary-native dead-call policy;
   do not accidentally treat `None` as permission for an observable callee.
3. For live values, consume the existing known-callee write summary and physical
   byte-preservation query. Retain the leaf, machine, SArgs, helper, barrier and
   unknown-write exclusions. No new call-graph traversal or effect-summary
   implementation belongs here.
4. Reuse the same admission in discovery and expanded lifetime verification.
   Audit the existing spill selector and `InvocationHomes`, which also consume
   `ResidenceCalls`; retain their own object eligibility and proof requirements.
   Do not broaden register-only bindings or terminal parameter handoff.
5. Report the remaining concrete reason for rejection: unsupported body,
   nonleaf preservation, missing entry requirements or unknown writes. A pure
   `=*` leaf must no longer fail merely for its declaration spelling.

Focused checks: an ordinary caller with a live byte/word across a native `=*`
leaf; a matching plain-declaration control; explicit low/high-lane clobbers;
extra entry inputs; dead versus live values; and a repeated target with an
invalid later call site. Keep a native nonleaf and an assembly `=*` negative
control. Exercise a caller spill through the same contract and the existing
complete-artifact rollback control. Disable inlining in the isolated positive
fixture so the call and preservation obligation remain real.

Exit: a profitable independent caller selects ZP homes through the existing
transaction; live clobbers and unsupported entries retain safe fallback.
Commit separately.

## Slice 3 — focused execution and TN measurement

Extend the existing [cross-call VM target](../tools/vm-runtime-tests/tests/cross_call_homes.rs)
with one grouped native-current-location control. Check a host-derived result,
actual argument values, unrelated memory sentinels and stack balance across
repeated calls and byte/page boundaries. Cover ActionCart and Standalone,
placement disabled/enabled, and LF/CRLF through the actual source loader.
Normalize host fixture text before newline-sensitive instrumentation.

Use the existing parent feature gates for the execution comparison. Retain a
pre-fix artifact for an exact correction-only size comparison; disabling all
cross-call placement also removes older optimizations and is not that baseline.
The positive integration case must show selected caller homes and a strict
packaged-image improvement, not just successful fact construction. Verify that
the callee still uses its direct entry and existing calling convention.

Run one optimized TN before/after comparison with identical sources, runtime,
origin and configuration. The retained
[baseline receipt](benchmarks/6502-zp-home-lifetimes/README.md) records ActionCart
at `$3000`, **9,811 packaged bytes**, with `Copy` at 673 instruction bytes plus
58 inline bytes (48 array bytes and 10 scalar bytes). Check source/compiler
identity before reusing it; record baseline drift instead of attributing it to
this fix. Reuse the shared build directory and retain large artifacts under
ignored `build/`.

Record changed caller homes, routine code/storage deltas, final image size and
placement-trial counts. Reinspect `Copy`/`SwapWin` and report remaining blockers.
An unchanged TN image is acceptable when the independent saving is demonstrated
and the restriction has been removed; do not widen the patch to manufacture a
TN reduction. Save a compact measurement receipt and update this plan's status.

Keep local testing focused: new/affected library tests, the cross-call integration
target, relevant ABI/handoff regressions and the new VM group. Do not rerun full
compiler or unrelated backend suites for this analysis/admission correction.
Broaden if failures or actual implementation changes expose wider consumers;
shared NIR/semantic contract changes would invoke the repository's mandatory
shared checks and require revisiting this plan's scope.

Exit: focused execution passes, independent savings and TN results are recorded,
and no extra placement continuation or source workaround is introduced. Commit
the final tests and receipt.
