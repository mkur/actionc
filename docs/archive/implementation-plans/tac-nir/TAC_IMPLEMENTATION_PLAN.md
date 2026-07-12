# TAC Implementation Plan

Snapshot date: 2026-05-30.

TAC is the long-term optimization and backend runway for `actionc`. It should
grow from SemIR, preserve semantic facts, and avoid repeating the AST codegen
mess. SemIR-native remains the correctness runway while TAC becomes observable,
verified, and eventually code-generating.

For the current 6502 MIR consumer contract, see
`docs/TAC_BOUNDARY_FOR_6502_MIR.md`. That document describes the exact
implemented TAC boundary; this plan mixes current status, target shape, and
backlog.

For the legacy-removal sequence and required explicit feature work, see
`docs/TAC_CLEANUP_PLAN.md`.

## Role

TAC owns future expression scheduling, temporary placement, optimization,
allocation, and backend-oriented lowering. It should not become an AST-byte
mimicry layer.

SemIR-native owns near-term correctness, compile coverage, and comparison data.
SemIR-native size deltas are `tac-deferred` unless they expose a semantic bug
or a reusable layer-boundary cleanup.

## Guardrails

- TAC input is SemIR, not AST.
- TAC retains widths, signedness, pointer targets, record facts, call effects,
  machine-block barriers, and source spans where available.
- TAC instructions describe semantic operations before concrete 6502 opcodes.
- Every TAC stage must be inspectable through `actionc-emit --emit-tac`.
- Every TAC program must pass a verifier before it is trusted by tests or later
  codegen.
- Optimizations start only after fixture lowering and verifier coverage are
  stable.

## Layer Contract

TAC is now split into explicit layers so it does not grow into another large
codegen file:

- `TacFacts` owns type and value facts derived from SemIR: TAC type kind,
  width, pointer interpretation, condition type, and literal width facts.
- `TacClassifier` owns read-only shape decisions: whether a call is a real
  value-producing call, whether call syntax is acting like indexed access,
  whether an operator lowers as a compare, and whether a place is stable enough
  for compound lowering.
- `TacLowerer` owns SemIR-to-TAC lowering. It may ask `TacFacts` and
  `TacClassifier` questions, but it should not print TAC or perform verifier
  policy checks.
- `TacBuilder` owns routine-local construction: blocks, labels, temps, static
  data, and ordered instruction insertion. It should not classify SemIR shapes
  or make semantic type decisions.
- `TacPrinter` owns formatting only. Printed output is an observation surface,
  not the source of truth for TAC semantics.
- `tac::ir` owns the transitional TAC data model and should evolve in place
  toward the target typed shape.
- `tac::verifier` owns structural and typed validity checks. As transitional
  forms disappear, the verifier should become the guard that prevents them from
  returning.

New TAC work should name the layer it changes. Vertical backlog work is still
allowed, but each slice should keep classification, lowering, construction,
printing, and verification responsibilities separate.

## Current State

TAC is past the structural skeleton stage and into typed-core construction. It
is an observable, verifier-backed IR runway, but it is not yet a code generation
backend.

Implemented lowering currently produces:

- `TacProgram`, `TacRoutine`, `TacBlock`, and `TacTerminator` structure;
- typed `TacType`, `TacOperand`, `TacPlace`, and `TacCondition` values;
- operations for declarations, assignments, compound assignments, `SET`, calls,
  binary expressions, compare conditions, machine/effect barriers, notes, and
  unsupported boundaries;
- `%tN` temporaries for lowered binary values and compare branch conditions;
- branch, goto, return, exit, fallthrough, open, and unknown terminators.

The verifier currently checks:

- duplicate globals, duplicate routines, duplicate block labels, and empty
  structural names;
- missing/open terminators and missing branch targets;
- operand/place type presence for typed operations;
- oversized literal width mismatches for assignments and compound assignments;
- block-local temp use-before-definition and duplicate temp definitions;
- basic typed result sanity for binary, compare, and call operations.

Observability and validation currently include:

- `actionc-emit --emit-tac <file.act>`;
- golden fixtures under `fixtures/tac`;
- `tests/tac_fixtures.rs` snapshot validation;
- `actionc-tac-sweep` for load/analyze/SemIR/TAC/verify validation;
- the current SemIR fixture sweep result: 32 OK, 0 load/semantic/TAC failures.

Known gaps before TAC-native codegen:

- complete replacement of expression summaries in lvalue-shaped lowering;
- broader width/type checks across every typed operation;
- call arity/effect verification;
- cross-block temp/dataflow validation;
- logical condition decomposition for `AND`, `OR`, `XOR`, and negated
  conditions;
- optimization passes and a `tac-native` codegen entry point.

## Target Typed Shape

The intended typed TAC shape is close to the sketch below. This is a target
model, not the current implementation. The important boundary is that value
positions contain only already-materialized values; storage and addressing live
in `TacPlace`; complex work is expressed as explicit instructions.

```rust
enum TacType {
    Void,
    Bool,
    U8,
    I8,
    U16,
    I16,
    Ptr16 {
        pointee: Option<Box<TacType>>,
    },
    Record {
        name: SymbolId,
        size: u16,
    },
}

enum TacValue {
    ConstU8(u8),
    ConstU16(u16),
    StringLiteral {
        value: String,
        ty: TacType,
    },
    Temp(TempId),
    Param(ParamId),
    GlobalAddr(SymbolId),
}

enum TacPlace {
    Local(LocalId),
    Global(SymbolId),
    Absolute(u16),
    Deref {
        addr: TacValue,
        ty: TacType,
    },
    Field {
        base: Box<TacPlace>,
        offset: u16,
        ty: TacType,
    },
    Index {
        base_addr: TacValue,
        index: TacValue,
        elem_ty: TacType,
        elem_size: u8,
    },
}

enum TacInst {
    Load {
        dst: TempId,
        place: TacPlace,
        ty: TacType,
    },
    Store {
        place: TacPlace,
        src: TacValue,
        ty: TacType,
    },
    Copy {
        dst: TempId,
        src: TacValue,
        ty: TacType,
    },
    Unary {
        dst: TempId,
        op: TacUnaryOp,
        src: TacValue,
        ty: TacType,
    },
    Binary {
        dst: TempId,
        op: TacBinaryOp,
        lhs: TacValue,
        rhs: TacValue,
        ty: TacType,
    },
    Compare {
        dst: TempId,
        op: TacCompareOp,
        lhs: TacValue,
        rhs: TacValue,
        ty: TacType,
    },
    Cast {
        dst: TempId,
        src: TacValue,
        from: TacType,
        to: TacType,
    },
    AddrOf {
        dst: TempId,
        place: TacPlace,
    },
    Call {
        dst: Option<TempId>,
        callee: TacCallee,
        args: Vec<TacValue>,
        effects: CallEffects,
    },
    MachineBlock {
        effects: MachineEffects,
    },
}

enum TacTerminator {
    Goto(BlockId),
    Branch {
        cond: TacValue,
        then_block: BlockId,
        else_block: BlockId,
    },
    Return(Option<TacValue>),
    Exit,
    Unreachable,
}
```

Notes:

- `I16` matters early because Action `INT` is signed word-sized. `I8` can stay
  present in the model but does not need to drive early implementation unless
  `CHAR`/byte signedness semantics require it.
- `Ptr16` should keep pointee facts when SemIR has them. Unknown or erased
  pointer targets can use `None`.
- `Record` is kept as a typed fact with a known size. Field access should lower
  to offsets in TAC places rather than carrying record syntax into codegen.
- `Index` should use an already-materialized `base_addr` so address formation
  is explicit and schedulable.
- String literals are currently represented as typed TAC values so call
  arguments can stay value-only. They should later be interned into static data
  symbols and become `GlobalAddr`/`ConstU16`-like addresses.
- `Branch` consumes a `TacValue`, normally a `Bool`/condition temp produced by
  `Compare` or logical lowering.
- Calls must carry effect information so optimization and scheduling do not
  move loads/stores across unsafe boundaries.

## Migration Plan To Target Shape

Move from the current transitional TAC to the target shape in compatibility
preserving slices. Each slice should keep `--emit-tac`, TAC fixtures, and
`actionc-tac-sweep` green before moving on.

1. Add structured IDs and type facts.
   Introduce durable IDs (`TempId`, `ParamId`, `LocalId`, `BlockId`,
   `SymbolId`) and a structured TAC type enum alongside the current printable
   type fields. First verifier checks should prove every lowered type has a
   structured kind, width, and pointer interpretation.

2. Split simple values out of legacy operands.
   Add `TacValue` for constants, temps, params, and materialized addresses.
   Keep legacy `TacOperand` temporarily, but migrate `Binary` and `Compare`
   first because they are already closest to value-only operands.

3. Make temp typing authoritative.
   Add a routine temp table and verify that every temp use has one definition
   and one type. During transition, instruction result types may remain for
   formatting, but verifier ownership should move to the temp table.

4. Introduce target instructions without a second IR universe.
   Evolve `TacOp` toward `TacInst` in place rather than maintaining parallel
   instruction trees. Add target variants only when lowering and verification
   can use them immediately.

5. Convert assignment shapes into explicit stores.
   Lower plain assignment to `Store`. Lower compound assignment to
   `Load -> Binary -> Store`. This removes special assignment semantics from
   later TAC-native codegen.

6. Make places storage-oriented.
   Replace stringly symbol, field, index, and dereference places with local,
   global, absolute, field-offset, indexed-address, and dereference-address
   forms. Field lowering should use offsets and typed facts from SemIR.

7. Add explicit address formation.
   Add `AddrOf` and remove address/place-shaped values from value positions.
   After this, executable TAC should not need operand variants like `Place`,
   `AddressOf`, `Expr`, or `Call`.

8. Normalize calls.
   Calls should accept only `TacValue` arguments, produce an optional typed
   temp, and carry conservative effect metadata before any optimization tries
   to move code around them.

9. Normalize conditions and terminators.
   Branches should consume `Bool`/condition values. Compare and logical
   lowering should produce those values or CFG structure explicitly, including
   `AND`, `OR`, `XOR`, and negated conditions.

10. Tighten the verifier as escape hatches disappear.
    Add hard checks for value-only operands, load/store type compatibility,
    binary/compare type support, bool branch conditions, call effects,
    machine-block barriers, and eventually CFG-aware temp dominance.

11. Remove transitional forms.
    Delete expression summaries, call-shaped operands, place-shaped operands,
    stringly operation names, and the old summary/width/pointer-only type
    representation once fixtures no longer rely on them.

TAC-native codegen should wait until value positions are simple, load/store and
address formation are explicit, branches consume condition values, calls have
effects, and the verifier rejects expression-shaped executable TAC.

## Legacy Shape Audit

Audit date: 2026-05-31.

The post-split TAC code still contains these transitional shapes. They are
allowed only while they are on this list, and each removal should add verifier
coverage so the shape cannot silently return.

Instruction and operand escape hatches:

- `TacOperandKind::Raw`, `UnresolvedName`, `CurrentLocation`, `Symbol`,
  `Place`, `AddressOf`, `AddressOfSymbol`, `Expr`, and `Call` remain as
  compatibility operands for `SET`, legacy assignment fallback, legacy
  compound assignment fallback, and printable notes.
- `TacOp::Assign` remains as a verifier-rejected legacy variant for now.
  Production lowering should emit `Store` or an explicit unsupported boundary
  instead.
- `TacOp::CompoundAssign` remains for unstable places and artificial forms such
  as `ForStep`. Stable compound assignments should already lower through
  `Load -> Binary -> Store`.
- `TacOp::Set`, `Define`, `Declare`, `Note`, and `Unsupported` are still
  string/summary-shaped observation or compatibility operations.

Stringly structured fields:

- `TacOp::MachineBlock` still carries effects as a formatted string. It should
  carry structured machine effects.
- `TacGlobal.kind`, `TacRoutine.params`, `TacRoutine.locals`, block labels, and
  branch targets are still printable names instead of durable IDs/fact tables.

Place escape hatches:

- `TacPlaceKind::Symbol`, `UnresolvedName`, `Deref` with a legacy operand,
  `Index` with legacy operands and syntax text, and `Field` with a field name
  remain. These should move to local/global/absolute places, address-valued
  dereferences, indexed addresses, and field offsets.
- Record-field fixture output still prints field names such as `rp.tag`; this
  is readable but not the target storage form.

Fixture-visible remaining transitional output:

- Top-level runtime setup still prints `set ...`.
- `FOR` step updates can still print legacy compound syntax such as
  `i ForStep= 1`.
- Machine blocks still print debug-formatted effect summaries.
- Declarations and defines still print string summaries.

Preferred removal order:

1. Split legacy `SET` operands into typed absolute stores or explicit
   compatibility barriers.
2. Replace string field/index/deref places with typed storage-oriented places.
3. Move routine params, locals, labels, and branch targets to durable tables and
   IDs.

## Milestones

### 1. Structural TAC Skeleton

Status: initial slice complete.

- Keep `TacProgram`, `TacRoutine`, `TacBlock`, `TacOp`, and `TacTerminator`
  inspectable.
- Lower SemIR routines and top-level statements into labeled blocks.
- Expose `actionc-emit --emit-tac <file.act>`.
- Add a verifier for labels, terminators, and structural integrity.
- Add golden TAC fixtures.

### 2. Typed TAC Core

Status: started. TAC now has type-bearing operands, places, conditions, and
call results for the current skeleton operations. Formatting is still textual
so early fixtures remain readable while the internals become less stringly.

Replace string-shaped operations with typed TAC primitives:

- `TacValueId` for temporaries;
- `TacType` for scalar, pointer, record, callable, and error facts;
- `TacPlace` for symbols, dereferences, indexed places, fields, and absolute
  storage;
- `TacOperand` for literals, values, places, addresses, and call results;
- `TacInstr` for copy, unary, binary, compare, load, store, address,
  call, machine barrier, and notes;
- `TacTerminator` for goto, conditional branch, return, exit, unreachable, and
  unknown/error boundaries.

The verifier should grow with the typed core: operand type checks, destination
width checks, call arity/effects, and dominance/definition checks once temps
exist.

### 3. Fixture Expansion

Add fixtures in small SemIR-backed slices:

1. scalar assignments;
2. byte/word arithmetic;
3. lvalues: symbol, dereference, index, field;
4. control flow: if, while, do-until, for, exit, return;
5. calls and return values;
6. arrays, strings, and pointer decay;
7. records and computed fields;
8. machine blocks and effect barriers;
9. Toolkit/stress probes that represent backend pressure.

Fixture updates should be intentional and reviewable, just like SemIR fixtures.

### 4. TAC Validation Sweep

Add TAC validation to the sweep workflow. Initial success means:

- source loads;
- semantic analysis succeeds;
- SemIR lowers;
- TAC lowers;
- TAC verifier passes.

Only later should TAC sweep success require native code generation.

### 5. TAC-Native Codegen

Add `--codegen-source tac-native` only after typed TAC and verifier coverage are
usable. TAC-native can reuse proven lower-level pieces from SemIR-native:

- storage/layout facts;
- tracked emitter helpers;
- source maps;
- proof hooks;
- materialization helpers that are still abstraction-correct.

TAC-native should own scheduling and temporary decisions. Reuse should not pull
SemIR-native's conservative code shape into the long-term optimizer.

### 6. Optimization

Start with verifier-backed local optimizations:

- constant folding;
- branch simplification;
- dead temporary removal;
- copy propagation;
- local common subexpression reuse;
- temporary lifetime shrinking.

Later work can introduce SSA, register/zero-page allocation, branch layout,
expression reshaping, and backend-specific lowering.

## Current Near-Term Execution

Completed:

- Documented this plan.
- Added a structural TAC verifier.
- Added initial golden TAC fixtures and a fixture test harness.
- Kept `--emit-tac` passing through the verifier.
- Started typed TAC internals for operands, places, branch conditions, calls,
  assignments, returns, and `SET`.
- Added the first width-compatibility verifier check for plain assignments.
- Added TAC fixture coverage for calls and return values.
- Added TAC fixture coverage for records and field places.
- Added TAC fixture coverage for machine/effect barriers.
- Added `actionc-tac-sweep` for load/analyze/SemIR/TAC/verify validation.
- Verified the current SemIR fixture set with `actionc-tac-sweep`: 32 OK,
  0 load/semantic/TAC failures.
- Started real TAC temporaries by lowering assignment/return binary expression
  values into `Binary` ops with `%tN` operands.
- Added verifier checks for block-local temp definition order and duplicate
  temp definitions.
- Added typed `Compare` ops for branch compare conditions, with branches now
  consuming typed `%tN` condition operands.
- Expanded the first width verifier beyond plain assignments to compound
  assignments.
- Added structured `TacTypeKind` facts alongside legacy TAC type display fields,
  with verifier checks that width and pointer facts remain consistent.
- Renamed TAC temp IDs to `TempId`, added durable ID wrappers, and introduced
  `TacValue` for simple constants, temps, params, and materialized global
  addresses.
- Added explicit `Load` TAC ops for lvalues used as values, so those value
  positions now consume temps instead of embedded places where lowering can
  materialize them.
- Materialized symbol reads through `Load` and tightened the verifier so
  `Binary` and `Compare` operands must be simple `TacValue`-compatible
  constants or temps.
- Materialized real value-producing calls through `TacCallResult` temps, while
  leaving array-call syntax out of TAC `Call` until indexed places are lowered
  explicitly.
- Changed `Binary` and `Compare` operands from legacy `TacOperand` fields to
  `TacValue` fields.
- Added explicit `AddrOf` TAC ops for address-of and implicit-address-of
  expressions, so address-shaped values can be materialized as temps.
- Changed `Call` arguments from legacy `TacOperand` fields to value-only
  `TacValue` fields.
- Materialized array decay, symbol address, and array-call/pointer-call index
  reads needed by value-only call arguments.
- Added a typed string-literal TAC value as a transitional representation until
  TAC has static data symbols.
- Added explicit `Store` TAC ops for assignments whose sources have already
  been materialized to `TacValue`, with legacy `Assign` kept as a fallback for
  remaining expression-shaped sources.
- Added verifier coverage for `Store` temp use and width compatibility.
- Audited SemIR fixture TAC output for remaining legacy executable expression
  summaries. Current fixture output no longer shows ordinary legacy assignment
  expression summaries; the known transitional surfaces are `SET`, `ForLimit`
  condition summaries, return operands, compound assignment, and the `Assign`
  fallback for sources that are not yet value-only.
- Added typed `Unary` and `Cast` TAC ops for value-producing unary plus,
  unary negation, and fundamental typed casts.
- Added TAC fixture coverage for unary and cast materialization.
- Changed `Return` terminators from legacy `TacOperand` payloads to value-only
  `TacValue` payloads.
- Lowered stable compound assignments into explicit `Load -> Binary -> Store`
  sequences. Indexed, dereferenced, unresolved, and artificial `ForStep`
  compound forms remain on the legacy `CompoundAssign` path until place
  evaluation is explicit enough to avoid duplicating side effects.
- Changed branch terminators to consume value-only `TacValue` conditions.
- Lowered `FOR` limit tests into explicit `Load -> Compare -> Branch`
  sequences instead of carrying `ForLimit` expression summaries in branch
  terminators.
- Added TAC fixture coverage for normalized `FOR` branch conditions.
- Added program-level TAC static data entries and interned string literals into
  generated static symbols.
- Replaced inline typed string-literal values with typed static-address values
  in TAC value positions.
- Added TAC fixture coverage for string literal static interning.
- Split TAC into explicit `facts`, `classifier`, `ir`, `lowerer`, `printer`,
  `verifier`, and `tests` modules, with a small facade in `src/tac.rs`.
- Replaced stringly unary, binary, and compare operator fields with typed TAC
  operator enums while preserving existing fixture output.
- Replaced stringly call callees and missing call effects with `TacCallee` and
  `TacCallEffects` while preserving existing fixture output.
- Stopped production lowering from emitting legacy `Assign` fallbacks and made
  the verifier reject hand-built `Assign` ops. Assignment-shaped lowering now
  uses `Store` when it can materialize a value.

Next:

1. Continue splitting remaining expression-shaped operands into real `TacInst`
   temporaries.
2. Continue expanding verifier width checks across typed operations.
3. Split legacy `SET` operands into typed absolute stores or explicit
   compatibility barriers.
4. Replace string field/index/deref places with typed storage-oriented places.
