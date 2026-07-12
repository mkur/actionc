# TAC Cleanup Plan

Snapshot date: 2026-05-31.

Progress:

- Milestone 1 started: numeric absolute `SET` directives lower to typed
  `Store` operations through `TacPlaceKind::Absolute`, and legacy `TacOp::Set`
  is verifier-rejected.
- Routine-local declarations and notes are now routine metadata instead of
  executable block operations; legacy metadata ops are verifier-rejected in
  blocks.
- `FOR` loop step updates lower to normal load/add/store TAC for stable places,
  and the old stringly `ForStep` compound op is verifier-rejected.
- Static string data now carries byte payloads plus display text, and
  `StaticAddr` values are checked against the static table.
- Branch terminators now verify that conditions are bool/condition values or
  literal `0`/`1`.

Goal: make TAC a clean, explicit input for the 6502 MIR by removing legacy
escape hatches and adding first-class support for required language/backend
features that are currently represented as strings, summaries, or compatibility
nodes.

This plan is implementation-oriented. Every slice should keep these green:

- `cargo test tac_fixtures_match_snapshots`
- `cargo run --bin actionc-tac-sweep -- fixtures/tac`
- `actionc-emit --emit-tac <fixture>` for any fixture touched by the slice

Each cleanup step must also tighten the verifier so the removed legacy shape
cannot silently reappear.

## Target Boundary

The cleanup is complete when verifier-clean TAC can be consumed by a 6502 MIR
without consulting legacy operands or parsing printable strings.

Required properties:

- all executable value positions use `TacValue`;
- all storage positions use structured `TacPlace`;
- every temp has one typed definition and all uses are dominated by it;
- params, locals, globals, blocks, statics, and machine blocks have stable IDs
  or fact-table entries;
- all control flow uses structured terminators;
- calls and machine blocks carry structured effects;
- `SET`, `FOR`, array indexing, record fields, pointer dereference, string
  literals, and inline machine code have explicit TAC forms;
- verifier-clean TAC contains no executable `TacOperand`, expression summary,
  call summary, unresolved name, stringly operation name, or unknown boundary.

## Cleanup Order

### 1. Split Metadata From Executable TAC

Problem:

- `Define`, `Declare`, `Note`, and `TacGlobal.kind` are printable metadata mixed
  into block operations.
- The MIR boundary has to know which ops are executable and which are
  observation-only.

Plan:

- Add structured metadata tables for defines, declarations, routine notes, and
  source annotations.
- Keep printed output stable initially by teaching `TacPrinter` to print the
  new tables.
- Stop inserting non-executable declarations into `TacBlock.ops`.
- Add verifier checks that executable blocks contain only executable ops and
  explicit barrier ops.

Exit criteria:

- `TacOp::Define`, `TacOp::Declare`, and `TacOp::Note` are gone or verifier-
  rejected in executable blocks.
- `TacGlobal.kind` is replaced by structured global facts.

### 2. Make Symbol And Storage Facts Explicit

Problem:

- `TacPlaceKind::Symbol(String)` collapses globals, params, locals, arrays,
  records, routine variables, and builtins into one name-shaped place.
- `TacRoutine.params` and `TacRoutine.locals` are strings rather than storage
  facts.

Plan:

- Add `TacSymbol`, `TacParam`, `TacLocal`, and `TacGlobalStorage` records with
  stable IDs, type, storage class, size, and layout/address facts when known.
- Replace executable `Symbol(String)` places with:
  - `Param(ParamId)`
  - `Local(LocalId)`
  - `Global(SymbolId)`
  - `Absolute(u16)` where the source semantically names fixed memory
- Keep display names as metadata on the fact records.
- Add name-to-ID resolution inside the lowerer; do not make the MIR resolve TAC
  names.

Exit criteria:

- MIR-relevant places do not require string lookup.
- Verifier rejects `TacPlaceKind::Symbol` in executable `Load`, `Store`, and
  `AddrOf`.

### 3. Replace Legacy `SET`

Problem:

- `TacOp::Set` uses legacy `TacOperand` for both address and value.
- Top-level runtime setup currently appears as printable `set $491 = $3000`.

Required feature:

- Explicit fixed-memory writes for Action `SET`.

Plan:

- Add `TacOp::StoreAbsolute { address: u16, src: TacValue, ty: TacType }` or
  represent it as `Store { place: Absolute(address), ... }`.
- Lower numeric `SET` address/value pairs directly to absolute stores.
- For `SET * = ...` or non-literal forms, add a named compatibility barrier
  until SemIR can express the precise meaning.
- Decide whether `SET` participates in normal routine order or belongs to a
  module initialization/precode segment, then encode that segment explicitly.

Exit criteria:

- Common `SET` fixtures no longer print `set ...`.
- Verifier rejects executable `TacOp::Set`.

### 4. Make Places Address-Oriented

Problem:

- `Deref` and `Index` contain legacy `TacOperand`.
- `Field` contains a field name, not an offset.
- Index syntax text is carried only for printing.

Required features:

- Pointer dereference.
- Array/pointer indexing.
- Record field access.

Plan:

- Replace `Deref { pointer: TacOperand }` with `Deref { addr: TacValue, ty }`.
- Replace `Field { base, field: String }` with
  `Field { base, offset: u16, ty }`.
- Replace `Index { base, index, syntax }` with
  `Index { base_addr: TacValue, index: TacValue, elem_ty, elem_size }`.
- Add explicit address formation ops where needed:
  - `AddrOfPlace`
  - `AddOffset`
  - `ScaleIndex`
  - or equivalent lowered address ops that keep scheduling visible.
- Preserve source syntax only as debug/source metadata.

Exit criteria:

- Executable places contain no `TacOperand`.
- Record fixture output can still be readable, but the structured TAC carries
  offsets and sizes.
- Verifier rejects `UnresolvedName`, string field names, and index syntax text
  in executable places.

### 5. Normalize Compound Assignment And `FOR`

Problem:

- `TacOp::CompoundAssign` remains for unstable places and artificial `ForStep`.
- This hides load/compute/store order and complicates MIR lowering.

Required features:

- Compound assignment for all assignable places.
- `FOR` loop step update and limit condition.

Plan:

- For every compound assignment, lower address/place evaluation once, then emit
  explicit `Load -> Binary -> Store`.
- For unstable places, materialize the address into a temp before load/store.
- Replace `ForStep` with explicit arithmetic on the loop variable.
- Preserve Action loop semantics with explicit compare direction and step
  handling. If only positive/default steps are currently supported, encode that
  limitation as `UnsupportedForStep` rather than a string op.

Exit criteria:

- `TacOp::CompoundAssign` is removed or verifier-rejected.
- `FOR` fixture output contains only normal loads, compares, binary ops, stores,
  and branches.

### 6. Normalize Conditions

Problem:

- Compare conditions are materialized, but logical conditions and nonzero-value
  conditions still rely on expression lowering shape.
- Short-circuit behavior and boolean value materialization need a precise
  contract.

Required features:

- `AND`, `OR`, `XOR`, and negation in conditions.
- Nonzero tests for scalar/pointer values.
- Branching on constant true/false.

Plan:

- Add condition lowering that explicitly chooses one of:
  - value-producing bool temps; or
  - CFG short-circuit blocks with compare/nonzero terminators.
- Add a `TestNonZero`/`CompareNeZero` op if that is clearer than encoding all
  tests as generic compare.
- Make `Branch` require `TacTypeKind::Bool` or a verifier-approved condition
  value.
- Add fixtures for nested logical conditions, negation, pointer nonzero tests,
  and constant branches.

Exit criteria:

- `Branch` conditions are always typed bool/condition values.
- Verifier rejects non-bool branch values unless explicitly allowed by a
  documented nonzero condition op.

### 7. Normalize Calls And ABI Facts

Problem:

- `TacCallee::Indirect` still stores a target expression summary string.
- Call effects carry counts for reads/writes, not regions.
- Call arity, ABI argument placement, and return placement are not verified.

Required features:

- User/builtin/runtime calls.
- Indirect calls through function/procedure pointers.
- Value-returning calls and no-result calls.
- Conservative effect barriers.

Plan:

- Replace indirect callee target string with `TacValue`.
- Add callable signatures to TAC facts: param count, param types, return type,
  routine kind, ABI class.
- Replace effect counts with structured memory effects:
  - reads unknown/all/regions;
  - writes unknown/all/regions;
  - register clobbers/preserves;
  - OS call and opaque flags.
- Verify call arity and argument width/type compatibility.
- Verify call result type matches callee return type.

Exit criteria:

- MIR can lower calls without looking back into SemIR for callee expression
  shape.
- Verifier rejects arity/type mismatches and untyped indirect callees.

### 8. Make Static Data Byte-Exact

Problem:

- `TacStaticData.value` is a decoded `String`.
- String literal byte encoding, terminator/length policy, and storage section
  are not part of TAC.

Required features:

- String literals as static data.
- Address-of static data as a value.

Plan:

- Replace string value with byte payload plus encoding metadata:
  - `bytes: Vec<u8>`
  - source/display text
  - alignment, mutability, and section
- Decide and document Action string representation at this boundary.
- Keep `StaticAddr` as the address-valued operand, but verify the referenced
  static exists and has pointer-compatible type.

Exit criteria:

- MIR can allocate static data from TAC alone.
- Verifier catches dangling `StaticAddr` IDs.

### 9. Structure Machine Blocks

Problem:

- `TacOp::MachineBlock` carries `items: usize` and formatted effect text, but
  not the machine items or structured effects.

Required features:

- Inline machine-code blocks.
- Machine-block barriers for scheduling and optimization.

Plan:

- Add `TacMachineBlock` with ordered items:
  - literal bytes;
  - labels/symbol refs if supported by SemIR;
  - source span/debug text;
  - structured effects.
- Lower SemIR machine items into this representation.
- Treat machine blocks as opaque by default, but allow precise effects when
  SemIR has them.
- Verify byte values, references, and effect structure.

Exit criteria:

- `MachineBlock` no longer depends on formatted `SemEffects`.
- MIR can preserve or emit inline machine blocks from TAC alone.

### 10. Add Temp Tables And CFG-Aware Verification

Problem:

- Temp validation is block-local.
- Result types live redundantly on instructions and temp values.

Required feature:

- Backend-safe temp definitions and uses across blocks.

Plan:

- Add a routine-local temp table: temp ID, type, defining block/op, optional
  source span.
- Move temp type authority to the table.
- Build CFG predecessor/successor sets from terminators.
- Add dominance or conservative dataflow validation for temp uses across
  blocks.
- Verify no temp is used before definition on any path.

Exit criteria:

- `TacValue::Temp` does not need to carry a full `TacType`, or its copy is
  verified against the temp table.
- Verifier catches cross-block use-before-definition.

### 11. Replace String Labels With Block IDs

Problem:

- `Goto` and `Branch` targets are string labels.

Required feature:

- Stable CFG identity for MIR lowering and optimization.

Plan:

- Add `BlockId` to `TacBlock`.
- Store labels/display names as metadata.
- Change terminators to target `BlockId`.
- Keep printer output readable by mapping IDs back to labels.

Exit criteria:

- Verifier validates block IDs rather than strings.
- MIR CFG construction does not require string maps.

### 12. Remove `TacOperand`

Problem:

- `TacOperand` is the main legacy carrier for expression summaries, unresolved
  names, raw source text, and place-shaped values.

Plan:

- Track all remaining users after the earlier steps.
- Replace each use with one of:
  - `TacValue`
  - `TacPlace`
  - structured metadata
  - explicit unsupported diagnostic before TAC
- Delete `TacOperand` and its verifier/printer paths.

Exit criteria:

- No executable TAC API exposes `TacOperand`.
- No fixture output contains expression-summary values except in source/debug
  metadata.

## Missing Required Feature Checklist

These must be explicit before the 6502 MIR is expected to consume broad TAC:

- absolute memory writes for `SET`;
- byte-exact static data;
- symbol/storage fact tables;
- structured params and locals;
- address-oriented dereference/index/field places;
- complete compound assignment lowering;
- explicit `FOR` step semantics;
- logical condition and nonzero-test lowering;
- indirect calls with value callee targets;
- structured call signatures and effects;
- inline machine block payloads and effects;
- block IDs and CFG fact tables;
- temp table and cross-block dominance/dataflow verification;
- typed verifier rules for casts, binary ops, compares, branches, calls, and
  loads/stores.

## Suggested Vertical Milestones

1. MIR-safe scalar core:
   structured symbols, absolute `SET`, `Load`, `Store`, scalar `Binary`,
   `Compare`, `Branch`, `Return`, no `TacOperand` in scalar fixtures.

2. Storage core:
   address-oriented pointer deref, array index, record field offsets, static
   data bytes.

3. Control core:
   explicit `FOR`, logical conditions, block IDs, CFG-aware temp verification.

4. Call and barrier core:
   structured call signatures/effects, indirect calls, machine block payloads.

5. Final legacy removal:
   delete `TacOperand`, reject all old metadata ops in blocks, remove string
   labels/operation names from executable TAC, and update the 6502 MIR boundary
   doc to describe the final clean contract.
