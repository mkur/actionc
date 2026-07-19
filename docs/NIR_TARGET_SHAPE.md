# NIR Target Shape

Snapshot date: 2026-05-31.

This document describes the recommended final shape of NIR, the Normalized
Intermediate Representation that should grow out of the current `tac` module.
It is the target contract for migration work, optimizer work, and the future
MIR6502 consumer.

This document is intentionally aspirational. The current `tac` implementation is
transitional and may temporarily contain legacy forms while it is being migrated.
Verifier-clean NIR is the end state described here.
During the migration, `actionc-emit --emit-nir` is backed by the transitional `tac`
module. The old `--emit-tac` observation alias has been removed. `fixtures/nir`
is the optimizer-facing snapshot contract; `fixtures/tac` remains as
transitional compatibility coverage for historical TAC shapes.

## Position In The Compiler

The intended pipeline is:

```text
Action source -> AST -> semantic model -> SemIR -> NIR -> MIR6502 -> emission
```

NIR is the final Action!-aware normalized IR and the first optimizer-grade IR.
It should be low enough that expressions, storage, branches, calls, and effects
are explicit, but high enough that it does not commit to 6502 registers,
addressing modes, or final instruction forms.

## Core Responsibilities

NIR owns:

- routine, block, and terminator structure;
- stable IDs for MIR-relevant entities;
- typed temps and typed values;
- explicit loads and stores;
- explicit casts, unary ops, binary ops, compares, and branches;
- explicit address-of and address-shaped storage facts;
- static data references;
- call signatures and conservative effects;
- machine-block barriers and payloads when available;
- verifier guarantees strong enough for optimization.

NIR must not own:

- source parsing;
- source-level name resolution;
- source-level type checking;
- Action! lvalue legality decisions;
- 6502 register allocation;
- 6502 addressing-mode selection;
- final instruction emission;
- source syntax as executable semantics.

## Non-Goals

Verifier-clean NIR is not:

- printed TAC text;
- AST-shaped syntax;
- a collection of expression summary strings;
- a 6502 instruction stream;
- a final storage allocator;
- an SSA-only IR.

NIR may later gain an SSA view or analysis layer, but the recommended base form
is explicit basic blocks with single-definition temps and verified use-def facts.

## Top-Level Shape

Recommended Rust-like target shape:

```rust
pub struct NirProgram {
    pub globals: Vec<NirGlobal>,
    pub statics: Vec<NirStaticData>,
    pub routines: Vec<NirRoutine>,
    pub signatures: Vec<NirSignature>,
    pub machine_blocks: Vec<NirMachineBlock>,
}

pub struct NirRoutine {
    pub id: RoutineId,
    pub name: String,
    pub signature: SignatureId,
    pub params: Vec<NirParam>,
    pub locals: Vec<NirLocal>,
    pub temps: Vec<NirTemp>,
    pub blocks: Vec<NirBlock>,
    pub effects: NirRoutineEffects,
    pub notes: Vec<NirRoutineNote>,
}

pub struct NirBlock {
    pub id: BlockId,
    pub label: String,
    pub params: Vec<NirBlockParam>,
    pub ops: Vec<NirOp>,
    pub terminator: NirTerminator,
}

pub struct NirBlockParam {
    pub dest: TempId,
    pub ty: NirType,
}
```

Display names such as routine names, block labels, local names, and global names
are metadata for printing and diagnostics. Stable IDs are the executable
identity.

Routine-entry facts that affect calling convention are structured metadata, not
printer strings. In particular, a source `=*` entry carries a structured
current-location entry kind so MIR6502 can preserve the public Action ABI
boundary without parsing a displayed note.

## Stable IDs

NIR should use stable ID newtypes for every entity that MIR6502 or optimizer
passes need to reference:

```rust
pub struct RoutineId(pub u32);
pub struct BlockId(pub u32);
pub struct TempId(pub u32);
pub struct ParamId(pub u32);
pub struct LocalId(pub u32);
pub struct GlobalId(pub u32);
pub struct StaticId(pub u32);
pub struct SignatureId(pub u32);
pub struct MachineBlockId(pub u32);
pub struct BuiltinId(pub u32);
```

String names should not be executable identity in verifier-clean NIR. A printer
may map IDs back to labels and names for readability.

## Types

NIR types should preserve the machine-relevant semantic facts from SemIR:

```rust
pub enum NirType {
    Void,
    Bool,
    U8,
    I8,
    U16,
    I16,
    Ptr16 {
        pointee: Option<Box<NirType>>,
    },
    Record {
        record: RecordId,
        size: u16,
    },
    Callable {
        signature: SignatureId,
    },
}
```

Required type facts:

- width in bytes;
- signedness for arithmetic and comparisons;
- pointer-ness and pointee facts where known;
- record identity and size;
- callable signature for routine values and indirect calls.

Recommended width rules:

```text
Void      -> 0 bytes
Bool      -> 1 byte
U8/I8     -> 1 byte
U16/I16   -> 2 bytes
Ptr16     -> 2 bytes
Callable  -> 2 bytes
Record    -> known record size
```

Signedness should be derived from `NirType`, not from opcode names or display
strings. For Action!, `INT` should lower to `I16`; `CARD`, pointers, and raw
addresses should lower to unsigned word-like behavior unless a specific operation
requires otherwise.

## Values

Value operands are already-materialized values. They are not places.

```rust
pub enum NirValue {
    ConstU8(u8),
    ConstU16(u16),
    StaticAddr(StaticId),
    RoutineAddr(RoutineId),
    Temp(TempId),
    Param(ParamId),
    GlobalAddr(GlobalId),
}
```

Rules:

- Constants are numeric and width-shaped; source literal text is metadata only.
- `Temp` values get their type from the routine temp table.
- `Param` values get their type from the routine parameter table.
- `StaticAddr`, `RoutineAddr`, and `GlobalAddr` are address-valued and should be
  compatible with a 16-bit pointer/callable type.
- Values must never be raw expression strings.

## Places

Places describe storage that can be loaded from, stored to, or addressed.

```rust
pub struct NirPlace {
    pub kind: NirPlaceKind,
    pub ty: NirType,
}

pub enum NirPlaceKind {
    Param(ParamId),
    Local(LocalId),
    Global(GlobalId),
    Static(StaticId),
    Absolute(u16),
    Deref {
        addr: NirValue,
    },
    Field {
        base: Box<NirPlace>,
        offset: u16,
    },
    Index {
        base_addr: NirValue,
        index: NirValue,
        elem_size: u8,
    },
}
```

Rules:

- `Symbol(String)` is not allowed as executable storage identity.
- Field access stores byte offsets, not field names.
- Index access stores semantic element size, not source syntax.
- Dereference and index forms use values, not legacy operands.
- Source syntax may be kept only as metadata for diagnostics or source maps.

## Operations

Recommended core operation set:

```rust
pub enum NirOp {
    Load {
        dest: TempId,
        place: NirPlace,
    },
    Store {
        place: NirPlace,
        src: NirValue,
    },
    AddrOf {
        dest: TempId,
        place: NirPlace,
    },
    Cast {
        dest: TempId,
        src: NirValue,
        from: NirType,
        to: NirType,
    },
    Unary {
        dest: TempId,
        op: NirUnaryOp,
        src: NirValue,
        ty: NirType,
    },
    Binary {
        dest: TempId,
        op: NirBinaryOp,
        left: NirValue,
        right: NirValue,
        ty: NirType,
    },
    Compare {
        dest: TempId,
        op: NirCompareOp,
        left: NirValue,
        right: NirValue,
        operand_ty: NirType,
    },
    Call {
        callee: NirCallee,
        args: Vec<NirValue>,
        result: Option<TempId>,
        signature: SignatureId,
        effects: NirEffects,
    },
    MachineBlock {
        id: MachineBlockId,
    },
}
```

Recommended operator sets:

```rust
pub enum NirUnaryOp {
    Plus,
    Neg,
    BitNot,
    LogicalNot,
}

pub enum NirBinaryOp {
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

pub enum NirCompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
```

Rules:

- `Load.dest`, arithmetic destinations, compare destinations, call results, and
  cast destinations define temps.
- The temp table is the type authority for temp IDs.
- `Compare` result type is always `Bool`; signedness comes from `operand_ty`.
- Expensive operations such as word multiply/divide may remain semantic NIR ops
  and lower to runtime helpers in MIR6502.
- NIR should not encode final 6502 addressing modes.

## Terminators

Every block has exactly one terminator.

```rust
pub enum NirTerminator {
    Goto(NirEdge),
    Branch {
        condition: NirValue,
        then_edge: NirEdge,
        else_edge: NirEdge,
    },
    Return(Option<NirValue>),
    Exit,
    Unreachable,
}

pub struct NirEdge {
    pub target: BlockId,
    pub args: Vec<NirValue>,
}
```

Rules:

- Branch targets are `BlockId`, not strings.
- Edge argument arity and types exactly match the target block parameters.
- Edge arguments are uses at the predecessor terminator; block parameters are
  definitions at target block entry.
- Every parameterized block has at least one predecessor contribution.
- Branch conditions must be `Bool` values, or a future explicitly documented test
  terminator must be added.
- There is no `Open` terminator in verifier-clean NIR.
- There is no `Unknown(String)` terminator in verifier-clean NIR.
- Fallthrough should either be made explicit as `Return(None)` where appropriate
  or represented by a documented terminator with clear MIR6502 behavior.

### MIR6502 merge handoff

Pre-materialization MIR6502 preserves the same typed merge contract with
`MirBlockParam` and typed `MirEdgeArg` values. This is a target representation,
not a request for NIR to choose registers or storage.

Before ordinary 6502 materialization, conditional edges that carry arguments
are split so their copies execute only on the selected edge. Each argument is
then lowered to its target parameter temp as a parallel copy. Copy cycles use a
fresh MIR temp, whose eventual register, zero-page, or spill placement remains
a MIR6502 decision. Post-materialization and pre-emission MIR contain neither
block parameters nor edge arguments.

## Conditions

Recommended initial condition model: value-producing bool temps.

Examples:

```text
%0:u8   = load skstat
%1:u8   = binary And %0, #$04
%2:bool = compare Ne %1, #0 operand_ty=u8
branch %2, bb_then, bb_else
```

Rules:

- Bitwise expressions in conditions are materialized and tested against zero.
- Short-circuit `AND` and `OR` lower to explicit CFG blocks.
- Negation lowers either to condition CFG inversion or to a bool-producing op.
- Constant conditions may be folded by a NIR optimization pass after verification.

A later MIR6502 pass may fuse compare/test/branch patterns to use processor
flags directly. NIR should prefer clarity and verifier simplicity.

## Calls

Recommended call shape:

```rust
pub enum NirCallee {
    User(RoutineId),
    Builtin(BuiltinId),
    Runtime {
        name: String,
        address: Option<u16>,
        signature: SignatureId,
    },
    Indirect {
        target: NirValue,
        signature: SignatureId,
    },
}

pub struct NirSignature {
    pub id: SignatureId,
    pub params: Vec<NirType>,
    pub result: Option<NirType>,
    pub abi: NirAbiClass,
}
```

Rules:

- Indirect callees use typed values, not expression summary strings.
- Call argument count and types are verified against the signature.
- Call result temps are verified against the signature result type.
- ABI class is known to NIR, but physical ABI placement belongs to MIR6502.

## Effects

Effects must be conservative enough to protect optimization around memory,
hardware, runtime calls, OS calls, and machine blocks.

```rust
pub struct NirEffects {
    pub memory_reads: NirMemoryEffect,
    pub memory_writes: NirMemoryEffect,
    pub may_call_os: bool,
    pub opaque: bool,
}

pub enum NirMemoryEffect {
    None,
    Regions(Vec<NirMemoryRegion>),
    Unknown,
    All,
}

```

Rules:

- NIR effects describe target-independent memory and ordering behavior only.
- Physical registers, condition flags, stack state, and ABI volatility are not
  represented in NIR. MIR derives those facts from the selected target and ABI.
- Unknown or opaque effects are full ordering barriers unless a later effect model
  proves a narrower behavior.
- Runtime and OS calls should be conservative by default.
- Absolute memory and hardware-register interactions must not be optimized away
  unless facts prove it is safe.
- Machine blocks are opaque by default.

## Static Data

Recommended static data shape:

```rust
pub struct NirStaticData {
    pub id: StaticId,
    pub name: String,
    pub ty: NirType,
    pub bytes: Vec<u8>,
    pub alignment: u8,
    pub section: NirStaticSection,
    pub mutable: bool,
    pub display: String,
}
```

Rules:

- `bytes` is authoritative for emitted data.
- `display` is for diagnostics and fixtures only.
- `StaticAddr(id)` must reference an existing static data entry.
- String representation policy should be documented at this boundary.

## Machine Blocks

Recommended machine block shape:

```rust
pub struct NirMachineBlock {
    pub id: MachineBlockId,
    pub items: Vec<NirMachineItem>,
    pub effects: NirEffects,
    pub source_span: Option<SourceSpan>,
}

pub enum NirMachineItem {
    Byte(u8),
    Word(u16),
    LabelDef(BlockId),
    LabelRef(BlockId),
    GlobalRef(GlobalId),
    StaticRef(StaticId),
    RawTextForDebug(String),
}
```

Rules:

- Machine blocks must either carry enough payload for MIR6502/emission to
  preserve them or produce a precise unsupported diagnostic before MIR6502.
- Formatted effect strings are not optimizer-grade effects.
- Default effects should be opaque and conservative.

## CFG And Temp Facts

NIR routines should expose or be able to derive:

```rust
pub struct NirCfg {
    pub entry: BlockId,
    pub preds: Vec<Vec<BlockId>>,
    pub succs: Vec<Vec<BlockId>>,
}

pub struct NirTemp {
    pub id: TempId,
    pub ty: NirType,
    pub def: NirTempDef,
    pub source_span: Option<SourceSpan>,
}

pub struct NirTempDef {
    pub block: BlockId,
    pub op_index: usize,
}
```

Verifier-clean NIR requires:

- every temp has exactly one definition;
- every temp use has a known type;
- every use is dominated by its definition or is accepted by a documented
  conservative dataflow rule;
- terminator targets exist;
- CFG predecessor/successor facts match terminators;
- unreachable blocks are either allowed explicitly or removed by cleanup.

## Verifier Contract

The NIR verifier should check at least:

- unique IDs in each table;
- valid references to routines, blocks, params, locals, globals, statics,
  signatures, temps, and machine blocks;
- every block has exactly one valid terminator;
- no verifier-clean block contains metadata as an executable op;
- no legacy operand or stringly executable shape appears in migrated profiles;
- temp single-definition and use-def validity;
- type compatibility for loads, stores, casts, unary ops, binary ops, compares,
  calls, returns, and branches;
- static address references are valid;
- call arity, argument types, result types, and effect facts are valid;
- branch conditions are bool/condition values;
- unsupported source constructs are rejected before NIR or represented by precise
  unsupported diagnostics that cannot reach optimization/codegen as normal ops.

## Optimization Readiness

Optimizer passes may run only on verifier-clean NIR.

Initial safe NIR passes:

- CFG cleanup;
- unreachable block removal;
- constant folding;
- constant condition folding;
- routine-wide constant, copy, and algebraic-identity propagation with
  conservative joins and sparse propagation over executable branch edges;
- branch simplification;
- routine-wide, liveness-based dead temp elimination;
- local load/store forwarding only when storage identity and effects make it
  safe.
- pruned private-scalar promotion, using storage live-in sets and iterated
  dominance frontiers to introduce typed block parameters only at required
  merges;
- explicit synchronization before effects that may read a promoted home and a
  reload after effects that may write it.

Promotion does not make a target allocation decision. NIR removes direct
source-home traffic and represents merged values with block parameters and edge
arguments. MIR owns the transient home, register, and spill strategy. The
initial automatic policy is deliberately pressure guarded: it promotes hot
ordinary byte locals with small definition sets, while initialized,
address-taken, aliased, absolute, machine-visible, wider, parameter, and colder
homes remain in storage form until target home coloring can carry them without
regressing output.

Do not perform aggressive alias-sensitive optimization until all of these are
strong enough:

- structured storage identity;
- call and machine-block effects;
- absolute memory policy;
- pointer dereference policy;
- dominance/use-def validation;
- volatile or hardware-register modeling.

Target-specific optimizations such as zero-page placement, compare/branch flag
fusion, indexed addressing selection, helper selection, and peepholes belong in
MIR6502 or later.

## Mapping From Current TAC To Target NIR

The current TAC typed core maps naturally to NIR:

```text
TacProgram        -> NirProgram
TacRoutine        -> NirRoutine
TacBlock          -> NirBlock
TacTypeKind       -> NirType
TacValue          -> NirValue
TacPlace          -> NirPlace
TacOp::Load       -> NirOp::Load
TacOp::Store      -> NirOp::Store
TacOp::AddrOf     -> NirOp::AddrOf
TacOp::Unary      -> NirOp::Unary
TacOp::Cast       -> NirOp::Cast
TacOp::Binary     -> NirOp::Binary
TacOp::Compare    -> NirOp::Compare
TacOp::Call       -> NirOp::Call
TacTerminator     -> NirTerminator
```

Current transitional forms should not survive as verifier-clean NIR:

```text
TacOperand                         -> remove from executable paths
TacPlaceKind::Symbol(String)       -> Param/Local/Global/Absolute IDs
TacPlaceKind::Field { field }      -> Field { offset }
TacPlaceKind::Index { syntax }     -> Index { base_addr, index, elem_size }
TacCallee::Indirect { target }     -> Indirect { target: NirValue }
TacOp::Assign                      -> Store
TacOp::CompoundAssign              -> Load/Binary/Store
TacOp::Set                         -> Store Absolute or explicit init segment
TacOp::Define/Declare/Note         -> metadata tables
TacOp::MachineBlock { effects }    -> structured payload and effects
TacTerminator::Goto(String)        -> Goto(BlockId)
TacTerminator::Branch string labels -> Branch with BlockId targets
TacTerminator::Open/Unknown        -> reject before verifier-clean NIR
```

## Red Lines

Do not consider NIR complete while any of these are true:

- optimizer passes run on legacy/stringly TAC shapes;
- MIR6502 consults SemIR to recover missing NIR facts;
- executable code uses `TacOperand`;
- executable storage identity depends on `Symbol(String)`;
- executable field/index forms preserve source syntax instead of semantic facts;
- calls lack signatures or conservative effects;
- machine blocks lack payload/effect handling or an explicit unsupported barrier;
- cross-block temp use is not verified;
- branch conditions are not typed or explicitly tested;
- verifier-clean IR can contain unknown/open boundaries.
