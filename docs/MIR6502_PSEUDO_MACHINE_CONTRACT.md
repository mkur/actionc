# MIR6502 Pseudo-Machine Contract

Snapshot date: 2026-06-01.

This note defines the intended contract for the first MIR6502 layer after
verifier-clean NIR. It incorporates the review items captured in
`docs/archive/reviews/MIR6502_CONTRACT_REVIEW.md`.

MIR6502 is a target-machine IR, not final emitted 6502 bytes. It should make
6502 lowering decisions explicit enough to verify, print, test, and optimize
locally before the emission layer writes exact opcodes.

The target pipeline is:

```text
Action source -> AST -> semantic model -> SemIR -> NIR -> MIR6502 -> emission
```

## Definitions

**Materialization** means turning abstract NIR/MIR values, storage, conditions,
or calls into a concrete target strategy: byte lanes, temporary homes, ABI homes,
carry/flag dependencies, helper calls, and address forms.

**Home** means a place where a byte or word value is intentionally located for a
MIR phase: a virtual temp, register, spill slot, zero-page slot, ABI slot, or
memory location.

**Address form** means a load/store addressing strategy, such as absolute,
static+offset, zero-page, absolute indexed, or indirect indexed through a
zero-page pointer pair.

**Barrier** means an operation boundary that prevents reordering or deletion
unless effects prove it is safe. Calls, OS/runtime interactions, raw data,
machine blocks, hardware registers, and unknown absolute memory are conservative
barriers by default.

**Pre-materialization MIR** is the first lowering target from NIR. It may contain
virtual temps, abstract storage homes, word pseudo ops, and compare results in
bool temps.

**Post-materialization MIR** is closer to executable 6502 work. It has byte-lane
expansion, explicit carry/borrow behavior, selected ABI homes, selected helper
calls, and concrete-enough address forms.

**Pre-emission MIR** is the final verified subset of post-materialization MIR. It
has no unsupported pseudo ops, unresolved storage, unresolved labels, or
unassigned virtual temps. It is ready to feed tracked emission helpers.

## Purpose

MIR6502 exists to bridge the gap between normalized Action!-aware NIR and final
6502 emission.

NIR owns source-language meaning and normalized computation:

- typed values, places, temps, routines, blocks, and terminators;
- structured storage identity;
- call signatures and conservative effects;
- explicit loads, stores, casts, arithmetic, compares, calls, and branches;
- static data and machine-block references.

MIR6502 owns target strategy:

- byte and word expansion;
- local A/X/Y/flags use;
- ABI argument and result homes;
- zero-page and scratch-slot decisions;
- 6502 addressing-form selection;
- runtime-helper selection;
- compare/test/branch fusion;
- target-specific peepholes and local machine cleanup.

Emission owns concrete bytes and output mechanics:

- exact opcode selection and writing;
- label binding and patching;
- branch-distance repair or diagnostics;
- Atari load-file segment writing;
- source maps, listings, and proof hooks;
- tracked processor-state updates;
- raw data and machine-code barriers.

## Red Lines

MIR6502 must not recover missing facts by inspecting SemIR or parsing printed IR.
Verifier-clean NIR is the only semantic input.

MIR6502 must not contain executable source syntax:

- no expression summary strings;
- no unresolved symbol names as executable identity;
- no record field names instead of byte offsets;
- no array/index source syntax strings;
- no SemIR expression handles as hidden lowering side channels.

MIR6502 may keep display names only as diagnostics, comments, source maps, or
printer metadata. Display names may be non-optional for readability, but they are
never executable identity.

## MIR Phases

The MIR machine is intentionally phased. The first implementation should not
force all NIR operations directly into final 6502 instruction forms.

### Pre-materialization MIR

Pre-materialization MIR is the initial NIR lowering target.

It may contain:

- virtual MIR temps;
- abstract storage homes;
- word-width pseudo operations;
- compare results materialized as bool temps;
- direct references to MIR blocks and storage IDs;
- calls expressed through ABI plans rather than final byte sequences.

Examples:

```text
vt0:u8  = load.u8 local(x)
vt1:u16 = add.u16 local(a), #1
vt2:bool = cmp.lt.i16 vt1, #100
branch bool vt2, bb_then, bb_else
```

A branch should not duplicate compare operands. The compare fact has one owner:
either a `Compare` op result or a lowered flag-producing sequence.

### Post-materialization MIR

Post-materialization MIR is closer to legal 6502 work, but it is still MIR, not
emitted assembly.

It should contain:

- byte-expanded word operations;
- explicit carry/borrow dependencies for byte-lane `Add`/`Sub`;
- selected ABI homes;
- selected runtime helpers for expensive operations;
- selected addressing forms where known;
- flag-producing compare/test sequences where useful;
- explicit barriers around calls, raw data, machine blocks, and unknown effects.

Example word add after byte-lane expansion:

```text
vt0:u8 = load.u8 local(a).lo
vt1:u8 = add.u8 vt0, #1 carry_in=Clear
store.u8 local(a).lo, vt1
vt2:u8 = load.u8 local(a).hi
vt3:u8 = add.u8 vt2, #0 carry_in=FromPrevious
store.u8 local(a).hi, vt3
```

The final emission layer decides which exact opcodes write these operations.
MIR may contain carry-aware pseudo ops, but it should not become a complete
one-variant-per-6502-opcode pseudo ISA in the first implementation.

### Pre-emission MIR

Pre-emission MIR is the final checked subset of post-materialization MIR.

It should contain:

- no unsupported pseudo ops;
- no unresolved storage or label references;
- no unassigned virtual temps;
- no unresolved helper selections;
- no abstract compare conditions;
- no raw data or machine-code boundary without an effect barrier;
- only ordinary instruction work expressible through tracked emission helpers.

## Core Program Shape

Recommended Rust-like shape:

```rust
pub struct MirProgram {
    pub statics: Vec<MirStatic>,
    pub globals: Vec<MirGlobal>,
    pub routines: Vec<MirRoutine>,
    pub runtime_helpers: Vec<MirRuntimeHelperDecl>,
}

pub struct MirRoutine {
    pub id: RoutineId,
    pub name: String,
    pub abi: MirRoutineAbi,
    pub frame: MirFrame,
    pub temps: Vec<MirTemp>,
    pub blocks: Vec<MirBlock>,
    pub effects: MirEffects,
}

pub struct MirBlock {
    pub id: MirBlockId,
    pub label: String,
    pub ops: Vec<MirOp>,
    pub terminator: MirTerminator,
}
```

Routine names and block labels are display metadata. Stable IDs are executable
identity.

Block order is a layout hint, not semantic identity. MIR passes may preserve or
adjust order for readability and branch locality. Emission remains responsible
for final label binding, branch patching, and long-branch repair or diagnostics.

## Frame And Storage Layout

`MirFrame` is the routine-local target layout record.

```rust
pub struct MirFrame {
    pub params: Vec<MirStorageSlot>,
    pub locals: Vec<MirStorageSlot>,
    pub spills: Vec<MirStorageSlot>,
    pub virtual_zero_page: Vec<MirZpSlot>,
    pub fixed_zero_page: Vec<MirFixedZpSlot>,
}

pub struct MirStorageSlot {
    pub id: MirStorageId,
    pub width: MirWidth,
    pub base: MirStorageBase,
    pub offset: u16,
    pub mutable: bool,
}

pub enum MirStorageBase {
    Param(ParamId),
    Local(LocalId),
    Spill(MirSpillId),
    Global(GlobalId),
    Static(StaticId),
    Absolute(u16),
}
```

Rules:

- NIR stable IDs remain the source of identity for params, locals, globals,
  statics, and routines.
- MIR may assign target storage homes, but it must not resolve names through
  SemIR or source strings.
- `MirZpSlot` is abstract/virtual until a zero-page allocation pass maps it to a
  concrete address.
- Fixed ABI zero-page locations must use a separate fixed form so they cannot be
  confused with allocatable zero-page temps.

## Width Model

MIR6502 should make byte and word work explicit.

```rust
pub enum MirWidth {
    Byte,
    Word,
}
```

Pre-materialization MIR may use `MirWidth::Word` on pseudo operations.
Post-materialization MIR should prefer explicit byte lanes for operations that
are ready for emission.

For post-materialization word values, use explicit low/high byte locations:

```rust
pub struct MirWordValue {
    pub lo: MirValue,
    pub hi: MirValue,
}

pub struct MirWordDef {
    pub lo: MirDef,
    pub hi: MirDef,
}
```

This avoids hiding 6502 byte order and carry behavior inside a generic word
location after materialization.

## Register And Flag Model

MIR6502 may mention physical 6502 resources when a target decision has been made.

```rust
pub enum MirReg {
    A,
    X,
    Y,
}

pub enum MirFlag {
    Z,
    N,
    C,
    V,
}
```

Pre-materialization MIR should avoid overcommitting to registers unless an ABI
or addressing mode requires it. Post-materialization MIR may use A/X/Y and flags
as explicit local resources.

There is no general register allocator in the first MIR implementation.
Materialization may assign A/X/Y locally for concrete sequences. A broader
allocator, if added later, should be a separate post-materialization pass.

## Definitions, Values, Memory, And Addresses

MIR distinguishes definition sites, value operands, and memory/addressing sites.
This avoids treating memory as if it could directly receive pure operation
results.

### Definition sites

A definition site can receive the result of a pure MIR operation.

```rust
pub enum MirDef {
    VTemp(MirTempId),
    Reg(MirReg),
}
```

Rules:

- `Load`, `LoadImm`, `Unary`, `Binary`, `Compare` materialization, `Extend`,
  `Truncate`, and `LeaAddr` define `MirDef`s.
- Memory destinations are written through `Store`, not used as operation defs.
- Pre-emission MIR must not contain unassigned virtual temps.

### Value operands

Values are already materialized or materializable machine values.

```rust
pub enum MirValue {
    ConstU8(u8),
    ConstU16(u16),
    Def(MirDef),
    Word { lo: Box<MirValue>, hi: Box<MirValue> },
    StaticAddr(StaticId),
    GlobalAddr(GlobalId),
    RoutineAddr(RoutineId),
}
```

Rules:

- Constants are numeric and width-shaped.
- Address values are 16-bit values and should materialize as low/high bytes.
- Source literal text is never executable MIR semantics.

### Memory and addressing sites

`MirMem` describes memory that can be read or written. It is not a value by
itself.

```rust
pub enum MirMem {
    Absolute(u16),
    Static { id: StaticId, offset: u16 },
    Global { id: GlobalId, offset: u16 },
    Local { id: LocalId, offset: u16 },
    Param { id: ParamId, offset: u16 },
    Spill { id: MirSpillId, offset: u16 },
    ZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
}
```

`MirAddr` describes the address form selected for a load/store operation.

```rust
pub enum MirAddr {
    Direct(MirMem),
    Label(MirLabel),
    ZeroPageIndexedX { base: MirZpSlot },
    AbsoluteIndexedX { base: MirMem },
    AbsoluteIndexedY { base: MirMem },
    IndirectIndexedY { zp: MirZpSlot },
}
```

Rules:

- `MirMem` says what memory is being accessed.
- `MirAddr` says how the access will be addressed.
- The first lowering slice should support only `Direct` forms.
- Indexed and indirect forms should be added when pointer and array lowering need
  them.
- Do not add source-shaped address forms.
- `Field` lowering must already have a byte offset before MIR.
- `Index` lowering should use element size facts from NIR, not source syntax.
- A byte index into directly allocated local, global, static, or absolute
  storage may select `AbsoluteIndexedX` or `AbsoluteIndexedY`. The storage ID
  remains authoritative until emission resolves its address. Pointer- and
  descriptor-backed arrays must retain an indirect address strategy.
- `Deref` lowering should materialize pointer values into an explicit address
  strategy, usually a zero-page pointer pair plus `Y` for indirect-indexed work.

## Carry And Borrow Model

Post-materialization byte-lane arithmetic must make carry behavior explicit.

```rust
pub enum MirCarryIn {
    Clear,
    Set,
    FromPrevious,
}

pub enum MirCarryOut {
    Ignore,
    Produce,
}
```

Rules:

- Pre-materialization word `Add`/`Sub` may omit carry fields.
- Post-materialization byte-width `Add` and `Sub` must carry explicit
  `carry_in` and `carry_out` facts.
- A low-byte add normally uses `carry_in=Clear` and `carry_out=Produce`.
- A high-byte add in the same chain normally uses `carry_in=FromPrevious`.
- A low-byte subtract normally uses `carry_in=Set` and `carry_out=Produce`,
  matching 6502 borrow convention.
- The verifier should reject byte-lane add/sub chains whose carry dependency is
  implicit or impossible to preserve across intervening flag-clobbering ops.

## Operation Families

MIR opcodes should be added by family. The initial enum should stay small and
stable enough for scalar lowering.

```rust
pub enum MirOp {
    LoadImm {
        dst: MirDef,
        value: u16,
        width: MirWidth,
    },
    Load {
        dst: MirDef,
        src: MirAddr,
        width: MirWidth,
    },
    Store {
        dst: MirAddr,
        src: MirValue,
        width: MirWidth,
    },
    Move {
        dst: MirDef,
        src: MirValue,
        width: MirWidth,
    },
    LeaAddr {
        dst: MirDef,
        target: MirMem,
        width: MirWidth,
    },
    Extend {
        dst: MirDef,
        src: MirValue,
        from_width: MirWidth,
        to_width: MirWidth,
        signed: bool,
    },
    Truncate {
        dst: MirDef,
        src: MirValue,
        from_width: MirWidth,
        to_width: MirWidth,
    },
    Unary {
        op: MirUnaryOp,
        dst: MirDef,
        src: MirValue,
        width: MirWidth,
    },
    Binary {
        op: MirBinaryOp,
        dst: MirDef,
        left: MirValue,
        right: MirValue,
        width: MirWidth,
        carry_in: Option<MirCarryIn>,
        carry_out: MirCarryOut,
    },
    Compare {
        dst: MirCondDest,
        op: MirCompareOp,
        left: MirValue,
        right: MirValue,
        width: MirWidth,
        signed: bool,
    },
    Call {
        target: MirCallTarget,
        abi: MirCallAbi,
        args: Vec<MirArgHome>,
        result: Option<MirResultHome>,
        effects: MirEffects,
    },
    RuntimeHelper {
        helper: MirRuntimeHelper,
        args: Vec<MirArgHome>,
        result: Option<MirResultHome>,
        effects: MirEffects,
    },
    Barrier {
        effects: MirEffects,
    },
    MachineBlock {
        id: MachineBlockId,
        effects: MirEffects,
    },
}
```

Compare destinations are explicit:

```rust
pub enum MirCondDest {
    Temp(MirTempId),
    Flags,
}
```

Operator sets should initially mirror only target-meaningful NIR operations:

```rust
pub enum MirUnaryOp {
    Neg,
    BitNot,
}

pub enum MirBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lsh,
    Rsh,
    And,
    Or,
    Xor,
}

pub enum MirCompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
```

Rules:

- `Move` represents identity/copy; there is no separate identity unary op.
- Source-level logical-not should already be a bool compare or branch inversion
  by the time MIR is reached. If NIR still delivers logical-not, NIR-to-MIR
  lowering should expand it into compare/branch logic, not preserve it as a MIR
  unary op.
- NIR `Cast` lowers to `Extend`, `Truncate`, or `Move` depending on width and
  signedness.
- NIR `AddrOf` lowers to `LeaAddr` or direct address materialization.
- `Mul`, `Div`, `Mod`, and wide shifts may remain pseudo ops until helper
  selection.
- Byte and word `Add`, `Sub`, `And`, `Or`, and `Xor` should be the first expanded
  arithmetic families.
- Do not add one pseudo-op per source-language pattern.
- Add a new pseudo-op only when it represents a stable target-level decision that
  NIR should not know about and emission should not rediscover.

## Terminators

MIR terminators are block-level control transfers.

```rust
pub enum MirTerminator {
    Jump(MirBlockId),
    Branch {
        cond: MirCond,
        then_block: MirBlockId,
        else_block: MirBlockId,
    },
    Return,
    Exit,
    Unreachable,
}
```

Rules:

- Terminator targets are block IDs, not strings.
- NIR fallthrough must be normalized to `Return(None)` or a documented NIR
  terminator before MIR lowering; MIR does not carry a separate `Fallthrough`.
- Pre-materialization branches should consume bool values produced by `Compare`
  or equivalent bool materialization.
- Post-materialization branches should prefer flag tests when a compare/test can
  feed the branch directly.
- Compare results that are used as ordinary values must still be materializable as
  `0` or `1` bytes.

## Condition Model

Conditions support both bool-value branches and flag-aware lowering without
duplicating compare operands.

```rust
pub enum MirCond {
    BoolValue(MirValue),
    FlagTest(MirFlagTest),
    FusedCompare {
        producer: MirOpRef,
        flag_test: MirFlagTest,
    },
}

pub enum MirFlagTest {
    ZSet,
    ZClear,
    CSet,
    CClear,
    NSet,
    NClear,
    VSet,
    VClear,
}
```

Rules:

- `MirCond` must not duplicate a full compare operation. The compare has one
  owner: a `MirOp::Compare` or an already-lowered flag-producing sequence.
- `BoolValue` is the ordinary pre-materialization branch form.
- `FlagTest` is the ordinary post-materialization branch form.
- `FusedCompare` may be used when a branch is fused with a specific compare op;
  it references the producer rather than copying operands.
- Multi-step word and signed comparisons may lower to a small MIR control-flow
  sequence rather than a single `FlagTest`.

## ABI Model

MIR must make call homes explicit before emission.

```rust
pub struct MirCallAbi {
    pub params: Vec<MirArgHome>,
    pub result: Option<MirResultHome>,
    pub clobbers: MirRegisterSet,
    pub preserves: MirRegisterSet,
}

pub enum MirArgHome {
    Reg(MirReg),
    RegisterPair { lo: MirReg, hi: MirReg },
    ZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
    Absolute(u16),
    StackFrame { base: u16, offset: u16 },
}

pub enum MirResultHome {
    Reg(MirReg),
    RegisterPair { lo: MirReg, hi: MirReg },
    ZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
    Absolute(u16),
    ReturnSlot { offset: u16 },
}
```

Rules:

- Call lowering should be signature-driven.
- Argument packing should be planned in one place, not spread across source
  shapes.
- Opaque, OS, runtime, and unknown-effect calls are full barriers unless precise
  effects prove otherwise.
- Indirect calls must have typed 16-bit callable targets before MIR lowering.
- MIR carries both `clobbers` and `preserves` so NIR effect facts are not silently
  lost before ABI lowering and call scheduling decisions consume them.

## Runtime Helpers

Known runtime helpers should be represented explicitly before emission.

```rust
pub enum MirRuntimeHelper {
    Mul,
    Div,
    Mod,
    Lsh,
    Rsh,
    SArgs,
}

pub struct MirRuntimeHelperDecl {
    pub helper: MirRuntimeHelper,
    pub target: MirRuntimeHelperTarget,
    pub abi: MirCallAbi,
    pub effects: MirEffects,
}

pub enum MirRuntimeHelperTarget {
    KnownAbsolute(u16),
    RuntimeSymbol(String),
    Deferred,
}
```

Rules:

- Use the known-helper enum for helper selection.
- Helper declarations provide target addresses, variants, ABI facts, and effects
  when emission needs them.
- Unknown helper targets are allowed only before pre-emission MIR.

## Effects And Barriers

Effects are required from the first MIR slice.

```rust
pub struct MirEffects {
    pub memory_reads: MirMemoryEffect,
    pub memory_writes: MirMemoryEffect,
    pub clobbers: MirRegisterSet,
    pub preserves: MirRegisterSet,
    pub stack_depth_delta: Option<i8>,
    pub may_call_os: bool,
    pub opaque: bool,
}

pub enum MirMemoryEffect {
    None,
    Regions(Vec<MirMemoryRegion>),
    Unknown,
    All,
}

pub struct MirMemoryRegion {
    pub kind: MirMemoryRegionKind,
    pub offset: u16,
    pub size: u16,
}

pub enum MirMemoryRegionKind {
    Local(LocalId),
    Global(GlobalId),
    Static(StaticId),
    AbsoluteRange,
    ZeroPage,
    Stack,
}

pub struct MirRegisterSet {
    pub a: bool,
    pub x: bool,
    pub y: bool,
    pub flags: bool,
    pub sp: bool,
}
```

Rules:

- Calls, runtime helpers, OS calls, machine blocks, raw data, stack operations,
  and unknown absolute memory interactions must preserve conservative ordering.
- Machine blocks are opaque by default.
- Hardware registers must not be optimized away or reordered unless explicitly
  modeled.
- `sp` models stack-pointer effects. `stack_depth_delta` may be `None` for
  opaque/unknown calls and must be balanced where the verifier is able to prove a
  routine-local stack discipline.
- `preserves` is retained from NIR effects until MIR ABI and scheduling decisions
  have consumed it.

## Initial Acceptance Profile

The first MIR6502 implementation should accept only a small scalar profile from
verifier-clean NIR. See `docs/NIR_TARGET_SHAPE.md` for the canonical definition
of each NIR operation consumed here.

Initial NIR inputs:

- scalar `Load`, `Store`, `AddrOf`, `Cast`, `Unary`, `Binary`, and `Compare`;
- `Goto`, `Branch`, `Return`, `Exit`, and `Unreachable` terminators;
- constants, temps, params, locals, globals, statics, absolute places, static
  addresses, routine addresses, and global addresses;
- direct user/runtime calls only if signatures and effects are complete.

Initial MIR outputs may contain:

- virtual temps;
- simple direct storage addresses;
- byte and word pseudo arithmetic;
- bool temp compare results;
- flag tests after materialization;
- call placeholders with conservative effects;
- barriers.

Initial MIR should reject or defer:

- unresolved NIR compatibility shapes;
- field/index/deref places not yet represented by exact address facts;
- machine blocks without structured payloads;
- indirect calls without typed callable values;
- record copies and aggregate operations;
- alias-sensitive memory optimization.

## Verifier Contract

The MIR verifier should support phase-specific validation.

### All MIR phases

Check:

- unique routine, block, temp, storage, and static IDs;
- valid references to blocks, temps, statics, globals, locals, params, helpers,
  memory regions, and machine blocks;
- every block has one terminator;
- terminator targets exist;
- operation widths are valid;
- memory destinations are written only through store-like operations;
- operation definitions target `MirDef`, not arbitrary memory;
- call ABI homes, clobbers, preserves, stack effects, and memory effects are
  present;
- machine blocks and barriers carry effects;
- `MirCond` does not duplicate compare operands;
- no executable source syntax or SemIR handles appear in MIR.

### Pre-materialization MIR

Allow:

- virtual temps;
- word pseudo ops;
- bool-value branch conditions;
- abstract storage homes;
- abstract zero-page slots;
- deferred runtime helper targets.

Reject:

- missing widths;
- unknown call effects;
- unresolved NIR compatibility forms;
- address forms without enough facts to lower later.

### Post-materialization MIR

Require:

- word pseudo ops expanded or explicitly assigned to runtime helpers;
- byte-lane `Add`/`Sub` carry behavior made explicit;
- ABI homes selected for calls;
- compare/branch forms either materialized as bool bytes, lowered to flag tests,
  or represented as explicit multi-block compare sequences;
- virtual temps assigned homes or proven acceptable for the next phase;
- concrete enough address forms for emission.

### Pre-emission MIR

Require:

- no unsupported pseudo ops;
- no unresolved storage or label references;
- no unassigned virtual temps;
- no unresolved runtime helper targets;
- no abstract zero-page slots unless the emission layer explicitly owns their
  final address assignment;
- all raw data and machine-code boundaries represented as barriers;
- all ordinary instruction work expressible through tracked emission helpers.

## Initial Implementation Slices

Suggested first commits:

```text
mir6502: document pseudo-machine contract
mir6502: add MIR observation surface
mir6502: define scalar MIR verifier profile
mir6502: map NIR storage to MIR homes
mir6502: lower scalar loads and stores from NIR
mir6502: lower scalar casts and address materialization
mir6502: lower scalar arithmetic from NIR
mir6502: lower compares and branches from NIR
mir6502: select helpers for wide operations
mir6502: emit scalar MIR through tracked emitter
```

The first code slice should create the observation surface only: module scaffold,
IR skeleton, verifier shell, printer, `--emit-mir6502` or `--emit-mir`, and one
fixture. It should not attempt full arithmetic, calls, register allocation, or
emission.

## Deferred Opcode Families

Do not fully design these until a lowering slice needs them:

- complete 6502 opcode-level pseudo ISA;
- full zero-page placement;
- general register allocation;
- dynamic indexed array addressing;
- pointer dereference address staging;
- signed relational compare sequences;
- machine-block payload preservation;
- indirect calls;
- target peepholes;
- final opcode scheduling.

Each deferred family should add MIR forms only when it represents a stable target
choice that cannot remain in NIR and should not be rediscovered by emission.
