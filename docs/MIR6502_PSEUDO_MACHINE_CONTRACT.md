# MIR6502 Pseudo-Machine Contract

Snapshot date: 2026-06-01. Updated for staged whole-record copies on
2026-08-30.

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

Runtime-helper selection may distinguish operand width from result width. In
particular, an unsigned byte multiply with a word consumer uses the target-owned
`MultB` contract: byte operands in A/X and the complete word result in A/X.
This decision is derived from typed MIR values, not from source syntax, and is
selected only when the chosen runtime makes the additional helper profitable.

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

`CompareIndirectWords` is a selected unsigned relational operation over two
already prepared, distinct indirect pointer pairs. It is legal only for `<` and
`>=`; reversible `>` and `<=` relations must be normalized before selection.
Its low-byte `CMP` produces the carry/borrow input consumed by the high-byte
`SBC`, so no intervening operation may alter carry. Emission uses one shared Y
offset, increments it for the high lane, and therefore requires both
`offset` and `offset+1` to fit in Y. The operation cannot use scaled-Y
consumers: scale-two address construction must already have been incorporated
into each prepared pointer.

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

Global initializers retain the structured NIR array fact needed by target
layout: element size, optional declared length, pointer-backed status, and any
explicit address initializer. MIR6502 layout may use those facts to place
uninitialized sized array backing in deferred storage; it must not infer array
identity from a display `kind` string.

Initialized data also retains literal bytes plus low-byte, high-byte, and
word-address relocation records. MIR6502 translates NIR targets to MIR storage
and routine IDs without consulting SemIR. Emission resolves storage targets
only after final object layout, resolves array identities to element backing
rather than descriptor cells, and leaves forward routine targets as normal
label fixups. A relocation reference makes a local or parameter home
address-observable, so ABI/home-elision passes must preserve that home.

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
    ParamAbiOnly(ParamId),
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
- `Param(ParamId)` denotes a physical parameter home. It participates in
  routine-local layout and may be referenced by MIR memory operations.
- `ParamAbiOnly(ParamId)` retains the formal's signature and direct-register ABI
  position after its private, write-only home has been proven unnecessary. It
  consumes no storage, has no storage symbol or address, and verifier-clean
  post-home MIR must not reference it as memory.
- Parameter-home elision is permitted only for ordinary internal Action ABI
  entries after a whole-routine effect query proves that the home has no read,
  address use, or non-store access. Entry capture and later ordinary stores may
  then be deleted together with the home. System-address and current-location
  entries remain ABI-observable and retain physical parameter homes. Machine
  blocks, address escape, opaque effects, or any residual non-store access also
  require the physical home.

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

The pre-home demand analysis may forward a uniquely defined, uniquely used byte
`Load` directly through A to an immediate store, compare, unary operation, or
byte binary operation. The load stays at its original program point. At most
one adjacent compiler barrier may remain between the load and consumer; this is
the volatile-load shape, and the barrier emits no instruction. The rewrite must
not delete or duplicate the load. Calls, machine blocks, additional barriers,
intervening operations, cross-block uses, and consumers that cannot accept A
retain a materialized home.

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
    FixedIndirectIndexedY { zp: MirFixedZpSlot },
    ComputedIndex { base: MirValue, index: MirValue, elem_size: u16, offset: u16 },
    PointerCell { ptr: MirMem, offset: u16 },
    PointerIndex { ptr: MirMem, index: MirValue, elem_size: u16, offset: u16 },
    Deref { ptr: MirValue, offset: u16 },
}
```

Rules:

- `MirMem` says what memory is being accessed.
- `MirAddr` says how the access will be addressed.
- Direct, indexed, pointer-cell, pointer-index, and dereference forms preserve
  structured NIR address facts until materialization.
- Do not add source-shaped address forms.
- `Field` lowering must already have a byte offset before MIR.
- `Index` lowering should use element size facts from NIR, not source syntax.
- A byte index into directly allocated local, global, static, or absolute
  storage may select `AbsoluteIndexedX` or `AbsoluteIndexedY`. The storage ID
  remains authoritative until emission resolves its address. Pointer- and
  descriptor-backed arrays must retain an indirect address strategy.
- `Deref` lowering should materialize pointer values into an explicit address
  strategy, usually a zero-page pointer pair plus `Y` for indirect-indexed work.
- `AdvanceAddress` accepts any nonzero byte scale representable by the MIR form.
  Scale 1/2 retain their compact paths; larger aggregate strides stage the
  index and add it to the address the required number of times.

### Index-preserving indirect-Y consumers

MIR6502 owns two target-specific address consumers that retain an index in Y
between address materialization and the indirect access:

- `PagedIndirectIndexedY` is valid only for a scale-one word-sized index (the
  CARD-indexed case) into a statically addressable byte array. The pointer pair
  contains the effective address page, with a zero low byte, and Y contains the
  effective address low byte. Materialization must propagate any carry from an
  unaligned static base into the pointer high byte. Only access offset zero is
  valid because changing Y could wrap without carrying into the pointer page.
- `ScaledIndirectIndexedY` is valid only for a scale-two index. The pointer pair
  contains the base with the scale carry folded into its high byte, while Y
  contains the scaled low-byte index. It supports only byte offsets zero and
  one under the verifier's ordered-access protocol.

Both forms are MIR6502 addressing decisions. NIR continues to describe the
typed base, index, element size, and byte offset without encoding A/X/Y or a
6502 pointer-pair strategy. Pointer-backed and otherwise dynamic bases do not
qualify for the paged form and retain ordinary full-address materialization.

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
- A byte `Lsh` or `Rsh` with `carry_in=FromPrevious` is a rotate through the
  carry produced by the preceding byte-lane shift. With no carry input it is a
  plain shift. `Clear` and `Set` are not valid shift inputs.
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

### Adjacent direct byte indexing

MIR6502 may canonicalize `base[index + delta]` to
`(base + delta)[index]` for byte arrays in directly allocated storage. The
canonical form uses one Y index for adjacent accesses. A selected comparison is
represented explicitly:

```rust
CompareDirectIndexedBytes {
    dst: MirCondDest,
    op: MirCompareOp,
    left: MirMem,
    right: MirMem,
    signed: bool,
}
```

The selector must require all of the following:

- both operands have the same direct storage base and byte index root;
- folded index additions are unsigned byte additions with known clear carry;
- address displacement arithmetic is checked rather than wrapping;
- a dominating unsigned guard proves `index <= 255 - max(delta)` and the index
  storage is unchanged on every path from that guard to the selected access;
- comparisons are unsigned, use a single flag test, and preserve operand order.

Emission lowers a selected comparison to `LDA base1,Y` / `CMP base2,Y` and an
adjacent same-array copy to ordinary `LDA base1,Y` / `STA base2,Y`. If the
no-wrap proof is unavailable, materialization must retain the independent index
computations so byte-index modulo behavior is unchanged.

### Destination-aware indirect word arithmetic

`IndirectWordCompound` is a post-selection MIR6502 operation for an alias-safe
word update through two prepared pointer pairs:

```rust
IndirectWordCompound {
    op: MirBinaryOp,
    target: MirAddressConsumer,
    source: MirAddressConsumer,
    offset: u16,
}
```

Its initial contract is deliberately narrow:

- only `Add` is accepted;
- target and source use distinct fixed zero-page pointer pairs;
- `$AE/$AF` is a third, disjoint compiler-reserved result pair;
- neither consumer may use scaled-Y addressing;
- `offset` and `offset + 1` must fit in a byte;
- both lanes of both inputs are read before either target lane is written;
- carry flows from the low-byte addition into the high-byte addition;
- result writes occur low lane first, then high lane;
- A, Y, flags, the fixed result scratch, and indirect memory effects are
  explicit in effect analysis.

The read-before-write rule makes identical pointers, one-byte source/target
overlap, and arbitrary runtime aliasing safe. Selection must prove the logical
destination is also one input and must reject the operation when its emitted
cost estimate is not smaller than the unfused sequence. `Sub` remains an
ordinary byte-lane sequence until its borrow schedule has equivalent runtime
coverage.

### Paired word arithmetic consumed by a comparison

A pre-home comparison selector may consume two adjacent pure word `Add`/`Sub`
producers directly when both definitions are single-use and the shared rewrite
proof shows that every removed definition is dead outside the window. The
selector must:

- evaluate both operations independently as modulo-16-bit carry/borrow chains;
- never replace `(x+k) op (y+k)` with `x op y`, because wrapping changes that
  relation;
- keep one result in the compiler-reserved `$AE/$AF` pair and the other in
  X/A, then branch from byte-comparison flags;
- normalize operand order only by an exact comparison reversal;
- reject effect barriers, unsafe source memory, fixed-scratch overlap, and
  multiple uses of either arithmetic result.

The fixed scratch is transient within the selected sequence. It must remain
explicit in MIR so zero-page reservation and effect analysis see the clobber.

### Alias-safe direct-word to indirect copy

`CopyDirectWordToIndirect` copies a word from ordinary compiler-owned storage
through one prepared destination pointer:

```rust
CopyDirectWordToIndirect {
    source: MirMem,
    destination: MirAddressConsumer,
    destination_offset: u16,
}
```

Its contract is:

- `source` is ordinary global, local, parameter, or spill storage, not absolute
  or hardware-backed memory;
- `destination` is an unscaled fixed zero-page pointer pair;
- `destination_offset` and its high lane fit in a byte;
- the source low and high lanes are both read before either indirect write;
- the destination low lane is written before its high lane;
- temporary stack staging is balanced and the stack pointer is restored;
- A, X, Y, flags, both source reads, the destination-pair reads, and the
  unknown indirect write are explicit in effect analysis.

Selection may therefore accept exact and one-byte overlap between source and
destination. Its post-home rewrite must prove A/X/Y/flags dead at the window
exit and must reject a nonpositive emitted-cost estimate.

### Alias-safe indirect byte range to fixed call homes

`CopyIndirectBytesToFixedZp` copies a bounded byte range through one prepared
pointer into consecutive fixed zero-page homes:

```rust
CopyIndirectBytesToFixedZp {
    source: MirAddressConsumer,
    source_offset: u16,
    destinations: Vec<MirFixedZpSlot>,
}
```

Its contract is:

- `source` is an unscaled fixed zero-page pointer pair;
- the operation contains two through eight consecutive destination homes;
- the complete source range fits in Y;
- every source byte is read and pushed before the first destination write;
- bytes are popped into the destinations in reverse stack order, preserving
  source-to-destination byte order;
- stack staging is balanced and restores the incoming stack pointer;
- A, Y, Z/N, the source-pair reads, the unknown indirect reads, and each exact
  fixed-ZP write are explicit in effect analysis.

The call-field selector uses this operation only for adjacent, nonoverlapping
word fields whose final homes are consumed by the immediately following call.
Any intervening argument load must be ordinary compiler-owned storage proven
disjoint from the fixed destination homes. This permits the source range to
alias the ABI homes themselves without changing argument evaluation.

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
- A signed word relation against zero may read only the high lane for `< 0` or
  `>= 0`. The `> 0` and `<= 0` forms must additionally test both lanes for
  zero, preserving the high-byte Z/N provenance across any empty intermediate
  block.

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
- A call result may be consumed directly from a `ReturnSlot` only when the call
  and consumer are adjacent, the result definition has no other live use, and
  no intervening operation can overwrite the return bytes. Removing the
  logical result temp does not weaken the call's memory or machine effects.
- When a call result is first stored to ordinary compiler-owned storage, later
  same-block uses may be redirected to that canonical stored value before
  result-home selection. The reaching call definition must be unique, all uses
  must be inside the rewritten window, and no call, machine block, barrier,
  unknown write, or write to either destination lane may intervene. This
  forwarding enables the existing adjacent call-result/store selector; it does
  not make the destination store dead.
- For the canonical four-byte Action argument prefix, selected word arithmetic
  may write the first word directly to A:X or the second directly to Y:`$A3`.
  The scheduler must place the companion word afterward only when that cannot
  overwrite an unread source, and must preserve the low-lane carry or borrow
  through the high-lane operation.
- A zero-extended indexed BYTE feeding a canonical Action word argument may be
  loaded directly into its fixed-ZP low lane when the base is identified
  compiler-owned storage, the index reads only ordinary compiler-owned
  storage, and the complete producer group is single-use and source-ordered.
  Constant additions to an element-size-one index may become an indirect-load
  offset, preserving 16-bit wrap and page crossing without a transient word
  home.
- The Y low lane is evaluated in source order and staged temporarily in `$A3`;
  after all indexed address calculations have finished, it moves to Y and
  `$A3` is initialized as the zero-extension high lane. Fixed argument homes
  must be unique, disjoint from pointer scratch `$AC-$AF`, and absent from all
  retained address inputs. Absolute and arbitrary pointer-backed indexed loads,
  indirect calls, barriers, and reordered producer groups are ineligible.

## Whole-Record Copies

NIR `CopyBytes` lowers to ordinary byte `Load` and `Store` operations. MIR6502
first loads every source lane into a distinct generated byte temporary and only
then emits the destination stores. Reusing the already-normalized address and
index values preserves single evaluation, while staging the complete value
makes self-copies and arbitrary overlap safe. Volatile source and destination
phases retain explicit barriers around their respective memory operations.

This is intentionally a correctness-first target strategy. Later target
optimization may select a smaller directional loop when structured address
facts prove the ranges disjoint or establish a safe direction, but emission
must not infer aggregate semantics or alias facts on its own.

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
- Optimized Action entries capture argument frames of up to four bytes directly;
  larger frames use `SArgs` to bound callee-prologue size.

### Atari FPP target services

Native `REAL` arithmetic uses a distinct structured call target:

```rust
pub enum MirCallTarget {
    // Ordinary source/runtime call targets omitted.
    AtariFpp(MirAtariFppService),
}

pub enum MirAtariFppService {
    IntegerToFloat, // IFP   $D9AA
    FloatToInteger, // FPI   $D9D2
    Add,       // FADD  $DA66
    Subtract,  // FSUB  $DA60
    Multiply,  // FMULT $DADB
    Divide,    // FDIV  $DB28
}
```

These identities are fixed Atari OS services. They are neither source-level
routines nor Action cartridge/standalone runtime helpers, and standalone
linking leaves them unchanged. Maps and listings expose them as Atari OS ROM
dependencies; the existing runtime-binding record is output metadata only and
does not cause an embedded routine to be linked.

Before a service call, lowering copies all six left operand bytes into FR0
(`$D4`-`$D9`) and all six right operand bytes into FR1 (`$E0`-`$E5`). After the
call it copies all six FR0 bytes to the typed destination. `PackedRealCopy`
provides the directional direct transfer and complete indirect-source staging
described below, so assignment remains correct for supported aliases. Constant
integer promotions use the same exact Atari decimal codec as literals. Dynamic
integer conversion calls IFP/FPI, whose unsigned-word convention is adapted for
signed Action `INT` values. Sign state that must survive a call uses generated
frame storage rather than a register or virtual temp.

An adjacent, single-use compiler-owned REAL result may remain in FR0 instead of
round-tripping through its six-byte frame slot. A following left-hand consumer
uses FR0 directly. A following right-hand subtraction or division first copies
FR0 to FR1 and then stages the left operand in FR0; addition and multiplication
instead stage the other operand directly in FR1. The latter rewrite relies only
on those operations' commutativity. Target lowering applies these forms only
after structured NIR use counting and an exact MIR sequence match; intervening
operand evaluation keeps the frame slot.

REAL equality and ordering compare the canonical six-byte representation
directly. Equality requires all six bytes to match; ordering first handles sign
classes and then compares bytes lexicographically, reversing same-sign order
for negative values. This preserves distinctions between adjacent packed
decimal values that subtraction-based comparison could round away. Direct REAL
conditions compare against canonical zero.

When a REAL comparison is the final NIR operation in a block and its result is
consumed only by that block's branch, MIR6502 selects `PackedRealCompare` after
staging the operands in FR0 and FR1. The operation compares sign and packed
bytes directly, leaves A as canonical Boolean zero or one, and exposes Z for
the immediate branch. It reads `$D4`-`$D9` and `$E0`-`$E5`, clobbers A and C,
and writes N/Z. The verifier requires it to be the final operation before a
Z-flag branch. Comparisons whose result is stored, returned, or otherwise used
as a value retain the ordinary explicit Boolean-producing lowering.

Aggregate REAL assignment lowers to one `PackedRealCopy` carrying structured
source and destination addresses plus byte offsets. Materialization prepares
independent fixed-ZP pointer pairs for indirect operands; pre-emission MIR
permits only direct or `(zp),Y` copy endpoints, and each indirect offset must
leave room for all six bytes. Ordinary direct ranges use one descending
X-indexed loop; a statically known leftward overlap retains a forward fallback
for explicit absolute aliases. If either endpoint is indirect, emission uses
two Y-indexed loops: the first pushes all six source bytes before any write and
the second pulls and stores them in reverse lane order. The stack is balanced
on exit, pointer aliases retain copy semantics, and the six lanes never become
six simultaneously live compiler temporaries.

The same operation implements native REAL negation with its `negate` flag.
After copying, it tests the six-byte magnitude and toggles bit 7 of byte zero
only for a nonzero value; zero is normalized to its canonical positive form.
This avoids staging zero and the operand in FR0/FR1 and avoids an Atari FPP
subtraction call. The operation clobbers A and N/Z, clobbers X for direct copies
or Y/C for an indirect endpoint, preserves V, and uses only transient balanced
stack storage. A negated immutable static source with one same-block consumer
may carry that flag directly to the consumer's packed copy, eliminating its
private REAL frame slot. Initialized scalar/array storage already contains
authoritative packed-decimal bytes; FPP calls are needed only for runtime
computation.

The audited core FPP services clobber A, X, Y, flags, and service-specific
subsets of the Atari FPP workspace. MIR uses the stable portable envelope for
compatible ROMs: structured zero-page reads and writes over `$D4-$FF`. These
calls are not opaque, do not make nested OS calls, and have a known balanced
stack-depth delta of zero. The verifier requires that exact contract. Emission
implements an Atari FPP call as `JSR service; CLD`, restoring the compiler's
binary-arithmetic invariant because compatible packages do not provide a
portable decimal-flag result.

The allocator reserves `$D4-$FF` in every routine containing an FPP call, so
virtual zero-page homes cannot overlap the workspace. Structured-effect
analysis records those bytes as exact fixed-zero-page homes rather than
turning them into unknown compiler-home effects. Consequently a pointer or
other value held in an ordinary param, local, spill, or the `$AA-$AF` pointer
scratch may be rematerialized across an FPP call. Values stored in the FPP
workspace itself remain killed normally. The byte-level source audit and the
original/AltirraOS compatibility union are recorded in
`docs/Action_2027/ATARI_FPP_ORACLE.md`.

This preservation proof applies only to the FPP call. It does not make a
pointer snapshot removable when its live range also crosses an indirect write:
without a stronger alias fact, an indirect `PackedRealCopy` may overwrite the
pointer cell itself, so the original pointer value still needs a durable home.

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
- MIR6502 may replace that default only after recognizing an exact instruction
  and data-flow contract. The current narrow case is a terminal
  `JMP (word-local)` whose vector was loaded from a compiler-known table
  containing only parameterless Action routines. It reads the two-byte local
  vector, observes no incoming register/flag/pointer-scratch state, clobbers the
  Action call-volatiles, and remains an all-memory-write barrier for the
  tail-dispatched routine. Other machine payloads stay opaque.
- Structured named local, parameter, global, static, and stack regions do not
  alias compiler spill or zero-page homes. Unknown/all-memory, absolute-range,
  and zero-page regions remain conservative home-liveness barriers.
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
- aggregate operations other than verified whole-record `CopyBytes`;
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
