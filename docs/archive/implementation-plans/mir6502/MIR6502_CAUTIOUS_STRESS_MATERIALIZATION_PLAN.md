# MIR6502 Cautious Stress Materialization Plan

Snapshot date: 2026-06-03.

This plan is intentionally conservative. It is for the next MIR6502
materialization work after the refreshed stress dump, where the old address
consumer failures appear to have narrowed down to mostly virtual temp and
virtual lane legalization. Do not treat that observation as proof that the whole
address materialization story is complete. Treat it only as the current evidence
from the current dump.

The goal is not to force all stress fixtures green quickly. The goal is to make
one small, explainable, verified improvement at a time, without weakening the
MIR contracts or hiding diagnostics.

Related documents:

- `docs/MIR6502_STRESS_MATERIALIZATION_IMPLEMENTATION_PLAN.md`
- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/MIR6502_COMPARE_BRANCH_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`

## Current Evidence Only

The refreshed stress dump reports:

```text
fixtures: 14
MIR succeeded: 12
materialized MIR succeeded: 2
source listings succeeded: 1
command failures: 27
```

Current materialization failures inspected from
`surveys/stress/outputs/mir6502/errors/*.materialized-mir.err` are mostly of
these forms:

```text
pre-emission MIR cannot contain virtual temp `vN`
pre-emission MIR cannot contain virtual temp byte `vN.b0`
pre-emission MIR cannot contain virtual temp byte `vN.b1`
pre-emission MIR cannot contain word-width pseudo ops
```

Do not infer more than that. In particular:

- do not assume every remaining `vN` has the same cause;
- do not assume `vN.b0` and `vN.b1` are always safe to split the same way;
- do not assume a fixture should be made green in one commit;
- do not assume a broad spill-all strategy is correct;
- do not assume source-listing failures are materialization failures;
- do not treat the `calls` NIR arity problem as part of materialization.

## Hard Rules

Codex must follow these rules while implementing this plan.

1. Keep the pre-emission verifier strict.
2. Do not silence diagnostics to improve dump counts.
3. Do not add fixture-specific hacks.
4. Do not introduce broad register allocation.
5. Do not introduce broad zero-page allocation.
6. Do not change raw MIR lowering unless the failing evidence proves raw MIR is
   wrong.
7. Do not combine materialization fixes with peephole optimization.
8. Do not combine materialization fixes with branch-range/emission fixes.
9. Do not make `strings` source-listing pass by changing materialized MIR unless
   materialized MIR is actually wrong.
10. Commit after each narrow, tested improvement.

If a change requires guessing about semantics, stop and add a diagnostic or a
small characterization test instead.

## Working Method

For every candidate fix, use this loop:

```text
1. Pick one fixture.
2. Pick one routine/block.
3. Pick one surviving temp or lane.
4. Inspect raw MIR and materialized MIR around that value.
5. Identify the producer and the consumer.
6. Decide the smallest legal consumer home.
7. Add a focused test or fixture expectation.
8. Implement only that case.
9. Run narrow tests.
10. Run the stress dump.
11. Commit if the result is better and no unrelated output got worse.
```

A valid fix should be explainable as:

```text
Producer P creates value V.
Consumer C needs V in home H.
Materialization now routes V directly to H, or to the smallest durable home
needed to reach C safely.
```

If that sentence cannot be written honestly, the fix is too broad.

## Before Any Code Change: Add A Local Classifier

Before changing materialization logic, add or use a small classifier for the
stress errors. It can be a script, test helper, or documented command. It should
summarize current failure categories by fixture.

It should not decide correctness. It should only count diagnostics.

Minimum categories:

```text
plain virtual temp
virtual byte lane
word-width pseudo op
abstract bool branch
computed index address
dynamic word index address
dynamic pointer word index address
raw MIR / NIR failure
source-listing / emission failure
```

Acceptance criteria:

- Running the classifier after `surveys/stress/mir6502-sweep.sh` gives a
  concise table.
- The table makes it obvious whether a change reduced only the intended bucket.
- The classifier itself does not change compiler behavior.

Suggested commit:

```text
mir6502: add cautious stress failure classifier
```

## Phase 1: Single Plain Temp In `pointers`

Current observed failure:

```text
pointers: Main:bb4: pre-emission MIR cannot contain virtual temp `v22`
```

This is the first candidate because it is a single plain temp in a pointer-heavy
fixture where the older address-consumer diagnostics appear to be gone.

Steps:

1. Inspect raw MIR around `v22` in `surveys/stress/outputs/mir6502/mir/pointers.mir6502`.
2. Inspect materialized MIR around `v22` if present.
3. Identify the exact producer of `v22`.
4. Identify the exact consumer of `v22`.
5. Determine whether the consumer is:
   - final byte/word memory store;
   - pointer/address home;
   - arithmetic input;
   - branch/compare input;
   - call argument;
   - return home;
   - something else.
6. Add one focused fixture or test that isolates this shape.
7. Implement only that shape.

Do not generalize to all `vN` temps yet.

Acceptance criteria:

- `pointers.materialized-mir.err` is removed or reduced to a different, more
  specific diagnostic.
- No reappearance of computed/dynamic address diagnostics in `pointers`.
- Non-stress MIR6502 tests remain green.
- The classifier shows only the intended bucket changed.

Suggested commit:

```text
mir6502: legalize one pointer stress temp shape
```

## Phase 2: Single Plain Temp In `zero_page`

Current observed failure:

```text
zero_page: Fill:bb3: pre-emission MIR cannot contain virtual temp `v8`
```

This is the second candidate because it is also a single plain temp, but in a
zero-page flavored fixture.

Steps are the same as Phase 1. Do not assume it is the same shape as
`pointers:v22`. Prove it from MIR inspection.

Acceptance criteria:

- `zero_page.materialized-mir.err` is removed or reduced to a different, more
  specific diagnostic.
- No unrelated change in `zero_page_scalars`, which already materializes.
- No broad spill-everything fallback is introduced.

Suggested commit:

```text
mir6502: legalize one zero-page stress temp shape
```

## Phase 3: Characterize Virtual Byte Lanes Before Fixing Them

Many refreshed failures are lane failures:

```text
vN.b0
vN.b1
```

Do not immediately implement a universal lane splitter. First classify lane
producer/consumer shapes.

For each lane failure, record:

```text
fixture
routine/block
lane value
word producer
lane consumer
whether both lanes are consumed
whether the word value has other uses
whether the value crosses a call or branch
whether signedness matters
```

Start with small cases only:

```text
arrays: Neg:bb2: v0.b0 / v0.b1
records: Walk:bb7: v4.b0 / v4.b1
arithmetic_control: SignedMix:bb8: v9.b0 / v9.b1
```

Do not start with `real_expr_chains`, `advanced_pointers`, `layout_integration`,
or `control_flow`. They are broader stress cases and may combine several causes.

Acceptance criteria:

- A short note or test comments describe at least three lane shapes.
- No compiler behavior change is required in this phase.
- If code changes are made, they should only improve diagnostics or test
  observability.

Suggested commit:

```text
mir6502: characterize stress lane legalization shapes
```

## Phase 4: Implement One Lane Shape Only

After Phase 3, choose the simplest proven lane shape. Prefer a same-block,
non-call-crossing, both-lanes-consumed case.

Possible safe first shape, if confirmed by inspection:

```text
word producer in same block
both low and high lanes consumed immediately by byte stores or byte operations
no call between producer and consumers
no branch crossing producer and consumers
```

Implementation guidance:

- Materialize the word producer into the consumer homes directly if possible.
- If direct consumer routing is not possible, materialize into the smallest
  explicit word home whose low/high lanes can be loaded legally.
- Preserve signedness only where the consumer actually requires it.
- Do not route all lane temps through a global spill by default.

Acceptance criteria:

- One selected lane failure disappears.
- No new failures appear in address materialization.
- No broad output churn in unrelated fixtures.
- The implementation has a clear guard for unsupported lane shapes.

Suggested commit:

```text
mir6502: legalize one virtual lane shape
```

## Phase 5: Repeat Lane Shapes One At A Time

After the first lane shape is proven, repeat cautiously.

Recommended order:

```text
arrays Neg
records Walk
arithmetic_control SignedMix
advanced_pointers Walk/Touch only after the smaller cases
layout_integration only after local-storage lane shape is understood
real_expr_chains only after expression-chain lane shape is understood
control_flow last
```

For every new lane shape:

- add or update one focused test;
- avoid changing unrelated materialization rules;
- verify the classifier bucket count changes in the expected direction;
- commit separately.

Stop if a new lane shape needs cross-block lifetime reasoning. That belongs in a
separate plan.

## Phase 6: Remaining Word-Width Pseudo Ops

Only after the plain temp and simple lane work, inspect the remaining
`word-width pseudo ops` diagnostics. The refreshed dump suggests these are
mostly concentrated in `control_flow`, but verify this with the classifier.

Do not implement a universal word-op expansion pass blindly. For each remaining
word pseudo op:

1. identify the operation;
2. identify the consumer;
3. decide whether direct consumer materialization is possible;
4. check whether flags, signedness, or branch semantics are involved;
5. add a focused test.

Start with non-control-flow word pseudo ops if any remain. Leave cross-block
control-flow cases for later.

Acceptance criteria:

- A selected word pseudo op is eliminated without introducing lane or temp
  regressions.
- The pre-emission verifier remains unchanged.

Suggested commit:

```text
mir6502: lower one remaining word pseudo op shape
```

## Phase 7: Control Flow Last

`control_flow` still has many virtual temps and some word pseudo ops. Do not use
it as the first target.

Reasons:

- temps may cross block boundaries;
- values may be live through branch joins;
- flags may be more valuable than materialized boolean bytes;
- word pseudo ops may be tied to compare/branch lowering;
- a broad fix here can easily hide real lifetime bugs.

Before changing control-flow materialization, prepare a separate note that
classifies each remaining `control_flow` temp by producer/consumer/lifetime.

Acceptance criteria for entering this phase:

- `pointers` and `zero_page` no longer fail on single plain temps.
- At least one simple lane shape is fixed.
- The classifier shows `control_flow` is now the main remaining materialization
  cluster.

Suggested commit for the preparatory note:

```text
mir6502: document remaining control-flow materialization temps
```

## Source Listing And Calls Are Separate

`strings` appears to materialize, but source listing reports branch range
failures. Treat that as branch relaxation or emission work, not materialization.
Do not try to fix it by changing materialized MIR unless inspection shows the
MIR is wrong.

`calls` reports a NIR call arity mismatch. Treat that as NIR/call lowering or
signature modeling work, not materialization.

## Stop Conditions

Stop the current slice and commit nothing if any of these happen:

- the fix requires guessing what a temp means;
- the fix changes raw MIR without proving raw MIR is wrong;
- the fix weakens the verifier;
- the fix makes more fixtures pass but introduces less specific MIR;
- a broad fallback spill path is needed to make progress;
- the stress dump changes many unrelated files unexpectedly;
- the same code path appears to need control-flow lifetime analysis.

When in doubt, add a characterization test or a diagnostic and stop.

## Definition Of Good Progress

Good progress is not necessarily making many fixtures pass. Good progress is:

```text
one fewer unresolved value class;
one clearer diagnostic;
one better producer-to-consumer materialization rule;
no weaker verifier;
no hidden semantic guessing;
one small commit that can be reverted safely.
```

Move slowly.
