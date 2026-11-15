# MIR6502 whole-image cost acceptance: implementation plan

Prepared 2026-11-15. Status: slice 1 complete; slice 2 qualification pending.
Compiler baseline: `8626beff`.

Slice 1 checks passed: three `whole_image_cost` tests (two new/updated boundaries
and one existing pointer-retention control), exact selected-artifact rollback,
and callee-contract rollback. [Check receipt](benchmarks/6502-whole-image-cost/slice1-checks.json).

Implements the [whole-image cost design refinement](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-refinement-whole-image-cost-acceptance).
Execute slices 1–2 in order and commit after each completed slice. Keep unrelated
working-tree changes outside those commits. Multiple-leaf-call selection remains
suspended while this policy is qualified.

## Objective

Accept a successfully verified candidate when its final packaged image is
strictly smaller than the transaction's selected baseline, even when individual
routines grow. Keep per-routine changes visible in diagnostics. Equal or larger
images, incompatible routine inventories and failed proofs remain rejections.

Apply this policy to the two final acceptance predicates only:

| Predicate | Consumers |
| --- | --- |
| [Executable::improves](../src/mir6502/preparation.rs) | Parameter-home, handoff and single-call placement through `try_proposal` |
| [improves_cost](../src/mir6502/late_cleanup/mod.rs) | Terminal store/producer/register cleanup and optional [spill pruning](../src/mir6502/late_cleanup/pruning.rs) |

Keep routine-set compatibility in `Executable::improves`. Terminal transactions
retain their existing permitted-delta and structural checks. Preserve every
binding, ABI, memory, register/flag, callee-contract, layout and replay obligation.
Cost savings cannot compensate for invalid code or missing proof.

Use existing finalized image measurements at identical origin/runtime/settings.
Do not replace them with a sum of routine costs: storage sharing, helpers,
branches and packaging belong to actual emission. No growth allowance, weighted
score, feature gate, new pass, trial, emission or allocation policy is needed.
Optional pruning remains off by default.

Earlier per-routine `does_not_grow` filters are outside this implementation.
Review their discovery/selection roles separately after the final guard is
qualified. Multiple leaf calls, nested call levels and subset selection are also
outside scope. The new policy does not promise the best unmeasured candidate
subset or a globally optimal sequence of optimizations.

## Minimal testing and baseline reuse

The user's explicit direction is **minimal testing**:

- One filtered unit group for both cost boundaries, plus the existing focused
  rollback checks that protect the relevant transaction contracts.
- One end-to-end integration regression using an existing source fixture, and
  its existing VM oracle. No new broad fixture or runtime matrix.
- One final TN ActionCart `$3000` comparison against the recorded baseline.
  Reuse old artifacts/receipts when source and configuration hashes match; do not
  rebuild the old compiler solely to regenerate them.
- No full `cargo test`, NIR sweep, full backend suites, MADS, origin/raw-NIR matrix
  or full TN VM scenario locally. Broaden only for a specific relevant failure.
- Do not repeat passing tests after documentation-only changes. Run modified
  expectations once after the implementation settles.

Reuse `build/checked-arena/shared-target` with dev/test debug info disabled,
dev/test opt-level 1, incremental off and two build jobs. Store large artifacts
under ignored `build/whole-image-cost/` and compact receipts under
`docs/benchmarks/6502-whole-image-cost/`. Avoid another build tree.

Use [scoped-observation qualification](benchmarks/6502-storage-observation/README.md)
as the starting evidence. Retain its historical results unchanged:

| Case | Recorded result under the old cost policy |
| --- | --- |
| Normal optimized TN, scoped observation on | 9,791 packaged cart bytes |
| TN with scoped observation off | 9,799-byte trial rejected; 9,847-byte fallback selected |
| Independent storage-observation control, scoped observation off | 229-byte trial rejected because Walk grows two bytes; 261-byte fallback selected |
| Same independent control, scoped observation on | 227 bytes selected |

The independent 229-byte trial supplies a real net-saving/local-growth regression.
It is not a hard-coded size requirement for the new compiler. Leave scoped
observation on for normal production measurements; use the explicit-off control
only to isolate the cost-policy boundary.

This task has no new policy switch. Before/after policy comparisons therefore
use the old and new compiler at the same configuration, not an unrelated
optimization toggle. Check normalized source, configuration, image/map/relocation
hashes and compiler/library identity before relying on a recorded reference.

## Slice 1 — revise final predicates and retain proof rejection

1. In `Executable::improves`, remove only the per-routine `cost <= baseline`
   conjunction. Retain strict `format_load_file(...).len()` comparison and the
   matching routine-ID inventory check. Keep `Executable.costs` for structural
   compatibility and reporting; document that its values no longer veto growth.
2. In `late_cleanup::improves_cost`, remove the per-routine instruction-size
   conjunction. Keep strict `new.image_bytes < old.image_bytes`. Remove the now
   unused program parameter and update both operation-cleanup and pruning callers.
   Retain measured instruction totals/maps for existing trace deltas.
3. Update [parameter_homes.rs](../src/mir6502/parameter_homes.rs) diagnostics:
   a cost failure should report `no-final-saving`; incompatible routine inventories
   should remain distinguishable from profitability and invalid-trial failures.
   Continue reporting routine code/home changes, including positive deltas on
   accepted trials. Reuse existing measurements and trace plumbing.
4. Leave candidate construction, batching, mutation guards, verification order,
   preservation replay and exact selected-artifact rollback intact. Do not weaken
   the optional pruning allocation guard or enable that feature.

Group cost-policy regressions under `whole_image_cost` in the existing preparation
and cleanup test modules. Update
`shared_cost_gate_rejects_zero_saving_and_individual_routine_growth` in
[pruning/tests.rs](../src/mir6502/late_cleanup/pruning/tests.rs), whose last assertion
currently enforces the policy being removed.

The small grouped cases must establish:

- A smaller final image is accepted despite a positive routine-cost/instruction
  delta, for both predicates.
- Equal or larger final images are rejected, even if routine estimates improve.
- A smaller placement candidate with a mismatched routine-ID inventory is rejected.

Use the existing transaction checks
`later_trials_keep_the_exact_selected_input_and_executable` and
`producer_cleanup_rolls_back_when_a_callee_exit_fact_would_change` to retain
coverage of exact rollback and correctness taking precedence over savings.
Update an expectation only if it specifically encodes the removed cost veto;
do not weaken semantic, frame, alias or callee-preservation assertions.

**Minimal checks:** the new filtered unit group and those two named existing
checks. Commit the predicate/diagnostic changes and focused regressions together.
No new default switch or optimizer stage is introduced.

## Slice 2 — prove a real acceptance change and measure TN

Add one focused `whole_image_cost` case to
[mir6502_cross_call_homes](../tests/mir6502_cross_call_homes.rs), reusing
[storage_observation.act](../fixtures/runtime/storage_observation.act).

Keep small-leaf inlining disabled for the isolated case and scoped observation
off. Compare cross-call placement off/on to inspect the actual selected trial:
require a smaller packaged image, an actual increase in Walk's code footprint,
and resident FindRecord homes. The recorded old-policy run at cross-call on
selected the fallback; the new policy must accept a verified net saving.
Use measured relationships rather than asserting only a fabricated cost table.
Check traced/untraced equality and truthful acceptance/routine-delta diagnostics.

The explicit scoped-observation override is part of this regression's fixed
configuration. It does not replace the normal optimized configuration for TN,
and it is not a switch between the old and new cost policies.

Run the existing VM case
`storage_observation_preserves_invocations_arguments_and_owned_memory` in
[the cross-call VM target](../tools/vm-runtime-tests/tests/cross_call_homes.rs).
Its scoped-observation-off arm now executes the previously rejected placement.
Its independent oracles already cover call arguments, return values, repeated
invocations, page crossing, stack balance, source sentinels, LF/CRLF loading and
one standalone smoke case. Reuse that group rather than adding another matrix.
Normalize checked-out host text before newline-sensitive processing; preserve
exact binary/memory comparisons and exclude source debug spans from LF/CRLF
artifact equality.

Inspect directly affected integration assertions and diagnostic expectations.
Keep per-routine assertions that are specific behavioral oracles for a particular
fixture; remove only assertions claiming that individual growth must always
reject an otherwise smaller candidate. Historical benchmark receipts remain
immutable; publish new receipts and explain any changed selected artifacts.

Measure normal optimized TN once at ActionCart `$3000`, with all current defaults
and scoped observation enabled. Compare against the 9,791-byte recorded artifact
at identical configuration. Record actual final image, routine code/ordinary/ZP
sizes, selected decisions, source/library/configuration hashes and image/map/
relocation hashes. Report every growing routine alongside the complete net delta.
An unchanged image is a valid result if the independent regression establishes
the policy change. Do not turn off unrelated optimizations to manufacture a
production saving. If the new final image grows, identify which changed selection
caused it before treating TN qualification as complete; local transaction savings
do not by themselves prove the best sequence of selections.

Reuse [the existing measurement harness](../tools/compare6502/parameter_homes.py).
Extend its reported rows or decision summary only if needed; add no cost-policy
flag. Capture work counts and one elapsed/RSS observation. The policy change
adds no structural trial or emission; counts can nevertheless change when a
previously rejected candidate is accepted and later work follows a different
path. Explain such deltas instead of asserting identical totals or inventing
extra measurement passes. Do not claim a timing regression or speedup from one
sample.

Publish the focused checks, actual acceptance changes, preserved correctness
rejections, TN outcome and deferred coverage. Mark the cost refinement and this
plan complete. Keep the multiple-leaf-call draft suspended until its batching
choice is revisited against this qualified baseline. Commit qualification,
expectation updates and documentation as the second slice.

## Commands and completion criteria

After introducing the new filter, verify nonzero selected test counts:

```sh
cargo test --lib whole_image_cost
cargo test --lib later_trials_keep_the_exact_selected_input_and_executable
cargo test --lib producer_cleanup_rolls_back_when_a_callee_exit_fact_would_change
cargo test --test mir6502_cross_call_homes whole_image_cost
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes storage_observation_preserves_invocations_arguments_and_owned_memory
```

Use the shared-target/profile settings above. Run any other directly changed
case only when its changed behavior or a failure requires it.

Completion means both final predicates accept proven overall savings despite
local growth, equality/growth and invalid proofs still reject, routine-set
compatibility and exact rollback remain intact, the independent generated-code
case passes its VM oracle, and TN has a recorded outcome. Earlier candidate
filters and multiple-call scheduling are not silently folded into this change.
