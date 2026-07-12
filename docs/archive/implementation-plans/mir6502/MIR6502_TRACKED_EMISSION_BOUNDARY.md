# MIR6502 To Tracked Emission Boundary

Snapshot date: 2026-06-01.

This note defines the interface contract between MIR6502 and the tracked emission
layer built around `NativeTrackedEmitter` and its opcode/helper facades.

Related documents:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md` is the canonical MIR6502 machine
  contract.
- `docs/MIR6502_IMPLEMENTATION_PLAN.md` is the milestone execution plan.
- `docs/SEMIR_NATIVE_EMISSION_PLAN.md` describes the current tracked-emission
  layer and its responsibilities.

## Core Rule

MIR6502 decides target strategy. Tracked emission writes concrete bytes for
already-decided actions and records the processor-state consequences.

In other words:

```text
NIR -> MIR6502 lowering -> MIR6502 materialization -> pre-emission MIR
    -> tracked emission helpers -> concrete bytes and tracked state
```

The tracker may choose how to encode a concrete action. It must not decide what
the action means.

## Layer Responsibilities

### MIR6502 owns

MIR6502 owns every decision that depends on source-language meaning, NIR storage
facts, target strategy, ABI, or lowering policy:

- mapping NIR values and places to MIR values, definitions, memory, and address
  forms;
- preserving whether storage is local, param, global, static, absolute, zero-page,
  fixed zero-page, spill, or a selected indexed/indirect form;
- deciding whether an absolute-backed symbol remains a symbolic global with
  absolute backing or is resolved to `MirMem::Absolute`;
- byte/word expansion;
- carry/borrow dependencies for expanded arithmetic;
- local A/X/Y/flags use needed by materialized MIR sequences;
- ABI argument and result homes;
- call clobbers, preserves, memory effects, stack effects, and barriers;
- runtime-helper selection;
- compare/test/branch fusion or multi-block compare lowering;
- zero-page allocation policy and fixed-vs-allocatable zero-page distinction;
- target-specific MIR peepholes, after correctness.

### Tracked emission owns

Tracked emission owns concrete bytes and state tracking:

- exact opcode bytes for a concrete pre-emission MIR action;
- direct zero-page versus absolute opcode encoding when the choice is explicitly
  byte-equivalent and semantics-preserving;
- label binding and patching;
- branch-distance repair or diagnostics;
- source-map, listing, and proof records tied to emitted bytes;
- updating known A/X/Y/flags/stack/memory state after each emitted instruction;
- conservative state invalidation after calls, raw data, machine blocks, unknown
  effects, and explicit barriers;
- small helper facades such as `emit_load_a_from_addr`, `emit_store_a_to_addr`,
  `emit_branch`, or `emit_jsr`, when their inputs are already concrete.

### Emission/tracker must not own

Tracked emission must not recover or infer facts that belong to NIR or MIR6502:

- no SemIR inspection;
- no parsing printed NIR/MIR/TAC;
- no source-name lookup;
- no deciding whether a symbol is a local, global, static, absolute alias,
  pointer dereference, field, or index;
- no deciding whether `BYTE COLOR=$02C8` is ordinary global storage or an
  absolute-backed alias;
- no deciding ABI homes from call signatures;
- no deciding call clobbers, preserves, memory effects, stack effects, or opaque
  behavior;
- no deciding whether a word operation should inline byte expansion or call a
  helper;
- no deciding whether a compare should materialize a bool byte, use flags, or
  lower to a multi-block sequence;
- no deciding whether a load/store/call/barrier may be reordered or deleted;
- no general register allocation;
- no zero-page allocation policy.

If tracked emission needs one of those facts, pre-emission MIR is not concrete
enough and the MIR pipeline must be fixed before emission.

## Interface Shape

The MIR-to-emission bridge should pass concrete target actions to tracked
emission. It should not pass source-shaped or NIR-shaped requests.

Good bridge calls look like:

```rust
emit_load_byte_to_a(src: MirAddr);
emit_store_a_to_byte(dst: MirAddr);
emit_load_imm_to_reg(dst: MirReg, value: u8);
emit_move_reg(dst: MirReg, src: MirReg);
emit_add_a_byte(src: MirValue, carry_in: MirCarryIn);
emit_and_a_byte(src: MirValue);
emit_branch(test: MirFlagTest, target: MirLabel);
emit_jump(target: MirLabel);
emit_jsr(target: MirCallTarget, effects: MirEffects);
emit_barrier(effects: MirEffects);
```

The exact Rust API may differ, but the shape must preserve the same boundary:
inputs are MIR-level addresses, registers, flag tests, labels, call targets, and
effects that have already been selected by MIR materialization.

Bad bridge calls look like:

```rust
emit_assignment(name: "COLOR", value: 4);
emit_expr(expr_id);
emit_array_access(source_syntax);
emit_call_by_semir_node(...);
emit_compare_expr(...);
emit_store_global_without_storage_facts(global_id, value);
```

Those forms require emission to rediscover semantic or MIR lowering decisions and
are not allowed.

## Address Encoding Policy

There are two separate decisions that must not be confused.

### Address strategy

MIR6502 decides the address strategy:

- direct absolute memory;
- direct global/static/local/param/spill storage;
- fixed zero-page;
- virtual zero-page lowered to concrete zero-page;
- absolute indexed;
- zero-page indexed;
- indirect indexed through a zero-page pointer pair;
- label or routine target.

The tracker must not change this strategy.

### Opcode encoding

Tracked emission may choose the exact opcode encoding for a concrete address only
when the choice is semantics-preserving.

Allowed example:

```text
MIR action: load byte from direct address $00FE
Emission: choose LDA zp instead of LDA abs if MIR did not require absolute form
          and the shorter encoding is byte-equivalent.
```

Disallowed example:

```text
MIR action: load byte from absolute-indexed address $00FE,Y
Emission: silently use zero-page-indexed form.
```

Zero-page indexed addressing wraps inside page zero, while absolute indexed
addressing does not. The tracker must not replace one with the other unless MIR
explicitly selected that strategy.

Rule:

```text
Tracked emission may choose a shorter direct zero-page encoding for known
non-indexed addresses only when MIR permits byte-equivalent encoding. It must not
change indexed, indirect, or address-staging strategy.
```

If this distinction becomes hard to enforce, add an explicit MIR flag such as:

```rust
pub enum MirEncodingPreference {
    AllowEquivalentShortForm,
    RequireAbsoluteEncoding,
    RequireZeroPageEncoding,
}
```

Do not guess in the tracker.

## Register And Flag State

MIR6502 may intentionally use A, X, Y, flags, and carry/borrow dependencies in
pre-emission MIR. Tracked emission records what happened to those resources after
writing the concrete bytes.

Tracked emission may:

- remember that A/X/Y now contain a known constant or memory byte;
- remember that flags are valid after a compare/test/arithmetic instruction;
- invalidate flags after a flag-clobbering instruction;
- invalidate registers after calls or opaque barriers;
- report an internal guardrail violation if an emission helper lies about state.

Tracked emission must not:

- invent a new register allocation;
- reorder instructions to preserve a known register value;
- repair an invalid MIR carry chain;
- decide that a flag-producing compare can be reused for a later branch if MIR did
  not make that dependency explicit.

Carry chains must be explicit in MIR. If a byte-lane `Add` or `Sub` uses
`carry_in=FromPrevious`, the pre-emission verifier must have proven that no
intervening operation clobbers flags before emission sees it.

## Effects And Barriers

MIR passes effects to emission; emission applies and records their consequences.

For calls, runtime helpers, machine blocks, raw data, OS interactions, unknown
absolute memory, stack operations, and explicit barriers:

- MIR must provide conservative `MirEffects`;
- tracked emission must invalidate known state according to those effects;
- tracked emission must not narrow effects based on source knowledge;
- tracked emission may add extra conservative invalidation if the byte sequence
  requires it.

A missing effect record is a MIR verifier error, not an emission guess.

## Labels, Layout, And Branch Distance

MIR block IDs and labels carry control-flow identity into pre-emission MIR.
Tracked emission owns final binding and patching of emitted labels.

Tracked emission may:

- bind labels to concrete addresses;
- patch relative branches;
- report branch-distance errors;
- apply a documented long-branch repair strategy if the backend supports one.

Tracked emission must not:

- change the semantic target of a branch;
- infer fallthrough semantics that are not present in MIR;
- reorder MIR blocks unless a dedicated MIR layout pass or documented emission
  layout policy owns that decision.

Block order in MIR is a layout hint, not semantic identity. If block reordering
becomes useful, prefer a MIR layout/materialization pass before emission.

## Absolute Alias Boundary Test

This program is the canonical smoke test for the boundary:

```action
BYTE COLOR=$02C8

PROC Main()
  COLOR=4
RETURN
```

MIR must preserve the absolute backing before tracked emission. Acceptable
pre-emission MIR shapes include either:

```text
global g0 COLOR: byte absolute $02C8

routine r0 Main
b0 bb0:
  store.b global g0+0, #$04
  return
```

or:

```text
routine r0 Main
b0 bb0:
  store.b absolute $02C8, #$04
  return
```

Unacceptable MIR:

```text
routine r0 Main
b0 bb0:
  store.b global g0+0, #$04
  return
```

with no table or storage fact showing that `g0` is absolute-backed.

Tracked emission must not be asked to rediscover `$02C8` from `COLOR`. If the
absolute address is missing in MIR, the bug is in NIR-to-MIR lowering, MIR
storage mapping, or the MIR printer/verifier, not in the tracker.

## Pre-Emission Requirements

Before calling tracked emission, MIR must satisfy the pre-emission verifier
profile from `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`:

- no unsupported pseudo ops;
- no unresolved storage or label references;
- no unassigned virtual temps;
- no unresolved runtime helper targets;
- no abstract zero-page slots unless emission explicitly owns final assignment;
- all raw data and machine-code boundaries represented as barriers;
- ordinary instruction work expressible through tracked emission helpers.

The MIR-to-tracker bridge should assert or require this verifier phase before
emitting bytes.

## First Implementation Slice

The first bridge slice should be deliberately small.

Scope:

- Add `src/mir6502/emit.rs` only after scalar pre-emission MIR exists.
- Require `verify_program(mir, MirPhase::PreEmission)` before emission.
- Support byte-width direct operations only:

```text
LoadImm byte
Load byte Direct
Store byte Direct
Move byte
byte Add/Sub with explicit carry
byte And/Or/Xor
Jump
Branch FlagTest
Return
Exit
Barrier
```

- Route every ordinary instruction through tracked emission helpers.
- Treat raw data and machine blocks as barriers unless structured payload support
  exists.

Acceptance criteria:

- Scalar pre-emission MIR emits through `NativeTrackedEmitter`.
- No ordinary instruction bypasses tracked emission.
- The `COLOR=$02C8` boundary test emits a store to `$02C8` because MIR already
  preserved that address.
- Missing storage/effect facts fail in MIR verification, not emission.

Suggested commit:

```text
mir6502: document tracked emission boundary
```
