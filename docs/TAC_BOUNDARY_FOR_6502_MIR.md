# TAC Boundary For 6502 MIR

Snapshot date: 2026-05-31.

This document is the consumer contract for a 6502 MIR that lowers from
`actionc` TAC. It describes the TAC shape that exists today, which parts are
authoritative, and which transitional forms a MIR must reject or preserve as
barriers.

TAC is produced from SemIR:

```text
Action source -> AST -> semantic model -> SemIR -> TAC -> 6502 MIR
```

The public Rust boundary is `actionc::tac`:

- `tac::lower_program(&SemProgram) -> TacProgram`
- `tac::verify_program(&TacProgram) -> Result<(), Vec<TacDiagnostic>>`
- `tac::format_program(&TacProgram) -> String`

`actionc-emit --emit-nir <file.act>` lowers SemIR, verifies the transitional
TAC-backed NIR surface, and prints the formatted program. A MIR consumer should
use the structured program after `verify_program` succeeds; printed NIR is an
observation surface and fixture format, not the semantic source of truth. The
old `--emit-tac` CLI alias has been removed.

## Required MIR Precondition

A 6502 MIR lowerer must run only on verifier-clean TAC:

```rust
let semir = actionc::semantic::ir::lower_program(&loaded.program, &model);
let tac = actionc::tac::lower_program(&semir);
actionc::tac::verify_program(&tac)?;
```

The current verifier checks structural validity, duplicate names/labels,
terminators, branch targets, type-shape consistency, block-local temp
definition order, duplicate temp definitions, and literal-width mismatches for
store-like operations.

Important limitation: temp validation is currently block-local. A MIR lowerer
must not assume cross-block dominance has been proven yet.

## Program Shape

`TacProgram` contains:

- `globals: Vec<TacGlobal>`
- `statics: Vec<TacStaticData>`
- `routines: Vec<TacRoutine>`

`TacGlobal` is declarative metadata:

- `name: String`
- `kind: String`

Global `kind` is still textual. The MIR may use globals for symbol discovery,
but should not parse `kind` as a stable ABI. Storage/layout facts should come
from the surrounding semantic/layout layer until TAC globals are made
structured.

`TacStaticData` is currently used for interned string literals:

- `id: SymbolId`
- `name: String`
- `ty: TacType`
- `bytes: Vec<u8>`
- `display: String`

The current lowerer emits string literals as `TacValue::StaticAddr` pointing at
these entries. A 6502 MIR should allocate the static bytes once and treat the
value as a 16-bit address. `display` is printable text only; `bytes` is the
payload for MIR allocation. Current string bytes are derived from the decoded
string content.

Top-level statements and `SET` directives are represented as a synthetic
routine named `<program>` when needed.

## Routine And CFG Shape

`TacRoutine` contains:

- `name: String`
- `params: Vec<String>`
- `locals: Vec<TacLocal>`
- `notes: Vec<TacRoutineNote>`
- `blocks: Vec<TacBlock>`

`params` are currently names, not stable IDs. `locals` carry a display name and
kind string as routine metadata. `notes` carry routine-level metadata such as a
system address annotation.

`TacBlock` contains:

- `id: BlockId`
- `label: String`
- `ops: Vec<TacOp>`
- `terminator: TacTerminator`

Block IDs are routine-local stable identities and are unique within one routine
after verification. Block labels are printable strings such as `bb0`, `bb1`,
etc. They remain unique debug/display names during the transition. Branch and
goto targets still use these label strings, but verifier target validation
resolves them through a routine-local label-to-`BlockId` table until terminator
targets migrate to `BlockId`.

The MIR should preserve TAC operation order inside a block unless it has a
separate effect-aware scheduler. Calls and machine blocks carry effect/barrier
information, but optimization legality is not fully encoded yet.

## Types

`TacType` carries both the structured type and legacy display facts:

- `kind: TacTypeKind`
- `summary: String`
- `width: Option<u16>`
- `pointer: bool`

After verification, `kind.width()` matches `width`, and `kind.is_pointer()`
matches `pointer`.

Structured kinds:

- `Void`: width `0`
- `Bool`: width `1`
- `U8`: width `1`, used for Action `BYTE` and `CHAR`
- `I8`: width `1`, present but not currently emitted for normal Action scalar
  types
- `U16`: width `2`, used for Action `CARD`
- `I16`: width `2`, used for Action `INT`
- `Ptr16 { pointee }`: width `2`
- `Record { name, size }`: width is `size`; current record sizes are often
  still `None`
- `Callable { kind }`: width `2`
- `Error`: no reliable width

MIR lowering should use `TacTypeKind` and `width`, not `summary`, for machine
decisions. `summary` is fixture/readability text.

Signedness boundary:

- `U8`, `U16`, and `Ptr16` compare/arithmetic as unsigned unless the operation
  semantics say otherwise.
- `I16` is Action `INT` and requires signed compare behavior for relational
  operators.
- `I8` has no established emission contract yet.

## Values

Typed executable TAC uses `TacValue` in value positions:

- `ConstU8(u8)`
- `ConstU16(u16)`
- `StaticAddr { id, name, ty }`
- `Temp { id, ty }`
- `Param(ParamId)`
- `GlobalAddr(SymbolId)`

Current production lowering primarily emits constants, static addresses, and
typed temps. `Param` and `GlobalAddr` exist in the data model but verifier
treats them as untyped today; a MIR should reject executable TAC containing
them until their type tables exist.

`StaticAddr` IDs are checked by the verifier against `TacProgram.statics`.

Constants are already width-shaped:

- `ConstU8` is one byte.
- `ConstU16` is two bytes.

Literal text from the source is not preserved in `TacValue`; the numeric value
is authoritative.

`TempId` numbering is routine-local. IDs are allocated monotonically in the
builder, but consumers should require single definition rather than depend on
dense numbering.

## Places

`TacPlace` describes storage:

- `kind: TacPlaceKind`
- `ty: Option<TacType>`

After verification, executable places have a type.

Current place kinds:

- `Symbol(String)`: named scalar, array, parameter, local, routine variable, or
  global symbol, depending on semantic context outside TAC.
- `Absolute(u16)`: fixed memory address. Current numeric `SET` directives lower
  to stores through this place.
- `UnresolvedName(String)`: unresolved source name. MIR must reject.
- `Deref { pointer: Box<TacOperand> }`: storage through a pointer-shaped legacy
  operand.
- `Index { base, index, syntax }`: indexed storage. `syntax` is `"call"` for
  Action array-call syntax like `a(0)` or `"index"` for bracket syntax.
- `Field { base, field }`: record field by name.

These places are still transitional. A MIR lowerer can consume `Symbol` places
once it can map names to storage/layout facts. It may consume `Field` and
`Index` only if it has enough external layout/type facts to compute addresses.
It should reject `UnresolvedName` and should reject legacy operand forms inside
`Deref`/`Index` unless they are simple materialized values it explicitly
supports.

Field places currently carry field names, not byte offsets. Record field
offsets are present in SemIR record facts, but not embedded in `TacPlace` yet.

## Executable Operations

The following operations are the typed core a 6502 MIR should target first.

### `Load`

```rust
Load { dest: TempId, ty: TacType, place: TacPlace }
```

Reads `place` into `dest`. The result has `ty`. For 6502 lowering, emit a
one-byte or two-byte load according to `ty.width`.

### `AddrOf`

```rust
AddrOf { dest: TempId, ty: TacType, place: TacPlace }
```

Computes the address of `place` into `dest`. Current address values are
16-bit. `ty` is normally a `Ptr16`.

### `Store`

```rust
Store { place: TacPlace, src: TacValue, ty: TacType }
```

Writes `src` to `place` using `ty.width`. The verifier catches oversized
literal stores, but does not yet prove all non-literal source/target type
compatibility.

### `Unary`

```rust
Unary { dest, ty, op, src }
```

Supported operators:

- `Plus`: identity
- `Neg`: two's-complement negation at `ty.width`

### `Cast`

```rust
Cast { dest, src, from, to }
```

Converts `src` from `from` to `to`. Width/sign behavior must follow Action
semantics. The current verifier checks type shape and temp availability, not
cast legality.

### `Binary`

```rust
Binary { dest, ty, op, left, right }
```

Supported operators:

- `Add`
- `Sub`
- `Mul`
- `Div`
- `Mod`
- `Lsh`
- `Rsh`
- `And`
- `Or`
- `Xor`

`ty` is the result type. Operands are `TacValue`s. The MIR should select byte
or word code by `ty.width`. Signed division/modulo semantics are not fully
specified at this TAC boundary yet; do not silently lower unsupported signed
cases.

### `Compare`

```rust
Compare { dest, ty, op, left, right }
```

Supported operators:

- `Eq`
- `Ne`
- `Lt`
- `Le`
- `Gt`
- `Ge`

The result type is currently `Bool`/`condition`, width `1`. A following
`Branch` may consume the temp. Signedness comes from operand/result type facts;
Action `INT` comparisons require signed 16-bit lowering.

### `Call`

```rust
Call {
    callee: TacCallee,
    args: Vec<TacValue>,
    result: Option<TacCallResult>,
    effects: TacCallEffects,
}
```

Callees:

- `User(String)`
- `Builtin(String)`
- `Indirect { target: String }`
- `Runtime { name: String, address: Option<u16> }`

Arguments are value-only. A result, when present, defines `result.dest` with
`result.ty`.

`TacCallEffects` carries:

- `reads: usize`
- `writes: usize`
- `may_call_os: bool`
- `opaque: bool`

The current `reads` and `writes` are counts, not structured memory regions.
For MIR scheduling, treat calls conservatively. If `opaque` or `may_call_os` is
true, the call is a full ordering barrier for memory and machine state unless a
later effect model proves otherwise. TAC/NIR does not describe physical
registers or flags; MIR6502 derives their volatility from the Action calling
convention and the selected call target.

## Terminators

`TacTerminator` ends every block.

### `Goto(String)`

Unconditional control transfer to a block label in the same routine.

### `Branch`

```rust
Branch {
    condition: TacValue,
    then_label: String,
    else_label: String,
}
```

Branches on nonzero/true `condition`. Current lowering normally feeds it with
a `Compare` result temp or `ConstU8(0/1)`. The verifier rejects non-bool temp
conditions and constants other than `0` or `1`.

### `Return(Option<TacValue>)`

Returns from the current routine. The optional value is already materialized.
The ABI placement of return values belongs to the 6502 MIR/backend ABI layer.

### `Exit`

Represents Action `EXIT` outside a loop. Loop-local `EXIT` is lowered to a
`Goto` to the loop's after-block.

### `Fallthrough`

Open-ended completion of a routine or synthetic top-level block. MIR should
treat this as a routine end with no explicit return value unless its caller
requires a stricter policy.

### `Unknown(String)` And `Open`

`Open` and `Unknown` should never survive verification. `Unknown` is an explicit
unsupported control boundary and must be resolved before verifier-clean TAC/NIR
can reach MIR lowering.

## Transitional And Barrier Operations

These operations are legacy or compatibility surfaces. They are not part of the
typed executable core for MIR lowering.

### `Set`

```rust
Set { address: TacOperand, value: TacOperand }
```

Represents Action `SET`, often used for compiler/runtime setup. Operands are
legacy `TacOperand`s. Numeric absolute `SET` pairs now lower to
`Store { place: Absolute(...), ... }`; verifier-clean TAC should not contain
`TacOp::Set`.

### `Define`, `Declare`, `Note`

Legacy metadata/observation operations. Routine-local declarations and notes
now live in routine metadata; verifier-clean TAC should not contain these ops
inside executable blocks.

### `CompoundAssign`

```rust
CompoundAssign { target, op: String, value }
```

Stable normal compound assignments are already lowered to
`Load -> Binary -> Store`. Remaining `CompoundAssign` is a legacy escape hatch,
for unstable places. `FOR` step updates now lower to normal load/add/store TAC
for stable places. MIR must reject any remaining `CompoundAssign` or lower it
through a dedicated compatibility pass.

### `Assign`

Legacy assignment variant. The verifier rejects it today, so verifier-clean
TAC should not contain it.

### `MachineBlock`

```rust
MachineBlock { items: usize, effects: String }
```

Represents an inline machine-code block. Effects are still formatted text.
MIR must treat this as an opaque barrier. It can preserve or delegate the raw
machine block only if it has access to the original machine items outside this
TAC node; the TAC node itself currently does not carry bytes.

### `Unsupported`

Explicit unsupported boundary. MIR must reject.

## Legacy Operands

`TacOperand` remains only for transitional operations and places:

- `Missing`
- `Raw(String)`
- `UnresolvedName(String)`
- `CurrentLocation`
- `Literal { text, value }`
- `Temp(TempId)`
- `Symbol(String)`
- `Place(Box<TacPlace>)`
- `AddressOf(Box<TacPlace>)`
- `AddressOfSymbol(String)`
- `Expr(String)`
- `Call(String)`

MIR code should avoid lowering from `TacOperand` except in a tightly scoped
compatibility path for `SET`, `Deref`, or `Index`. Executable arithmetic,
stores, calls, branches, and returns should use `TacValue`.

## Minimum MIR Acceptance Profile

For an initial 6502 MIR backend, accept only TAC with:

- `Load`, `AddrOf`, `Store`, `Unary`, `Cast`, `Binary`, `Compare`, and `Call`
  as executable ops;
- `Goto`, `Branch`, `Return`, `Exit`, and `Fallthrough` terminators;
- `TacValue::ConstU8`, `ConstU16`, `StaticAddr`, and typed `Temp`;
- `TacPlaceKind::Symbol`, plus `Field`/`Index` only when external layout facts
  can make their addresses exact;
- no `Unsupported`, `Unknown`, `CompoundAssign`, `MachineBlock`, or unresolved
  places in codegen input.

Treat `Set`, `Define`, `Declare`, and `Note` as metadata or pre-codegen
compatibility work, not normal scheduled MIR instructions.

## Validation Fixtures

TAC snapshot fixtures live under `actionc/fixtures/tac`. The test harness in
`actionc/tests/tac_fixtures.rs` loads each `.act`, lowers to SemIR, lowers to
TAC, verifies TAC, formats it, and compares against the `.tac` file.

Useful commands:

```text
cargo test tac_fixtures_match_snapshots
cargo run --bin actionc-emit -- --emit-nir fixtures/tac/scalar_assignments.act
cargo run --bin actionc-tac-sweep -- fixtures/tac
```

The fixture set is the best concrete source for current printed shapes, but
the MIR should bind to the Rust data model described above.
