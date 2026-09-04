# NIR Target Shape

Snapshot date: 2026-09-04. Updated for the target-parameterized NIR contract,
typed target-independent sizes, offsets, and absolute addresses, the completed
TAC-to-NIR naming migration, address-based native `REAL`
operations, cartridge-compatible integer arithmetic typing, explicit record
copies, and the native scalar/callable type surface.

This document describes the target shape of NIR, the Normalized Intermediate
Representation implemented under `src/nir`. It is the contract for hardening,
optimizer work, and independent MIR6502, MIR65816, and MIR68K consumers.

Parts of this document remain aspirational where legacy NIR variants are still
representable internally. The verifier rejects those variants from executable
NIR, and `fixtures/nir` is the optimizer-facing snapshot contract. There is no
separate TAC module or TAC fixture contract.

## Position In The Compiler

The intended pipeline is:

```text
Action source -> AST -> semantic model -> SemIR
              -> target-parameterized NIR
              -> MIR6502 | MIR65816 | MIR68K
              -> target emission and linking
```

NIR is the final Action!-aware normalized IR and the first optimizer-grade IR.
It should be low enough that expressions, storage, branches, calls, and effects
are explicit, but high enough that it does not commit to registers, addressing
modes, byte order, bank-register state, or final instruction forms.

## Core Responsibilities

NIR owns:

- routine, block, and terminator structure;
- stable IDs for MIR-relevant entities;
- typed temps and typed values;
- explicit loads and stores;
- explicit casts, unary ops, binary ops, compares, and branches;
- destination-passing operations for address-only native `REAL` values;
- explicit address-of and address-shaped storage facts;
- the resolved target data-layout contract used by all storage facts;
- static data references;
- call signatures and conservative effects;
- machine-block barriers and payloads when available;
- verifier guarantees strong enough for optimization.

NIR must not own:

- source parsing;
- source-level name resolution;
- source-level type checking;
- Action! lvalue legality decisions;
- target register allocation;
- target addressing-mode selection;
- final instruction emission;
- source syntax as executable semantics.

## Non-Goals

Verifier-clean NIR is not:

- printed or source-summary text;
- AST-shaped syntax;
- a collection of expression summary strings;
- a target instruction stream;
- a final storage allocator;
- an SSA-only IR.

NIR may later gain an SSA view or analysis layer, but the recommended base form
is explicit basic blocks with single-definition temps and verified use-def facts.

## Target Parameterization

Target-independent does not mean target-free. Every verifier-clean
`NirProgram` is produced under one explicit, resolved data-layout contract.
The operation, CFG, storage, type, and effect vocabulary is shared by all
targets; widths, alignments, address spaces, and aggregate offsets are facts in
that contract or in the program facts derived from it.

The compiler boundary separates five concerns:

| Concern | Owns | Does not belong in NIR operations |
| --- | --- | --- |
| CPU | instruction set and architectural state | A/X/Y, D/A registers, flags, 65816 M/X or bank state |
| Platform | memory map, external symbols, hardware regions | Atari OS addresses as generic language meaning |
| Data layout | endian, address width, pointer classes, alignment and aggregate policy | instruction selection for aligned or unaligned access |
| Runtime ABI | externally visible signatures and target bindings | physical argument registers, stack slots, helper sequences |
| Output format | sections, relocatable objects, load files and entry records | XEX segments, 65816 bank records, 68k executable headers |

NIR stores a complete stable layout value or layout ID plus all resolved facts
needed by a backend. A backend must not consult SemIR to recompute field
offsets, element strides, storage extents, pointer classes, signatures, or
effects.

### Verified backend handoff

`backend::VerifiedNir` is the only unchecked-to-backend transition. Its
constructor runs the complete NIR verifier and exposes three read-only inputs:
the verifier-clean program, its resolved `TargetLayout`, and its selected
`NirRuntimeBinding` table. The common `NirBackend` interface rejects a target
that the selected backend does not advertise before target lowering begins.

MIR6502, MIR65816, and MIR68K implement separate entry points behind this
boundary. They may share analyses whose inputs and outputs remain entirely in
NIR vocabulary, but they must not share register models, instruction forms,
physical calling conventions, object formats, linker policy, or listing
syntax. Compatibility wrappers may accept an unchecked `NirProgram`, but must
construct a `VerifiedNir` token before invoking target lowering.

The current MIR65816 and MIR68K entry points are contract canaries, not code
emitters. They independently lower scalar arithmetic, address forms, aggregate
copies, branches, calls, returns, and typed relocations. MIR65816 records the
native three-byte and small-model two-byte pointer policies on a 24-bit
architecture. MIR68K projects integer data as big-endian, retains four-byte
data and code pointers, and selects bytewise access for a 16-bit value unless
its base alignment and even displacement prove a legal 68000 word access.

Action! scalar meaning remains fixed: `BYTE` and `CHAR` are 8-bit, `CARD` and
`INT` are 16-bit, and `LONG` and `ULONG` are 32-bit. `ADDRESS` and `SIZE` are
distinct unsigned integer roles whose widths come from the selected target
layout. Pointer and callable widths are selected by their address spaces and
are not aliases for `CARD` on native 65816 or 68k targets. The classic Atari
ABI may retain the historical 16-bit pointer/`CARD` interoperability as an
explicit compatibility rule.

Classic Atari records remain packed. A native ABI may use target-natural field
and tail alignment, but layout is resolved once before NIR optimization and is
never changed as an optimization. Packed records remain available for hardware
maps and external byte layouts; MIR68K must lower an unaligned word field to a
safe byte sequence when alignment is not proven.

Semantic layout facts now retain each record's final size, required alignment,
tail padding, stable field IDs, field offsets, and field alignments. Array facts
retain element size, element alignment, padded stride, optional element count,
and complete storage extent. NIR receives the resolved record sizes, field
offsets, copy extents, index strides, and target-sized descriptor extents; MIR
backends do not walk source record declarations. An initialized sized-array
descriptor occupies one data pointer plus the Action! two-byte size word, while
an unsized descriptor occupies one data pointer. Callable-address descriptors
use the selected code-pointer width.

Portable source observes layout through the canonical compile-time intrinsics
`SYS.SIZEOF`, `SYS.ELEMENTS`, `SYS.ALIGNOF`, and `SYS.OFFSETOF`. Their
unqualified compatibility-prelude aliases are ordinary shadowable names. The
queries fold during semantic/layout lowering, so executable NIR receives their
role-preserving `SIZE` constant results and resolved layout facts rather than
source-level intrinsic operations. Semantic array lengths, record sizes, field
offsets, alignments, and strides remain at least 32-bit until they cross into
checked `ByteSize` and `ByteOffset` NIR facts; no `CARD`-sized host field limits
native objects to 64 KiB.

## Routine Activation And Automatic Storage

The target ABI selects a routine activation model independently from physical
register and stack placement. The classic Atari ABI retains one fixed
parameter/local storage block per routine. A native ABI uses reentrant
activations: parameters and ordinary locals denote a distinct dynamic object
for every invocation, including recursive and concurrently re-entered calls.

SemIR owns the source meaning of declarations, aliases, initialization, and
storage duration. Verifier-clean NIR now carries:

- a structured routine identity, signature, call-convention identity, and
  activation model;
- automatic versus routine-static duration for every storage-bearing local;
- final target-selected size and alignment for every local cell, descriptor,
  parameter, and aggregate backing object.

Storage analysis interprets an automatic `LocalId` or `ParamId` in the owning
routine's current invocation, while classic storage identities remain
routine-wide. Alias facts retain their target identity and its duration.
Address-taking, aggregate copies, volatile operations, foreign-code metadata,
and opaque escape barriers keep affected automatic objects addressable. The
lowerer represents native initialization as ordinary operations at the start
of the entry block: scalar and pointer stores, immutable-template `CopyBytes`,
and descriptor stores. Descriptor-backed arrays have a distinct hidden
automatic backing `LocalId`; their descriptor receives that invocation's
backing address on every entry. Uninitialized automatic objects carry neither
a load-time image nor implicit zeroing.

Optimization uses the same identity domain. An automatic scalar is proven
private to its current invocation only while no address-forming operation
requires its storage. Unknown and recursive calls cannot name that private
dynamic instance, so its value may remain promoted or forwarded across the
call; a recursive callee's equal lexical `LocalId` denotes a different object.
Routine-static homes keep the shared-cell call barrier. Address-taking,
aliases, volatile or aggregate access, explicit effect regions, and foreign
visibility conservatively retain an addressable home. Every storage pass
re-verifies NIR, including activation/duration consistency, after rewriting.

NIR does not assign a stack offset. MIR68K and MIR65816 independently select
register homes, frame objects, stack or software-frame layouts, prologues,
epilogues, and physical call sequences. MIR6502 continues to map classic
routine storage to its established fixed locations. An automatic object must
never acquire a load-time relocation or be silently promoted to static storage
because its address escapes.

The executable NIR now carries structured routine IDs, callable signatures,
call-convention identities, entry classifications, activation models, storage
durations, and final object layouts. It describes automatic storage without
choosing stack offsets or registers. The verifier rejects load-time
relocations to automatic storage as well as malformed alias duration, target,
cycle, and ownership facts. The complete sliced migration is specified in
[`NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md`](NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md).

The detailed migration order is recorded in
[`NIR_TARGET_INDEPENDENCE_IMPLEMENTATION_PLAN.md`](NIR_TARGET_INDEPENDENCE_IMPLEMENTATION_PLAN.md).
The byte-exact Atari guardrail is recorded in
[`NIR_ATARI_BASELINES.md`](NIR_ATARI_BASELINES.md).

## Top-Level Shape

The initial executable contract is implemented by `src/target.rs` and the
`target_layout` fields on `SemProgram` and `NirProgram`. `TargetId` selects one
of four complete registered layouts; verifier-clean NIR rejects a layout whose
contents do not match its ID. The Atari layout remains the default and is
omitted from normal IR printing to keep established fixtures stable. Candidate
target layouts are printed explicitly during NIR inspection.

Recommended Rust-like target shape:

```rust
pub struct NirProgram {
    pub target_layout: NirTargetLayout,
    pub globals: Vec<NirGlobal>,
    pub statics: Vec<NirStaticData>,
    pub routines: Vec<NirRoutine>,
    pub signatures: Vec<NirSignature>,
    pub machine_blocks: Vec<NirMachineBlock>,
}

pub struct NirRoutine {
    pub id: RoutineId,
    pub name: String,
    pub signature: NirCallableSignature,
    pub convention: NirCallConvention,
    pub activation: NirActivationModel,
    pub entry: NirRoutineEntry,
    pub params: Vec<NirParam>,
    pub locals: Vec<NirLocal>,
    pub temps: Vec<NirTemp>,
    pub blocks: Vec<NirBlock>,
    pub effects: NirRoutineEffects,
    pub notes: Vec<NirRoutineNote>,
}

pub struct NirParam {
    pub id: ParamId,
    pub duration: NirStorageDuration,
    pub layout: NirObjectLayout,
    // display name, storage class, and value type omitted
}

pub struct NirLocal {
    pub id: LocalId,
    pub duration: NirStorageDuration,
    pub layout: NirObjectLayout,
    // display metadata, type, backing, and initializer omitted
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

The current implementation still contains Atari runtime-symbol and 6502
machine-payload forms. Data images are endian-neutral: typed integers and
addresses remain logical fragments until a backend projects them. NIR no
longer uses Action! `CARD`/raw `u16` fields for storage extents, byte offsets,
or absolute addresses. The remaining target-specific forms are migration
inputs, not the portable contract.
Verifier tightening must remove each old form once its replacement lands so a
backend cannot silently recover the old assumption.

Display names such as routine names, block labels, local names, and global names
are metadata for printing and diagnostics. Stable IDs are the executable
identity.

`NirRoutine.locals` contains storage-bearing parameters and declarations only.
Source-local `TYPE` and `RECORD` declarations are consumed into semantic type,
layout, field-offset, and record-size facts; they do not acquire `LocalId`
values or MIR6502 frame slots. Lexical shadowing may produce duplicate local
display names in one routine, but each stored declaration has a distinct
`LocalId`, and target labels derive uniqueness from that ID rather than from
the display name.

Routine-entry facts that affect calling convention are structured metadata, not
printer strings. In particular, a source `=*` entry carries a structured
current-location entry kind so MIR6502 can preserve the public Action ABI
boundary without parsing a displayed note.

The structured `NirRoutineEntry.program` fact records Action!'s source rule
that the last code-emitting `PROC` is the program entry. `Main` has no special
entry-point meaning, a trailing function cannot replace the entry, and runtime
routines linked after the application must not inherit or override this fact.
MIR6502 uses it when emitting Atari `RUNAD`; any corresponding note is debug
metadata only.

Source `ORG` is root-program placement metadata owned by SemIR. SemIR resolves
its constant expression to a numeric address, after which compiler orchestration
selects the effective origin using command-line override precedence. `ORG` does
not become a `SemItem`, executable NIR operation, or target instruction; NIR and
MIR6502 receive the already selected materialization origin.

Sized-array backing expressions follow the same ownership rule. Semantic
analysis resolves literal and qualified-`CONST` arithmetic to the structured
`SemDeclarationStorage::Array.fixed_address` fact, rejecting runtime-dependent
or out-of-range expressions. NIR projects that exact address to
`NirGlobalBacking::Absolute` and `NirArrayFact::address_initializer`; it does
not re-evaluate source syntax or ask MIR6502 to resolve a SemIR name. The source
expression remains only as readable initializer metadata. Verifier-clean NIR
therefore guarantees that executable fixed-array consumers use the resolved
storage identity and an address-space-qualified target address.

When that procedure also has a current-location (`=*`) entry, MIR6502 retains a
combined program-entry/observable-ABI classification; choosing it for `RUNAD`
must not enable private-entry parameter-home optimizations.

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
pub struct RuntimeSymbolId(pub u32);
pub struct MachineBlockId(pub u32);
pub struct BuiltinId(pub u32);
```

String names should not be executable identity in verifier-clean NIR. A printer
may map IDs back to labels and names for readability.

Routine addresses are carried as `NirValue::RoutineAddr` with a stable routine
ID. They are not encoded as `AddrOf` on a name-bearing or synthetic global
place. MIR6502 maps that value directly to its routine-address form.

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
    Real,
    Pointer {
        pointee: Option<Box<NirType>>,
        address_space: AddressSpaceId,
    },
    Record {
        record: RecordId,
        size: ByteSize,
    },
    Callable {
        signature: SignatureId,
        convention: NirCallConvention,
        address_space: AddressSpaceId,
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
Pointer   -> selected data-pointer width
Callable  -> selected code-pointer width
Real      -> 6 bytes, address-only
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
    Null { ty: NirType },
    AddressConst { address: AddressValue, ty: NirType },
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
- `StaticAddr`, `RoutineAddr`, and `GlobalAddr` are address-valued and carry or
  resolve to the selected pointer/callable type.
- Values must never be raw expression strings.
- Native `REAL` is not a `NirValue`: a six-byte value remains in a typed place
  or immutable typed static throughout NIR.

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
    Absolute(AddressValue),
    Deref {
        addr: NirValue,
    },
    Field {
        base: Box<NirPlace>,
        offset: ByteOffset,
    },
    Index {
        base_addr: NirValue,
        index: NirValue,
        elem_size: ByteSize,
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
    VolatileLoad {
        dest: TempId,
        place: NirPlace,
    },
    Store {
        place: NirPlace,
        src: NirValue,
    },
    VolatileStore {
        place: NirPlace,
        src: NirValue,
    },
    CopyBytes {
        destination: NirPlace,
        source: NirPlace,
        size: ByteSize,
        destination_volatile: bool,
        source_volatile: bool,
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
        kind: Integer | Pointer | IntegerToPointer | PointerToInteger,
    },
    PointerOffset {
        dest: TempId,
        base: NirValue,
        offset: NirValue,
        subtract: bool,
        ty: NirType,
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
    Real(NirRealOp),
}
```

SemIR decides whether a source lvalue is volatile. NIR records that decision
on the executable access, after names and aliases have been resolved, so MIR
does not need to recover language meaning from SemIR. A volatile load or store
executes exactly once and is a conservative memory-ordering barrier. It may
use the same target instruction as an ordinary access; the distinction limits
legal transformations rather than prescribing an opcode.

Whole-record assignment lowers to `CopyBytes`, not to a scalar load/store or a
source-shaped field walk. Both operands are typed addressable record places and
their known storage widths must equal the non-zero copy extent. The operation
copies the source value as it existed before any destination byte is written,
so overlapping and self copies are well-defined. Its two volatile flags retain
the independently resolved source-read and destination-write facts. Ordinary
`Load`, `VolatileLoad`, `Store`, and `VolatileStore` reject record types; this
keeps aggregates out of the byte/word scalar lane and gives MIR6502 one explicit
aggregate-copy contract to lower.

Native entry initialization also uses `CopyBytes` to copy an immutable static
template into an automatic array, record, or descriptor-backing byte range. In
that form the verifier proves the non-zero extent against the referenced
storage object's final layout and the static template extent; it does not infer
an aggregate size from an element type. Address fragments targeting automatic
storage are removed from the template and materialized afterward with
`AddrOf` and typed stores, so load-time data never captures another
invocation's address.

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
- Ordinary `Compare` operands have one common scalar machine type. SemIR
  promotes both source operands before NIR lowering. An operator retains its
  own semantic result type, and any assignment or comparison conversion is an
  explicit outer `Cast`; the NIR verifier checks operand width and signedness
  against `operand_ty`.
- Integer `Mul` always has an `I16` result, and integer `Neg` always has an
  `I16` result. Byte operands are explicitly extended before negation. The
  verifier rejects the former byte-result shapes so MIR6502 never has to
  reconstruct the cartridge typing rule.
- Constant `U8` addition and subtraction remain `U8` only when their 16-bit
  result fits in one byte. Overflow and subtraction underflow produce `I16`;
  an explicit cast back to `U8` records intentional truncation. Dynamic byte
  addition and subtraction remain byte operations.
- Expensive operations such as word multiply/divide may remain semantic NIR ops
  and lower to runtime helpers in MIR6502.
- NIR should not encode final 6502 addressing modes.

### Address-based native REAL

Native `REAL` does not use ordinary `Load`, `Store`, `Unary`, `Binary`, `Cast`,
or six-byte temps. Its executable shape is explicitly destination-passing:

```rust
pub enum NirRealOp {
    Copy {
        destination: NirPlace,
        source: NirRealSource,
    },
    Unary {
        operation: NirUnaryOp,
        destination: NirPlace,
        operand: NirRealSource,
    },
    Binary {
        operation: NirBinaryOp,
        destination: NirPlace,
        left: NirRealSource,
        right: NirRealSource,
    },
    Compare {
        predicate: NirCompareOp,
        result: TempId,
        result_type: NirType,
        left: NirRealSource,
        right: NirRealSource,
    },
    IntegerToReal {
        destination: NirPlace,
        source: NirValue,
        source_type: NirType,
    },
    RealToInteger {
        result: TempId,
        result_type: NirType,
        source: NirPlace,
    },
}

pub enum NirRealSource {
    Place(NirPlace),
    Static {
        id: StaticId,
        name: String, // display metadata only
    },
}
```

Compiler-created six-byte evaluation locals carry
`NirLocalPurpose::RealTemporary`. Ordinary addressable locals carry
`NirLocalPurpose::Storage`. This structured purpose is the only fact target
lowering may use to distinguish private REAL staging storage; printable names
and declaration-kind strings never affect executable lowering.

The lowerer materializes mutable or computed expression children left-to-right
into compiler-owned six-byte locals. Literal operands flow directly into unary,
binary, and comparison operations as immutable six-byte `rodata` sources, so
they do not require an otherwise redundant six-byte staging local and copy.
Statics are identified by stable IDs and deduplicated by canonical Atari packed
decimal bytes. The static name is diagnostic/printer metadata, not executable
identity.

The verifier guarantees:

- every real place operand and destination is typed six-byte `Real` storage;
- every real static operand names immutable six-byte `rodata`;
- every `RealTemporary` local is ordinary, uninitialized, scalar six-byte
  `Real` storage;
- routine temps and block parameters never carry `Real`;
- scalar operations, scalar calls, and scalar returns never carry `Real`;
- real comparisons alone define an ordinary canonical Boolean temp;
- integer-to-real conversion sources are ordinary typed 8- or 16-bit integers.
- real-to-integer conversion results are ordinary typed 8- or 16-bit integers;
  the six-byte source remains an address-based place.
- indexed REAL places carry a six-byte REAL element type and an element stride
  of six; REAL field places carry the same six-byte field type metadata.

Until a narrower proof exists, optimizers treat real operations as memory/call
ordering barriers. They may rewrite scalar temps used to address a real place,
but they do not fold, eliminate, reorder, or scalarize the real operation.
Selection of Atari FR0/FR1 workspaces and FPP entry points belongs exclusively
to MIR6502.

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
- Comparison-only `AND` and `OR` trees lower recursively to explicit CFG
  blocks. Each right operand is emitted only in the predecessor selected by
  its left operand, preserving left-to-right evaluation and conditional calls.
- Numeric bitwise trees, including conditions such as `flags & mask`, remain
  value-producing operations and are tested against zero. They are not
  reinterpreted as logical control flow from operator spelling alone.
- `IF`, `WHILE`, and `DO`/`UNTIL` share this condition lowering contract.
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
        symbol: RuntimeSymbolId,
        name: String,
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
    pub convention: NirCallConvention,
}
```

Rules:

- Indirect callees use typed values, not expression summary strings.
- Call argument count and types are verified against the signature.
- Call result temps are verified against the signature result type.
- Calling-convention class is structured and participates in signature
  identity and indirect-call verification; physical ABI placement belongs to
  the selected MIR backend.
- Runtime calls use a stable `RuntimeSymbolId`; the readable name is debug/link
  metadata and the selected runtime target comes from the verified program
  binding table.

Runtime declarations and classic helper overrides are program metadata:

```rust
pub struct NirRuntimeBinding {
    pub symbol: RuntimeSymbolId,
    pub name: String,
    pub target: Option<NirRuntimeTarget>,
}

pub enum NirRuntimeTarget {
    Absolute(AddressValue),
    Routine(RoutineId),
}
```

They must not appear as executable operations. An unbound target names a
service that the selected runtime or linker must resolve below NIR.

## Effects

Effects must be conservative enough to protect optimization around memory,
hardware, runtime calls, OS calls, and machine blocks.

```rust
pub struct NirEffects {
    pub memory_reads: NirMemoryEffect,
    pub memory_writes: NirMemoryEffect,
    pub may_call_external: bool,
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
- Runtime and platform/environment calls should be conservative by default.
- The external-call flag does not name a particular operating system.
- Absolute memory and hardware-register interactions must not be optimized away
  unless facts prove it is safe.
- Machine blocks are opaque by default.

## Static Data

Recommended static data shape:

```rust
pub struct NirStaticData {
    pub id: SymbolId,
    pub name: String,
    pub ty: NirType,
    pub image: NirDataImage,
    pub alignment: ByteSize,
    pub section: NirStaticSection,
    pub mutable: bool,
    pub display: String,
}

pub struct NirDataImage {
    // Explicit source bytes and zero placeholders only.
    pub bytes: Vec<u8>,
    pub fragments: Vec<NirDataFragment>,
}

pub enum NirDataFragment {
    Integer {
        offset: ByteOffset,
        width: ByteSize,
        value: u64,
    },
    Address {
        offset: ByteOffset,
        encoding: Pointer(AddressSpaceId, ByteSize)
                | TargetByte(TargetId, u8),
        target: Storage(NirStorageId)
              | Routine(RoutineId)
              | Absolute(AddressValue),
        addend: i64,
        span: SourceSpan,
    },
}
```

Rules:

- `image.bytes` contains authoritative explicit bytes and zero placeholders at
  logical-fragment positions. A backend serializes integer fragments with the
  selected endianness and lowers address fragments to its relocation model.
- Every initialized storage object has one exact declared extent. Its literal
  bytes plus explicit zero-fill equal that extent; a present initializer may
  never disappear into fallback zero storage.
- Aggregate layout is already resolved before NIR. Record and record-array
  images contain final byte offsets and widths, with no source initializer
  strings, field names, or SemIR lookup required by MIR6502.
- Address fragments use stable storage or routine identity and do not assign
  final addresses. Full pointer fragments carry their address space and
  target-selected width. Explicit address-byte selectors are target-tagged and
  rejected under a different target rather than masquerading as generic
  low/high relocations.
- A storage address fragment names the source-level object's data address. For an
  array this is its element backing, not an implementation descriptor cell.
- Load-time images may address routine-static or external storage, but may not
  contain a relocation to an invocation-relative automatic object. Native
  per-entry address construction belongs in executable initialization NIR.
- Fragment ranges must fit within the image and must not overlap.
- Fragment placeholder bytes must be zero, and total image extents must fit
  the NIR `ByteSize` storage model. Global, descriptor-backing, and local-backing image
  extents are verifier-checked against their storage descriptors.
- Image-end materialization is a generic link value in NIR. The Atari backend
  maps it to the historical program-end word; that load-file convention is not
  an NIR operation or relocation kind.
- `display` is for diagnostics and fixtures only.
- `StaticAddr(id)` must reference an existing static data entry.
- String representation policy should be documented at this boundary.

## Foreign Code And Machine Blocks

Executable foreign code has one target-tagged envelope:

```rust
pub struct NirForeignCode {
    pub target: TargetId,
    pub kind: LegacyMachineBlock | InlineAssembly,
    pub payload: Structured(Vec<NirMachineItem>)
               | Bytes { bytes: Vec<u8>, relocations: Vec<NirForeignRelocation> },
    pub source: String,
    pub span: SourceSpan,
}

pub struct NirForeignRelocation {
    pub offset: ByteOffset,
    pub encoding: Address(ByteSize)
                  | Unsigned(ByteSize)
                  | TargetByte(TargetId, u8),
    pub target: NirForeignCodeTarget,
    pub addend: i32,
    pub required_address_bits: Option<u8>,
    pub symbol_use: ForeignSymbolUse,
    pub span: SourceSpan,
}
```

Rules:

- A payload target must equal the selected NIR target. Legacy Action! machine
  blocks and current inline assembly are tagged `Atari6502`; 65816 and 68k
  compilation rejects them at the retained source span.
- Machine blocks must either carry enough structured payload for the selected
  backend to preserve them or produce a precise unsupported diagnostic.
- Raw parser items do not enter executable NIR; lowering replaces the whole
  machine block with an explicit `Unsupported` operation.
- Formatted effect strings are not optimizer-grade effects.
- Default effects should be opaque and conservative.

Source-level inline assembly uses a stricter verifier-clean payload than
legacy Action! machine blocks. The assembler has already encoded target code
before NIR, so NIR carries generic bytes, stable relocations, and
target-independent effects without carrying 6502 mnemonics, addressing modes,
registers, or flags. Target byte selectors are explicitly tagged rather than
represented as generic low/high meanings.

`Storage` denotes the compiler-managed storage object whose bounds the verifier
checks. A fixed numeric array whose Action-compatible representation includes a
pointer or descriptor cell instead uses `Absolute(declared_backing_address)`:
the machine operand denotes the array's declared backing, not its descriptor.
Its addend is preserved on the relocation, and its memory effect is an absolute
range at the resolved backing address plus that addend. The descriptor remains
a separate, correctly sized storage object. Signed addends are resolved without
clamping, and absolute underflow or overflow fails verification.

The verifier checks the target tag, relocation bounds and overlap, stable target IDs,
inline-offset bounds, and address-size constraints. Optimizers consume the
structured storage/memory effects; they must not decode the debug `source`
string. The selected backend may decode the byte payload for target-specific
register, flag, stack, and internal-control analysis because those facts belong
below the NIR boundary. Conversion from the integrated 6502 assembler's types
belongs to the semantic or MIR6502 adapter boundary; verifier-clean `src/nir`
has no assembler dependency.

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
- fixed-point sparse value propagation and CFG cleanup after representation
  changes expose new constants;
- folding of block parameters whose executable incoming values are identical;
- dominance-safe GVN for pure typed computations when reuse does not lengthen
  the canonical temporary's live range.

Promotion does not make a target allocation decision. NIR removes direct
source-home traffic and represents merged values with block parameters and edge
arguments. MIR owns the transient home, register, and spill strategy. The
initial automatic policy is deliberately pressure guarded: it promotes hot
ordinary byte locals with small definition sets, while initialized,
address-taken, aliased, absolute, machine-visible, wider, parameter, and colder
homes remain in storage form until target home coloring can carry them without
regressing output.

After promotion, backward storage liveness may remove a direct private-local
store only when no later load, structured effect read, machine barrier, or exit
persistence rule observes the value. A source local declaration may then be
removed only when it has no remaining access, address, initializer, alias,
effect-region reference, or machine visibility. Parameter homes remain ABI
owned, and globals remain observable storage; neither is erased by this pass.

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

## Red Lines

Do not consider NIR complete while any of these are true:

- optimizer passes run on legacy/stringly NIR shapes;
- MIR6502 consults SemIR to recover missing NIR facts;
- executable field/index forms preserve source syntax instead of semantic facts;
- calls lack signatures or conservative effects;
- machine blocks lack payload/effect handling or an explicit unsupported barrier;
- cross-block temp use is not verified;
- branch conditions are not typed or explicitly tested;
- verifier-clean IR can contain unknown/open boundaries.
