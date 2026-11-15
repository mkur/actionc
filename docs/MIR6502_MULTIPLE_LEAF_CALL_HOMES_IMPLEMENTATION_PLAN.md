# MIR6502 homes across multiple leaf calls: implementation plan

Refreshed 2026-11-15. Status: slice 1 complete; runtime/TN qualification and optimized default pending.
Baseline: `0880cd2f`, following whole-image cost qualification in `65b223ce`.

The selector is implemented behind `enable_multiple_leaf_call_homes`, currently
off by default. Four focused proof tests and the existing exact rollback control
passed. [Slice 1 receipt](benchmarks/6502-multiple-leaf-calls/slice1-checks.json).

Implements the [multiple-leaf-call design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-multiple-calls-to-leaf-routines).
The [cost qualification](benchmarks/6502-whole-image-cost/README.md) leaves normal
TN at 9,791 bytes, with identical image/map/relocations and compiler work. Its
retained map confirms FindItem at 74 code bytes plus three ordinary-home bytes.

Implement these two slices in order, committing each completed slice. Keep
unrelated working-tree changes outside those commits. Testing remains minimal.

## Selection decision

Eligible single-call and multiple-call callers join **one existing ordinary-call
batch**, built from the same selected input and expanded callee snapshot. Use the
current `cross-call-homes` stage and one `try_proposal` continuation. Accept only
strict complete-image savings after all proofs succeed; individual routine growth
remains diagnostic.

The transaction baseline is the exact `SelectedPlacement` after leaf placement
and terminal handoff, immediately before ordinary-call selection. A failed trial
restores it. An unsupported caller is excluded during discovery; a later failure
of the combined trial rolls back the complete batch. There is no extra single-call
trial, multiple-call stage or subset retry. Best-subset selection is outside scope.

The `enable_multiple_leaf_call_homes` gate controls discovery only: off preserves
exactly-one-call admission; on also admits two or more supported sites. Parent
peephole, parameter-home and cross-call gates still apply. Both configurations
have zero or one ordinary-call trial, with no new materialization/emission stage.

## Scope and existing authority

Every site must directly call a supported ordinary Action leaf routine. Check
actual entry requirements, canonical argument supply and known writes at every
site, even when several sites share one callee. A caller's forbidden set is the
union of bytes any of its callees may write; apply it to complete parameter and
initialized-local placements, retaining source-ZP exclusions and allocation order.

Reuse expanded lifetime checks, logical initialization rules, invocation-home
certificates, final-placement replay and exact rollback. The union filters
placement; it does not prove argument-setup safety or liveness through each call.
Retain the four-byte incoming-parameter limit and existing native-operation and
pointer-arithmetic restrictions. Nested/recursive callees, unknown/indirect calls,
unsupported helpers, machine bodies, save/restore and register residence remain
outside scope. Source-language and NIR contracts stay unchanged.

## Slice 1 — admit multiple sites in the existing proposal

1. Add the discovery gate to [Mir6502Config](../src/mir6502/passes.rs), initially
   false in both conservative and optimized configurations. Pass it to the current
   proposal call in [parameter_homes.rs](../src/mir6502/parameter_homes.rs); retain
   scheduling and `SelectedPlacement` ownership.
2. In [cross_call.rs](../src/mir6502/parameter_homes/cross_call.rs), collect each
   eligible caller's sites in stable block/op order. Skip zero sites. With the gate
   off, require exactly one; with it on, allow any nonzero count.
3. Reuse one [ResidenceCalls](../src/mir6502/parameter_homes/calls.rs) context for
   the selected expanded snapshot. Invoke `writes` separately for every site and
   accumulate forbidden bytes using `preserves_fixed_byte`. A failure rejects that
   caller before installing bindings. Never cache successful argument validation
   solely by target ID or replace unknown writes with an empty set.
4. Feed the union into the existing parameter/local placement helpers. Preserve
   complete byte/word objects, absent-home handling and deterministic allocation.
   Build one proposal for all eligible callers and reuse `try_proposal`, the
   qualified cost guard and the current continuation.
5. Keep diagnostics in `cross-call-candidate`, adding call count and failed-site
   identity where useful. Reuse `cross-call-home-result` and the zero-or-one
   `cross_call_home_trials` counter; diagnostics never control acceptance.

Add a small `multiple_leaf_calls` unit group beside the selector/call tests:

- Two distinct supported callees and a repeated target qualify when enabled;
  the same multi-site caller is unselected with the discovery gate off.
- Only the second callee writes one lane of a prospective word home. Placement
  avoids it; forcing a conflicting binding fails expanded lifetime verification.
  Include an initialized local in the union check.
- A valid first site cannot certify an invalid second site. Cover an unsupported
  callee and repeated calls where only the later site lacks a required entry input.
- Invalidate a second-site proof after discovery and exercise the existing
  transaction seam: rejection restores the exact selected input, executable,
  map/relocations and trace selection, with at most one continuation. Reuse existing
  rollback helpers and `later_trials_keep_the_exact_selected_input_and_executable`.

Reuse unchanged alias, exposure, ABI and local-initialization coverage. Run the
new unit group and that existing rollback control. Commit the gated implementation
and focused tests; optimized admission remains off for this slice.

## Slice 2 — independent execution, TN and optimized default

Add a small `fixtures/runtime/multiple_leaf_calls.act` control. Include a caller
with a pointer/scalar and a local initialized on every invocation that calls two
different leaves, plus a repeated-target caller. Exercise conditional execution
and a loop without nested callees. Use explicit results and source-memory sentinels
so runtime expectations are independent of private home placement.

Add a grouped `multiple_leaf_calls` case to
[mir6502_cross_call_homes.rs](../tests/mir6502_cross_call_homes.rs). Disable inlining
for the isolated control, keep other optimized choices fixed and compare the new
gate off/on. Require actual final savings, resident homes, correct initialization,
truthful deltas and traced/untraced artifact equality. Include a one-call candidate
in the same program and assert one combined trial. Check parent gates in this
small control, without a configuration matrix.

Move `Twice` out of the existing blanket unsupported-call list into gate-specific
coverage. Retain nested/recursive, exposed and unsupported-call negatives; rerun
that directly edited test once. Keep fixture-specific correctness assertions and
allow individual routine growth when the complete candidate saves bytes.

Extend the existing [cross-call VM target](../tools/vm-runtime-tests/tests/cross_call_homes.rs)
with one grouped `multiple_leaf_calls` test: cart off/on, actual LF/CRLF loading for
the enabled path and one enabled standalone smoke case. Check every leaf's
arguments and call order, results, stack balance, repeated invocations, conditional/
loop paths, page crossing and protected source memory. Normalize host text before
newline-sensitive processing; compare executable artifacts rather than LF/CRLF
debug offsets. Reuse existing VM helpers and keep the scenario group small.

This slice also enables the gate in `Mir6502Config::optimized()` while leaving
`Default` false. Include default-versus-explicit-on coverage in the integration
group. Make default/expectation edits before the final focused runs and qualify
them before committing; do not rerun passing tests for documentation-only edits.

Reuse [parameter_homes.py](../tools/compare6502/parameter_homes.py) and its Rust
probe for one final TN gate-off/on comparison at ActionCart `$3000`. Add only the
discovery-gate override and necessary routine/decision rows, including FindItem;
reuse the focused cart-only mode. Keep scoped observation and other optimized
defaults enabled. Verify unchanged sources/settings and require the off arm to
reproduce the qualified 9,791-byte image. The added false config field is expected
in the new configuration record; all previous settings must match the reference.
Record:

- Complete image delta; FindItem code and ordinary/ZP homes; every other changed
  routine footprint, including growth and the Ord/Items controls.
- Image/map/relocation hashes, selected decisions and compiler-library identity.
- Trial/materialization/emission/preservation work. Both arms remain bounded to
  one ordinary-call trial; larger candidates may change downstream analysis work.
  Capture one elapsed/RSS observation without a performance claim.

TN savings are measured, not promised. If unchanged, identify the remaining
eligibility/proof/cost blocker while retaining the independent saving and execution
evidence. If TN grows versus the off arm, investigate the selected sequence before
qualifying the optimized default. Do not weaken the guard or add subset retries.

Publish compact receipts under `docs/benchmarks/6502-multiple-leaf-calls/`, update
design/plan status and commit qualification, default and fixture changes. Broader
candidate-selection work remains a separate follow-up.

## Minimal checks and build budget

Reuse `build/checked-arena/shared-target`, dev/test opt-level 1, debug information
disabled, incremental off and two build jobs. Keep large artifacts under ignored
`build/multiple-leaf-calls/`; avoid another target tree. Planned filters:

```sh
cargo test --lib multiple_leaf_calls
cargo test --lib later_trials_keep_the_exact_selected_input_and_executable
cargo test --test mir6502_cross_call_homes multiple_leaf_calls
cargo test --test mir6502_cross_call_homes unsupported_calls_exposed_objects_and_exhaustion_retain_the_fallback
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes multiple_leaf_calls
```

Verify nonzero selected test counts. These are future filters, not checks already
run. Defer full backend suites, NIR sweep, raw-NIR/origin matrices, MADS and full TN
runtime scenarios under the user's minimal-testing direction. Broaden only for an
actual relevant failure. Documentation-only updates need content/link checks.

Completion requires every site to participate in placement/proofs, one bounded
combined trial with exact rollback, independent final savings with a passing VM
oracle, qualified gate/default behavior and a recorded TN outcome.
