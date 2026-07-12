# MIR6502 Stress Materialization Implementation Plan

Snapshot date: 2026-06-03.

This is a Codex-ready implementation plan for closing the materialization gaps
shown by the updated stress fixture dumps in
`surveys/stress/outputs/mir6502/`. The important conclusion from the fresh
outputs is that raw MIR is now mostly present and useful; the current blocker is
that the materializer does not yet lower enough expressive MIR into the strict
pre-emission contract.

Do not make raw MIR less expressive to make the verifier pass. The raw MIR is
allowed to be optimal and abstract. The fix is to grow materialization so it can
turn computed addresses, word pseudo operations, virtual temps, and abstract
boolean branches into concrete pre-emission MIR.

Related documents:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_COMPARE_BRANCH_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_DYNAMIC_POINTER_WORD_INDEX_PLAN.md`
- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`

## Evidence From The Current Stress Dumps

The current stress dump summary is:

```text
fixtures: 14
MIR succeeded: 12
materialized MIR succeeded: 1
source listings succeeded: 1
command failures: 28
```

That means most failures are not parser failures and not ordinary NIR-to-MIR
lowering failures. They are failures at the boundary where expressive raw MIR is
being checked as pre-emission MIR.

The only currently successful stress materialization is
`zero_page_scalars.mir6502`. It is a good reference for simple scalar materialized
MIR: byte operations are explicit, word stores are split into byte stores, and
loop tests lower to `flags = cmp.b ...` plus `branch fused ...`.

The current failures fall into five buckets:

```text
A. address-consumer materialization missing
B. word-width pseudo operations still present
C. virtual temps and virtual byte lanes still present
D. abstract boolean branches still present
E. NIR/call-shape errors that are not materialization failures
```

Bucket E is currently represented by `calls`, where raw MIR itself fails with a
NIR call arity mismatch. Keep that separate from this plan unless it blocks a
focused materialization test.

## North Star

Maintain two clear contracts.

Pre-materialization MIR may contain:

```text
virtual temps
computed addresses
pointer-backed/indexed address forms
word-width pseudo operations
abstract boolean values and branches
call results in abstract homes
```

Pre-emission MIR must not contain:

```text
virtual temps or virtual temp lanes
computed addresses that still require strategy
dynamic word/pointer index address forms
word-width pseudo operations
abstract boolean branches
address values that have not been staged into a concrete address home
```

The materializer owns the transition between those contracts. The emitter should
not recover semantic meaning, guess storage intent, or invent hidden
materialization strategy.

## Execution Rules For Codex

Implement in small, test-gated slices. Commit after every major slice.

For each slice:

1. pick one failure bucket only;
2. add or update focused fixtures before broad stress cleanup;
3. run narrow tests first;
4. run the stress dump after the narrow tests pass;
5. commit before starting the next slice;
6. do not mix infrastructure, emission support, and peephole optimization in one
   commit.

Suggested recurring checks:

```sh
cargo test -q mir6502 --lib
cargo test -q mir6502_fixtures_match_snapshots
scripts/dump_mir6502_fixtures.sh
surveys/stress/mir6502-sweep.sh
```

When a slice changes only stress behavior, also inspect:

```sh
surveys/stress/outputs/mir6502/README.txt
surveys/stress/outputs/mir6502/errors/*.err
surveys/stress/outputs/mir6502/mir/*.mir6502
surveys/stress/outputs/mir6502/materialized-mir/*.mir6502
```

Keep the pre-emission verifier strict. Do not silence verifier errors to improve
counts.

## Milestone 0: Add A Stress Failure Classifier

Goal: make progress measurable and prevent guessing.

Add a small script, test helper, or documented command that groups
`surveys/stress/outputs/mir6502/errors/*.materialized-mir.err` by diagnostic
class:

```text
computed index addresses must be materialized before emission
dynamic word index addresses must be materialized before emission
dynamic pointer word index addresses must be materialized before emission
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain virtual temp
pre-emission MIR cannot contain virtual temp byte
pre-emission MIR cannot contain abstract bool branch conditions
```

Acceptance criteria:

- The classifier reports counts per bucket and per fixture.
- It identifies `calls` as a raw-MIR/NIR failure, not a materialization failure.
- It is easy to run after `surveys/stress/mir6502-sweep.sh`.

Suggested commit:

```text
mir6502: classify stress materialization failures
```

## Milestone 1: Address-Consumer Materialization Core

Goal: handle the dominant failure class first.

Implement or complete a materialization path for values consumed as addresses.
This should build on `docs/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`.
The key rule is: an address consumer requests an address home; the value should
not first be forced through an ordinary word temp unless a durable value is
really needed.

Initial consumer forms:

```text
load.b *addr+offset
load.w *addr+offset
store.b *addr+offset, value
store.w *addr+offset, value
load/store computed base[index;1]+offset
load/store computed base[index;2]+offset
```

Initial address homes:

```text
fixed zero-page source pointer pair
fixed zero-page destination pointer pair
```

Use separate source and destination pairs when a store needs to preserve the
computed destination while evaluating the RHS.

Acceptance criteria:

- `pointers.materialized-mir.err` loses the diagnostics about computed index
  addresses and dynamic word index addresses in the simple routines first
  (`ReadInc`, `StoreThrough`, `CopyCard`).
- Materialized MIR shows explicit address staging before indirect loads/stores.
- Destination address staging is preserved across RHS evaluation when RHS can
  clobber address registers or call-return registers.
- Existing non-stress MIR6502 fixtures remain green.

Suggested commit:

```text
mir6502: materialize address consumers to pointer pairs
```

## Milestone 2: Computed Byte Index Addresses

Goal: clear byte-indexed computed addresses before tackling all word-index cases.

Support `scale=1` computed addresses for byte arrays, strings, and byte
pointers. Examples from stress raw MIR include byte stores such as:

```text
store.b computed base[index;1]+0, value
load.b computed base[index;1]+0
```

Rules:

- If base is an address value, stage base directly to a pointer pair and add the
  byte index.
- If the target is an absolute/global/local byte array and the backend can use
  direct indexed addressing safely, materialize to that direct indexed form.
- If RHS evaluation can clobber `X`/`Y` or the selected pointer pair, preserve the
  destination index/address before evaluating RHS.
- Keep this as materialization, not a peephole.

Primary fixtures:

```text
pointers
strings
arrays
layout_integration
zero_page
```

Acceptance criteria:

- `computed index addresses must be materialized before emission` decreases
  substantially in the stress errors.
- `strings` no longer fails only because byte string/array indexes survive.
- Same-index simple byte copies stay compact where already supported.

Suggested commit:

```text
mir6502: materialize computed byte index addresses
```

## Milestone 3: Dynamic Word And Pointer Word Index Addresses

Goal: handle `scale=2` and pointer-word-index address forms.

Support computed addresses where the element size is a word:

```text
load.w computed base[index;2]+0
store.w computed base[index;2]+0, value
```

Also support the stress-specific diagnostic class:

```text
dynamic pointer word index addresses must be materialized before emission
```

Rules:

- Byte index to word offset can be scaled with an explicit low/high carry path.
- Prefer direct low/high byte-lane access once the final address is staged.
- Preserve destination address across RHS materialization and calls.
- Do not introduce global zero-page allocation in this slice.

Primary fixtures:

```text
pointers
arrays
real_expr_chains
advanced_pointers
layout_integration
zero_page
```

Acceptance criteria:

- `dynamic word index addresses must be materialized before emission` is gone or
  limited to clearly unsupported edge cases.
- `dynamic pointer word index addresses must be materialized before emission` is
  gone or limited to clearly unsupported edge cases.
- Word element loads/stores lower to explicit byte-lane operations through staged
  addresses.

Suggested commit:

```text
mir6502: materialize dynamic word index addresses
```

## Milestone 4: Word-Width Pseudo Operation Expansion

Goal: remove `.w` pseudo operations from pre-emission MIR.

Implement direct byte-lane expansion for common word operations that currently
survive materialization:

```text
store.w dst, #const
store.w dst, src_word
load.w src
word add/sub with carry propagation
word bitwise and/or/xor
word neg
word compare inputs used by branches
```

This should be consumer-driven. For example, a word expression immediately stored
to memory should produce stores to the final low/high destination lanes, not an
ordinary spill followed by a copy.

Acceptance criteria:

- The diagnostic `pre-emission MIR cannot contain word-width pseudo ops` drops
  sharply in `pointers`, `arrays`, `zero_page`, `real_expr_chains`,
  `advanced_pointers`, `layout_integration`, and `control_flow`.
- Word stores through staged pointer pairs are emitted as low/high byte stores.
- Existing scalar fixtures keep their current compact output.

Suggested commit:

```text
mir6502: expand word pseudo ops during materialization
```

## Milestone 5: Virtual Temp And Lane Legalization

Goal: remove remaining `vN`, `vN.b0`, and `vN.b1` from pre-emission MIR.

This milestone should come after address consumers and word pseudo expansion,
because many temp errors are downstream of those missing paths.

Legal homes:

```text
A register for byte values
fixed scratch byte/word locations when a value must survive
return home for immediate return consumers
call argument homes for immediate call consumers
source/destination pointer pairs for address consumers
final memory destination for store consumers
```

Rules:

- Prefer consumer homes over generic spills.
- Only spill when the value has multiple uses, crosses a call/barrier, or lacks a
  specific consumer home.
- Treat `vN.b0` and `vN.b1` as lane views of a word value. Legalize them by
  materializing the producing word into a known word home or directly into the
  consuming byte operation.

Primary fixtures:

```text
records
arithmetic_control
control_flow
all remaining stress failures after Milestones 1-4
```

Acceptance criteria:

- `records` materializes cleanly or fails only on a more specific non-temp
  diagnostic.
- `pre-emission MIR cannot contain virtual temp byte` is eliminated from stress
  outputs or limited to documented unsupported producers.
- No verifier weakening.

Suggested commit:

```text
mir6502: legalize virtual temps after materialization
```

## Milestone 6: Abstract Boolean Branch Lowering

Goal: lower `branch bool v ? ...` before pre-emission.

Use the already successful `zero_page_scalars` shape as the model:

```text
flags = cmp.b value relation rhs
branch fused ... ? true_bb : false_bb
```

Support:

```text
bool temps produced by byte comparisons
bool temps produced by word comparisons
zero/nonzero tests
boolean values stored as 0/1 when required by a value consumer
boolean values consumed directly by branch when no materialized 0/1 is needed
```

Primary fixtures:

```text
strings
arithmetic_control
control_flow
```

Acceptance criteria:

- `pre-emission MIR cannot contain abstract bool branch conditions` is removed
  from stress outputs.
- Branch consumers use flags directly when possible.
- Boolean store consumers still materialize explicit `0`/`1` bytes.

Suggested commit:

```text
mir6502: lower abstract boolean branches before emission
```

## Milestone 7: Fix The Calls Raw-MIR Failure Separately

Goal: handle the one non-materialization failure class.

`calls` currently reports NIR call arity mismatch before it becomes useful for
materialization diagnosis. Investigate it separately from materialization.

Likely areas:

```text
call signature modeling
optional/default argument handling
staged call argument lowering
NIR verifier expectations vs MIR call representation
```

Acceptance criteria:

- `calls` produces raw MIR.
- Once raw MIR is available, classify any remaining materialization failures into
  the same buckets as the other stress fixtures.

Suggested commit:

```text
nir: fix stress call arity lowering
```

## Milestone 8: Stress Closure And Regression Guardrails

Goal: make the stress dump useful as a regular guardrail.

After the materialization buckets are closed, update or add documentation with a
fresh summary:

```text
fixtures: 14
MIR succeeded: 14
materialized MIR succeeded: 14
source listings succeeded: 14
command failures: 0
```

If that target is not reached because a fixture intentionally exercises an
unsupported language feature, document the exception explicitly and keep the
error targeted.

Acceptance criteria:

- `surveys/stress/mir6502-sweep.sh` is green or has documented expected
  failures.
- `scripts/check-stress-fixtures.sh` remains green according to its existing policy.
- The pre-emission verifier still catches unresolved temps, addresses, word ops,
  and abstract branches.
- The final commits do not weaken raw MIR expressiveness.

Suggested commit:

```text
mir6502: close stress materialization gap
```

## Recommended Work Order

Start with:

```text
pointers -> arrays -> strings -> records -> arithmetic_control -> control_flow
```

Use `advanced_pointers`, `real_expr_chains`, and `layout_integration` as later
stress targets. They combine several failure modes and are too broad for the
first implementation slice.

The practical sequence is:

1. classify errors;
2. implement address-consumer staging;
3. handle byte computed indexes;
4. handle word/pointer word computed indexes;
5. expand word pseudo ops;
6. legalize remaining temps and lanes;
7. lower abstract bool branches;
8. fix `calls` separately;
9. refresh the dump summary.

The main rule: keep raw MIR expressive, keep pre-emission MIR strict, and make
materialization the explicit bridge between them.
