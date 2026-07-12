# MIR6502 SSA-Lite Plan

Snapshot date: 2026-06-09.

The structural peephole layer now catches many exact short patterns, but the
remaining TN bloat is mostly the same underlying issue repeated in different
surface shapes: a short-lived value is copied into a spill or scratch home and
then read back by a nearby consumer. SSA-lite should model those value facts
directly, without committing to a full SSA conversion or cross-block phi
construction yet.

## Goal

Reduce MIR6502 load/store traffic and spill pressure by tracking block-local
byte value equivalence through materialization:

- avoid one-write/one-read scratch and spill homes;
- forward stable byte sources into stores, compares, binary ops, calls, and
  X/Y loads;
- keep A/X/Y and private ZP scratch facts alive only while locally provable;
- leave joins, calls, machine blocks, hardware aliases, and unknown absolute
  memory conservative.

## Non-Goals

- No full CFG SSA form.
- No phi nodes.
- No cross-block value propagation in the first phase.
- No speculative alias analysis for raw absolute memory or hardware registers.
- No source-name-specific TN rewrites.
- No replacement for the existing exact peepholes until SSA-lite proves itself.

## Core Model

For each basic block, maintain a small value environment while scanning ops:

```text
A fact:        A == ValueKey
X/Y facts:     X == ValueKey, Y == ValueKey
memory facts:  MirMem == ValueKey for compiler-owned byte homes
```

`ValueKey` should start small:

- `ConstU8(n)`
- `DirectMem(MirMem)` for stable byte memory sources
- `Reg(MirReg)` only as a transient source when the register is still known
- later: simple `Unary`/`Binary` expression keys if useful

Memory facts are valid only for byte-width compiler-owned homes:

- locals;
- params;
- spills;
- virtual/fixed ZP scratch known to the compiler;
- globals/statics only when the existing layout rules say idempotent access is
  safe.

Raw absolute addresses should be excluded at first unless they are already
classified as safe zero-page source homes by existing MIR6502 rules.

## Invalidation Rules

Kill facts conservatively:

- writes to a memory home kill facts for that home;
- writes to A/X/Y kill the corresponding register fact;
- calls/runtime helpers kill registers according to ABI clobbers and all
  memory facts unless effects prove otherwise;
- barriers and machine blocks kill all facts;
- indirect stores kill pointer-related facts and any memory facts that may
  alias;
- writes to fixed pointer scratch kill `$AC/$AD` or related pointer facts;
- branch terminators do not export facts in phase 1.

The first implementation should prefer false negatives over false positives.

## Slice 1: Infrastructure Only

Add a block-local SSA-lite scanner behind the structural peephole pipeline:

- define `ValueKey`, `ValueEnv`, and invalidation helpers;
- scan a block and update facts;
- collect optional stats for facts learned/killed;
- do not rewrite code yet.

Acceptance:

- existing MIR6502 tests pass;
- focused unit tests cover fact learning and invalidation;
- peephole reporting can later include SSA-lite counters.

Suggested commit:

```text
mir6502: add block-local ssa-lite value tracker
```

## Slice 2: Forward Direct RHS Into Consumers

Use SSA-lite facts to rewrite byte consumers when the original source is still
stable:

- `LDA src; STA tmp; LDA lhs; CMP tmp` -> `LDA lhs; CMP src`;
- `LDA src; STA tmp; LDA lhs; ADC/SBC/AND/... tmp` -> direct RHS;
- `LDA src; STA tmp; LDA tmp; STA dst` -> direct copy or reload removal;
- `LDA src; STA tmp; LDX/LDY tmp` -> `LDX/LDY src` when tmp is dead.

This should subsume several current exact staged-RHS peepholes over time, but
do not remove those peepholes in the same slice.

Acceptance:

- direct MIR tests for each consumer class;
- negative tests for later tmp reads, calls, barriers, and unsafe memory;
- TN peephole/quality report shows rule counts and load/store movement.

Suggested commit:

```text
mir6502: forward ssa-lite byte sources into consumers
```

## Slice 3: Store Forwarding and Dead Scratch

Use facts to remove redundant reloads and dead stores more generally:

- skip `LDA tmp` when A is already known equal to tmp;
- remove private scratch stores when the value is forwarded to all consumers;
- keep the original store when memory visibility or later reads require it.

This is where the current adjacent `STA m; LDA m` and dead private scratch
cleanup can become less pattern-specific.

Acceptance:

- no removal of stores to unsafe absolute globals;
- no flag-sensitive reload removal unless flags are dead or overwritten;
- direct MIR tests for register/flag liveness.

Suggested commit:

```text
mir6502: use ssa-lite facts for store forwarding
```

## Slice 4: Pointer and Index Facts

Extend the value environment with tiny target-specific facts:

- `$AC/$AD` contains known pointer;
- `$AE/$AF` contains known pointer/index pointer;
- Y contains known byte index.

Use these facts to avoid restaging pointer pairs and reloading Y across short
array/string/pointer runs.

Acceptance:

- only within one block;
- kill on calls, barriers, indirect stores, and scratch writes;
- direct MIR and source-level array/pointer tests.

Suggested commit:

```text
mir6502: track block-local pointer and index facts
```

## Slice 5: Replace Redundant Exact Peepholes

After SSA-lite has equivalent test coverage and metrics, simplify the structural
peephole layer:

- keep target-specific idiom folds such as word inc/dec and indirect compound
  ops;
- remove exact folds that are fully subsumed by SSA-lite;
- keep peephole stats names or map old names to new SSA-lite counters so TN
  metric continuity is not lost.

## Observability

SSA-lite should report counters through the same peephole reporting path:

```text
ssa-lite-facts-learned
ssa-lite-facts-killed
ssa-lite-consumer-forwards
ssa-lite-reloads-removed
ssa-lite-dead-stores-removed
```

For TN work, run:

```sh
ACTIONC_MIR6502_PEEPHOLES=per-routine \
  cargo run --quiet --bin actionc-emit -- --backend mir6502 --emit-listing samples/tn/modern/TN.ACT \
  > target/tn-mir.lst

cargo run -q --bin actionc-listing-quality -- target/tn-mir.lst
```

Compare:

- instruction count;
- code bytes;
- LDA+STA instruction and byte percentage;
- spill data label count;
- adjacent `STA m; LDA m` pairs;
- per-routine spill pressure, especially `SetWin`.

## Stop Rules

Do not expand SSA-lite when a rewrite needs:

- facts from multiple predecessors;
- join equality proofs;
- non-local alias reasoning;
- call effect precision not represented in MIR;
- flag liveness beyond the current local helpers.

Those cases belong in a later full dataflow/SSA phase, not in SSA-lite.

## Next Phase: Higher-Level MIR Optimization

After the materializer refactor and structural peephole cleanup, avoid growing a
large catalog of highly specialized materialization paths. That would recreate
classic codegen behavior one final instruction shape at a time. The next useful
work should move optimization earlier, where MIR still has semantic structure
and can prove rewrites that were too fragile in the legacy backend.

The goal is to reduce load/store traffic, spill pressure, and repeated address
setup before final 6502 materialization chooses concrete instructions.

### Preferred Direction

1. Build SSA-lite v2 as a MIR fact/dataflow layer, not another exact peephole
   matcher.
2. Prefer copy propagation, memory value facts, call-aware liveness, and address
   expression reuse over target-shaped special cases.
3. Keep target-specific peepholes only for true 6502 idioms such as word
   inc/dec, indirect compound ops, or branch/flag encodings.
4. Preserve observability: every new higher-level rewrite should report counts
   through the existing peephole/quality metrics path.

### Slice A: Observability-Only Fact Scanner

Add or extend a MIR pass that scans each basic block and reports facts without
rewriting code:

- temp aliases learned;
- direct memory value facts learned;
- facts killed by calls, stores, barriers, machine blocks, and unknown effects;
- loads that appear replaceable;
- temp aliases that appear replaceable.

This should run before spill placement/coloring so it can explain likely spill
pressure wins before codegen changes are enabled.

### Slice B: MIR Copy Propagation And Dead Temp Elimination

Implement conservative MIR-level copy propagation:

- replace temp uses with their producer when the producer is pure and still
  valid;
- eliminate dead one-write/one-read temp definitions;
- propagate through single-consumer move/load/store chains;
- stop at calls, machine blocks, unknown memory effects, and unsafe aliases.

This attacks the one-write/one-read temp pattern without caring what 6502
instruction sequence would have been emitted.

### Slice C: Memory SSA-Lite For Compiler-Owned Homes

Track stores to known compiler-owned byte homes as value definitions:

- locals;
- params;
- spills;
- zero-page and fixed zero-page scratch owned by MIR6502.

Use those facts to replace later loads when no intervening write/call/alias can
invalidate the home. Start with byte-width facts only. Exclude raw absolute
memory and pointer-derived stores until alias behavior is represented explicitly.

### Slice D: Call ABI-Aware Argument Sinking

Move call argument materialization as late as possible:

- keep arguments as MIR values until the call boundary;
- materialize directly into A/X/Y/fixed ZP/ABI homes when safe;
- avoid routine storage staging when no intervening use requires it;
- kill facts according to the callee ABI/effects model.

This should help TN's call-heavy routines without adding routine-specific
materialization paths.

### Slice E: Address Expression Reuse

Recognize repeated MIR address expressions before they are lowered:

- repeated base pointer plus constant offset;
- repeated byte index into the same array/string;
- repeated word-array byte index scale;
- pointer/index pairs prepared once and consumed by adjacent loads/stores.

Represent these as reusable MIR address facts rather than hard-coding more final
instruction sequences. The materializer should then choose `(zp),Y`, absolute
offset, or scratch staging based on those facts.

### Cross-Block Expansion

Only after the block-local version is reliable, extend facts across simple CFG
edges:

- single-predecessor propagation first;
- join facts only when all predecessors provide the same fact;
- kill facts conservatively at calls and unknown memory effects;
- do not introduce phi nodes in SSA-lite.

If optimization starts needing loop-carried values, non-local alias reasoning,
or real phi construction, stop and design a true SSA/dataflow phase instead of
stretching SSA-lite.
