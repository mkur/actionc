# MIR6502 Indirect Call Target Materialization Plan

Snapshot date: 2026-06-03.

This note is a Codex-ready implementation plan for the next MIR6502 materialization
slice after compare/branch materialization.

It targets the remaining callable-value / indirect-call failures in the fixture
dump.

Related documents:

- `docs/MIR6502_COMPARE_BRANCH_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/bugs/MIR6502_CALL_ABI_FIRST_BYTE_ARG_BUG.md`

## Current Snapshot

The latest fixture dump summary is:

```text
fixtures: 116
materialized MIR succeeded: 107
source listings succeeded: 101
command failures: 24
```

Representative remaining errors:

```text
indirect_proc_call.materialized-mir.err:
  mir6502 Main:bb1: pre-emission MIR cannot contain virtual temp `v1`

indirect_func_call_byte.materialized-mir.err:
  mir6502 Main:bb1: pre-emission MIR cannot contain virtual temp `v2`

indirect_func_call_word.materialized-mir.err:
  mir6502 Main:bb1: pre-emission MIR cannot contain virtual temp `v2`

callable_param_forwarding.materialized-mir.err:
  mir6502 Invoke:bb1: pre-emission MIR cannot contain virtual temp `v0`
```

The sampled signed compare and short-circuit errors disappeared, so the next
high-leverage cluster is callable values consumed by indirect calls.

## Goal

Materialize typed callable values into the indirect-call target home before
pre-emission.

A callable value consumed by an indirect call must not survive as a virtual temp.
The materializer should produce a target-shaped indirect call form carrying:

```text
call target home
call signature / ABI plan
argument homes
result home, if any
effects / clobbers / preserves
```

Emission must not recover callable meaning, source names, or signatures.

## Red Lines

Do not mix this slice with unrelated call optimization.

Out of scope:

- call inlining;
- devirtualizing indirect calls to direct calls;
- broad register allocation;
- peepholes;
- SArgs redesign;
- general procedure pointer optimization;
- recovering signatures from SemIR or source names during emission;
- changing direct-call ABI behavior except where shared helpers require small
  cleanup.

If MIR lacks a typed callable signature for an indirect target, fail with a
precise diagnostic before emission rather than guessing.

## Background Rules

MIR6502 owns call target strategy and ABI homes. Tracked emission owns concrete
bytes and state tracking only.

Callable values are word-sized address values, but they are not ordinary word
values: they carry a required routine/function signature. The indirect-call path
must keep that signature available until ABI planning is complete.

Direct calls should already route arguments through ABI homes. This plan extends
that machinery to indirect call targets.

## Milestone 1: Represent Indirect Call Target Homes Explicitly

Goal: add or finalize the MIR form used by pre-emission indirect calls.

Scope:

- Ensure `MirCallTarget` or the equivalent call representation has an explicit
  indirect callable target form.
- The target form should name the selected callable target home, not an arbitrary
  virtual temp.
- The target must carry or reference the typed signature needed for ABI planning.

Conceptual shape:

```text
call indirect target=<callable_home> signature=<sig> args=[...] result=... effects=...
```

Rules:

- Do not represent pre-emission indirect calls as `call *vN` where `vN` is still a
  virtual temp.
- Do not recover the signature from a source name in emission.
- Keep routine address values and callable variable values distinct from untyped
  word values when needed.

Acceptance criteria:

- The MIR printer can show indirect call target homes clearly.
- The pre-emission verifier rejects indirect calls whose target remains a virtual
  temp or lacks a callable signature.
- Direct call fixtures remain green.

Suggested commit:

```text
mir6502: represent indirect call target homes
```

## Milestone 2: Materialize Routine Address Values For Indirect Calls

Goal: support the simplest indirect call case where the callable value is a known
routine address or a variable assigned a routine address.

Scope:

- Materialize routine address values into the selected indirect-call target home.
- Support procedure-call targets first.
- Preserve the callable signature from MIR/NIR facts.

Rules:

- Do not devirtualize to a direct call in this milestone.
- Do not spill the callable address to ordinary word storage unless the selected
  target home requires it.
- If the callable value is already in a compatible home, reuse it.

Acceptance criteria:

- `indirect_proc_call.materialized-mir.err` disappears.
- Pre-emission MIR has no virtual temp for the indirect procedure target.
- Source listing/object emission succeeds for the indirect procedure fixture or
  fails only on a more specific emission unsupported-form diagnostic.

Suggested commit:

```text
mir6502: materialize indirect procedure targets
```

## Milestone 3: Materialize Indirect Function Targets And Result Homes

Goal: support indirect function calls returning byte and word values.

Scope:

- Materialize callable target homes for indirect functions.
- Reuse existing direct-call result-home logic for byte and word results.
- Ensure result bytes are materialized into the consumer home after the call.

Rules:

- Function result ABI homes must be selected before emission.
- Word results must be low/high byte homes, not word-width pseudo values.
- Do not leave callable target or result as virtual temps in pre-emission MIR.

Acceptance criteria:

- `indirect_func_call_byte.materialized-mir.err` disappears.
- `indirect_func_call_word.materialized-mir.err` disappears.
- Direct function return fixtures remain green.
- Existing direct-call ABI fixtures remain green.

Suggested commit:

```text
mir6502: materialize indirect function targets
```

## Milestone 4: Callable Parameter Forwarding

Goal: support passing a callable value as a parameter and invoking/forwarding it
without leaking virtual temps.

Scope:

- Materialize callable parameters into callable homes.
- Forward callable values through ABI homes where required.
- Preserve callable signatures through parameter storage or ABI records.

Rules:

- Callable parameter forwarding must not erase the signature.
- If a callable parameter is stored in routine-local parameter storage, its value
  is still typed callable data, not an untyped word.
- Emission must not inspect source names to recover the callable target.

Acceptance criteria:

- `callable_param_forwarding.materialized-mir.err` disappears.
- Callable parameter fixture source listing succeeds or fails only on a more
  specific unsupported emission form.
- Indirect procedure/function fixtures remain green.

Suggested commit:

```text
mir6502: materialize callable parameter forwarding
```

## Milestone 5: Emit Indirect Call Forms If Needed

Goal: ensure the new pre-emission indirect-call forms have a tracked-emission
bridge.

Scope:

- Emit selected indirect call sequence for the supported target home.
- Apply call effects conservatively.
- Preserve direct-call emission behavior.

Rules:

- Emission receives an already-selected target home and ABI plan.
- Emission must not infer signatures, argument homes, or effects.
- If the target home cannot yet be emitted, report a precise unsupported form.

Acceptance criteria:

- Indirect procedure/function source-listing fixtures succeed if all required
  emission forms exist.
- If not, materialized MIR succeeds and source-listing failure names the missing
  indirect-call emission form precisely.

Suggested commit:

```text
mir6502: emit indirect call targets
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

- materialized MIR successes increase from 107;
- source-listing successes increase from 101 or do not regress;
- indirect call / callable virtual-temp diagnostics disappear;
- remaining failures should likely be machine-block, OS/builtin, aggregate
  initialization, or explicit unsupported emission cases.

Suggested commit:

```text
mir6502: refresh indirect call gap snapshot
```

## Suggested First Codex Task

```text
Implement MIR6502 indirect procedure target materialization.

Scope:
- Add or finalize an explicit indirect call target home in MIR.
- Materialize typed callable values consumed by indirect PROC calls into that
  target home before pre-emission.
- Preserve callable signatures for ABI planning.
- Do not devirtualize indirect calls into direct calls.
- Do not implement indirect FUNC results in the same commit unless the helper
  naturally supports them without broadening the change.
- Do not implement call inlining, peepholes, or general register allocation.

Acceptance:
- indirect_proc_call.materialized-mir.err disappears.
- Pre-emission MIR for the fixture contains no virtual temp for the call target.
- Direct call ABI fixtures remain green.
- Existing compare/branch, dynamic-index, pointer deref, and store-consumer
  fixtures remain green.

Required checks:
- cargo test -q mir6502 --lib
- cargo test -q mir6502_fixtures_match_snapshots
- scripts/dump_mir6502_fixtures.sh

Suggested commit:
- mir6502: materialize indirect procedure targets
```
