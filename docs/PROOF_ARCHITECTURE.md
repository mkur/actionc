# Proof Architecture

This note records the current proof/fact-finding architecture and the intended
direction as `actionc` moves from AST lowering toward semantic-IR native
lowering.

## Current Codegen Paths

`actionc` currently has three effective lowering paths:

1. **Compat AST codegen**
   - Uses the AST-era `Generator`.
   - Prioritizes original Action!-compatible ABI, layout, and code shape.
   - Should avoid modern-only proof-driven optimization except where needed for
     correctness.

2. **Modern AST codegen**
   - Also uses the AST-era `Generator`.
   - Enables modern layout and local codegen improvements.
   - Uses the current processor-state tracker.
   - Consumes some proof objects from `codegen/proof.rs`.

3. **semIR-native codegen**
   - Consumes semantic IR and emits 6502 through `NativeTrackedEmitter`.
   - Has a native processor-state guardrail and optimization guard.
   - Does not yet consume the existing proof system.

## Current Proofs

The current proof system is mostly an AST-codegen proof system.

The proof objects live in `src/codegen/proof.rs`, but the fact finders are
methods on the AST `Generator`. They depend on AST/codegen context such as:

- `Expr`
- `StorageSlot`
- `lookup_slot`
- `expr_size`
- `lvalue_slot`
- array storage layout
- routine ABI metadata

The active consumers today are mostly in the modern AST backend:

- value availability proofs for call results, constants, storage bytes, and
  return-slot bytes;
- index/address proofs for absolute,Y and indirect,Y array lowering;
- pointer dereference proofs for choosing indirect forms or falling back to
  address arithmetic.

Some other proof structures exist as foundation and tests, but are not yet
deeply consumed by lowering:

- routine boundary proofs;
- call boundary proofs;
- zero-page temp lifetime and placement proofs.

## Desired Direction

Do not directly transplant AST `Generator` proofs into semIR-native lowering.
They are useful evidence, but they are coupled to AST-era storage and lowering
decisions.

Instead, split facts into two layers:

1. **Semantic facts**
   - Derived from semantic IR, symbols, and types.
   - Examples: value width, signedness, byte-sized index, pointer element type,
     expression purity, call/evaluation-order sensitivity.
   - These should be reusable by future modern lowering and eventually TAC/SSA.

2. **Lowering proofs**
   - Derived after modern layout and internal ABI decisions.
   - Examples: this byte is available in A, this lvalue has direct absolute
     storage, this array access can use absolute,Y, this call result does not
     need public return-slot materialization.
   - These are target/layout-specific and belong under `codegen`.

The semIR-native backend should consume semantic facts plus native lowering
proofs, guarded by `NativeTrackedEmitter` state. Modern AST codegen may keep
using its existing proof system while it remains the transitional backend.

## Rule Of Thumb

If a fact can be stated without mentioning 6502 registers, zero page, Action!
runtime slots, or final storage addresses, it probably belongs in semantic
analysis.

If a fact mentions A/X/Y, `(zp),Y`, absolute,Y, return slots, generated storage,
or call clobbers, it belongs in codegen lowering.
