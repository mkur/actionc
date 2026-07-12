# MIR6502 Compare/Branch Materialization Plan

Snapshot date: 2026-06-03.

This note is a Codex-ready implementation plan for the next MIR6502 materialization
slice after dynamic pointer word-index materialization.

It targets the remaining compare/branch failures: word and pointer comparisons,
signed `INT` relational comparisons, short-circuit booleans, and later indirect
call callable-value materialization.

Related documents:

- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_DYNAMIC_POINTER_WORD_INDEX_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/bugs/MIR6502_LOOP_CONSUMER_SPILLS_BUG.md`

## Current Snapshot

The latest fixture dump summary is:

```text
fixtures: 115
materialized MIR succeeded: 100
source listings succeeded: 94
command failures: 36
```

Representative remaining errors:

```text
signed_int_compare.materialized-mir.err:
  pre-emission MIR cannot contain word-width pseudo ops
  pre-emission MIR cannot contain virtual temp `v0`
  pre-emission MIR cannot contain virtual temp `v1`
  pre-emission MIR cannot contain virtual temp `v2`
  pre-emission MIR cannot contain abstract bool branch conditions

pointer_compare.materialized-mir.err:
  pre-emission MIR cannot contain word-width pseudo ops
  pre-emission MIR cannot contain virtual temp `v0`
  pre-emission MIR cannot contain virtual temp `v1`
  pre-emission MIR cannot contain virtual temp `v2`
  pre-emission MIR cannot contain abstract bool branch conditions

short_circuit_and.materialized-mir.err:
  pre-emission MIR cannot contain virtual temp `v3`

indirect_proc_call.materialized-mir.err:
  pre-emission MIR cannot contain virtual temp `v1`
```

The dynamic pointer word-index errors have disappeared from the sampled fixtures,
so the next highest-leverage area is compare/branch consumer materialization.

## North Star

If a compare result is consumed only by a branch, do not materialize a bool byte or
ordinary virtual temp. Materialize the compare directly into flags/control flow.

Pre-emission MIR must not contain:

```text
word-width compare pseudo ops
virtual compare temps
abstract bool branch conditions
```

A bool byte should be materialized only when the boolean is used as a value, not
when it is immediately consumed by a branch.

## Red Lines

Do not mix this slice with unrelated work.

Out of scope for the first commit:

- short-circuit boolean CFG materialization;
- indirect calls and callable values;
- call ABI changes;
- peepholes;
- branch layout optimization;
- cross-block value propagation;
- general register allocation;
- zero-page allocation;
- source-name or SemIR recovery.

Do not weaken the pre-emission verifier. The goal is to remove abstract compare
forms before pre-emission, not to allow them through.

## Milestone 1: Word Equality/Inequality Branch Consumers

Goal: materialize word-sized `Eq` / `Ne` compares that feed only a branch.

Scope:

```text
CARD Eq/Ne consumed by branch
pointer Eq/Ne consumed by branch
word-sized address values Eq/Ne consumed by branch
```

Materialization strategy:

For word equality:

```text
compare low byte
if low differs -> not equal
compare high byte
if high differs -> not equal
otherwise equal
```

For word inequality:

```text
compare low byte
if low differs -> not equal
compare high byte
if high differs -> not equal
otherwise equal
```

The exact MIR control-flow form may differ. The invariant is that the branch
condition is no longer an abstract bool temp at pre-emission.

Rules:

- Reuse byte compare materialization where possible.
- Pointer equality can use the same byte-lane equality path as `CARD` equality.
- Do not implement relational `<`, `<=`, `>`, `>=` in this milestone.
- Preserve then/else branch targets exactly.

Acceptance criteria:

- Pointer Eq/Ne compare fixtures no longer report word pseudo ops, virtual temps,
  or abstract bool branch conditions.
- Word Eq/Ne branch fixtures, if present, materialize to pre-emission.
- Existing byte compare, dynamic-index, pointer-deref, call ABI, and store-consumer
  fixtures remain green.

Suggested commit:

```text
mir6502: materialize word equality branches
```

## Milestone 2: Unsigned Word Relational Branch Consumers

Goal: materialize unsigned `CARD` relational compares that feed only a branch.

Scope:

```text
CARD Lt/Le/Gt/Ge consumed by branch
pointer unsigned ordering only if the language permits/represents it as unsigned
```

Materialization strategy:

Unsigned word relational compares should compare high byte first, then low byte
when high bytes are equal.

Conceptual shape:

```text
compare high bytes
branch if high decides result
compare low bytes
branch based on low result
```

Rules:

- Do not use signed `INT` rules here.
- Do not materialize an intermediate bool byte unless the bool is used as a value.
- Preserve branch targets exactly.

Acceptance criteria:

- Unsigned word relational branch fixtures materialize to pre-emission.
- Existing word Eq/Ne fixtures remain green.
- No abstract bool branch conditions remain for supported unsigned word compares.

Suggested commit:

```text
mir6502: materialize unsigned word branch consumers
```

## Milestone 3: Signed `INT` Relational Branch Consumers

Goal: materialize signed `INT` relational compares that feed only a branch.

Scope:

```text
INT Lt/Le/Gt/Ge consumed by branch
INT Eq/Ne if not already covered by word equality
```

Materialization strategy:

Signed 16-bit compare should not reuse unsigned relational logic blindly. It must
preserve two's-complement signed ordering.

Acceptable strategies include:

```text
sign-bit split:
  compare high-byte sign bits first;
  if signs differ, negative is less;
  if signs same, use unsigned word comparison;

or a verified target-specific signed compare sequence that preserves the same
semantics.
```

Rules:

- Keep this separate from unsigned `CARD` relational compares.
- Add focused fixtures for negative/positive and same-sign cases.
- Do not rely on host integer comparison at emission time.
- Do not leave word pseudo ops or bool temps in pre-emission MIR.

Acceptance criteria:

- `signed_int_compare.materialized-mir.err` disappears.
- Tests cover at least: negative < positive, positive > negative, same-sign less,
  same-sign greater/equal.
- Existing unsigned word compare fixtures remain green.

Suggested commit:

```text
mir6502: materialize signed int branch consumers
```

## Milestone 4: Short-Circuit Boolean Branch Conditions

Goal: eliminate virtual bool temps left by short-circuit `AND` / `OR` control
flow.

Scope:

```text
short_circuit_and
short_circuit_or
return_from_if if it uses branch-produced bool temps
```

Rules:

- Prefer CFG/control-flow materialization over bool-byte materialization when the
  result is immediately consumed by a branch.
- If the boolean is truly used as a value, materialize it as an explicit byte in a
  known home.
- Do not perform branch layout optimization in this milestone.

Acceptance criteria:

- `short_circuit_and.materialized-mir.err` disappears.
- `short_circuit_or.materialized-mir.err` disappears.
- Branch targets remain semantically correct.
- Existing compare branch fixtures remain green.

Suggested commit:

```text
mir6502: materialize short-circuit branch conditions
```

## Milestone 5: Indirect Call Targets And Callable Values

Goal: route callable values into indirect call target homes instead of leaving
virtual temps in pre-emission.

Scope:

```text
indirect procedure calls
indirect byte function calls
indirect word function calls
callable parameter forwarding
```

This milestone should be done only after compare/branch materialization is stable,
because it is a distinct callable-value materialization path.

Rules:

- Indirect call target must be a typed 16-bit callable value before emission.
- Do not recover call signatures from source names or SemIR during emission.
- Preserve conservative effects unless precise effects are available.
- Do not implement call inlining or peepholes.

Acceptance criteria:

- `indirect_proc_call.materialized-mir.err` disappears.
- `indirect_func_call_byte.materialized-mir.err` disappears.
- `indirect_func_call_word.materialized-mir.err` disappears.
- Direct call ABI fixtures remain green.

Suggested commit:

```text
mir6502: materialize indirect call targets
```

## Milestone 6: Refresh Fixture Dump And Re-bucket

Goal: measure the impact and identify the final remaining clusters.

Run:

```sh
scripts/dump_mir6502_fixtures.sh
```

Record:

```text
fixtures total
materialized MIR successes
source-listing successes
command failures
remaining error filenames grouped by diagnostic text
```

Expected progress:

- materialized MIR successes increase from 100;
- source-listing successes increase from 94 or do not regress;
- word/pointer/signed compare diagnostics disappear or become narrower;
- remaining large bucket should be indirect call/callable and any explicit
  unsupported machine/builtin cases.

Suggested commit:

```text
mir6502: refresh compare branch gap snapshot
```

## Suggested First Codex Task

```text
Implement MIR6502 word/pointer equality branch materialization.

Scope:
- Materialize word-sized Eq/Ne compare results consumed only by a branch directly
  into control-flow/flag form.
- Support CARD Eq/Ne and pointer Eq/Ne first.
- Reuse byte compare materialization for low/high byte lanes.
- Do not materialize a bool byte unless the boolean is used as a value.
- Do not implement unsigned relational, signed INT relational, short-circuit
  booleans, or indirect calls in this commit.

Acceptance:
- pointer_compare no longer reports word-width pseudo ops, virtual temps, or
  abstract bool branch conditions for Eq/Ne cases.
- word Eq/Ne branch fixtures, if present, materialize to pre-emission.
- Existing byte compare, dynamic-index, pointer deref, direct-call, and
  store-consumer fixtures remain green.

Required checks:
- cargo test -q mir6502 --lib
- cargo test -q mir6502_fixtures_match_snapshots
- scripts/dump_mir6502_fixtures.sh

Suggested commit:
- mir6502: materialize word equality branches
```
