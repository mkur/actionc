# MIR65816 lowering contract

This describes the arithmetic and static-storage facts retained by the 65816
backend before instruction selection. Both `wdc-65816-native` and
`wdc-65816-small` use this contract. Their pointer widths remain three and two
bytes respectively; both have a 24-bit architectural address space.

SemIR owns source meaning. Verified NIR supplies typed operations, storage
identities, backing relationships and initialization. MIR65816 consumes those
facts without consulting SemIR, source expressions or display names. The
native emitter consumes this contract; small-model lowering remains separate.

## Arithmetic

- `Mir65816Op::Binary.signed` comes from the operation's NIR integer type at
  every width, including `LONGINT`. It is not inferred from a particular type
  spelling or a sixteen-bit width.
- `Mir65816Op::Compare.signed` comes from `operand_ty`, alongside the operand
  width and comparison operator. The boolean result type does not determine
  signedness. Pointer and address comparisons remain unsigned.
- These are instruction-selection facts. Preserving a division operation does
  not establish an executable division helper.
- Casts retain `from_signed` independently of widths and conversion kind, so
  signed widening does not need to recover the source type after lowering.

- `PointerOffset.offset_signed` comes from the offset's NIR integer type.
  Native NIR accepts integer displacements through four bytes; classic Atari
  retains its two-byte limit. Address/pointer values are never displacements.
  Emission sign-extends narrow signed offsets and uses modular pointer-width
  arithmetic, without consulting a source spelling.

## Control flow and entries

Routines retain their signature identity, entry/placement facts and typed
temporary table. Blocks retain parameter definitions; edges retain argument
values as well as target block IDs. The MIR verifier checks definitions against
the temporary table, unique identities and edge arity/width agreement. Emission
performs parallel edge copies using invocation storage. Lowering must not drop
edge values introduced by native loop promotion or infer an entry from its name.

## Storage and initialization

`Mir65816DataId` distinguishes global storage, compiler static templates and
array backing storage. A descriptor and its elements have different identities
even when owned by the same global symbol. `name` is display metadata only.

Each data item retains:

- placement: allocate storage, use an absolute address, or alias another global
  at a byte offset;
- complete size, initialized bytes and a separate trailing zero-fill extent;
- alignment, mutability, type/array metadata and the optional NIR section hint;
- relocations identifying their targets independently of names.

Ordinary globals without explicit initializers retain their zero-filled storage.
Absolute and alias declarations do not allocate or implicitly clear memory.
Large zero-fill arrays do not require a matching host byte buffer.

Initialized arrays retain both the descriptor and backing item, including any
partially initialized tail. The descriptor contains a data-pointer relocation
to its backing and, when present, the Action! two-byte size word in little-endian
order. Routine-address descriptors retain the code-pointer relocation and size
word. Image-end initializers preserve their declared relocation width and the
remaining zero-fill bytes in the object.

Scalar alignment follows the selected native target layout. Descriptor cells
use pointer alignment; their elements use the declared element type's alignment.
NIR supplies no global record field-alignment table, so records use conservative
word alignment. Compiler static templates retain their explicit NIR alignment.
This does not change record field offsets or array strides.

Native automatic locals remain invocation objects in routine frame plans;
immutable initialization templates remain separate static data. No global
allocation is invented for an automatic local. Unsized initialized native BYTE
and string arrays have separate descriptor/backing objects, just like wider
elements. Their element template initializes backing memory, never the pointer
descriptor. Descriptor addresses are constructed for the current invocation.

`Copy` retains both resolved volatility flags from `NirOp::CopyBytes` as well as
its extent and overlap semantics. The initial emitter handles ordinary copies
and rejects volatile aggregate copies explicitly; it cannot silently discard
the flags and emit ordinary memory movement.

## Relocations

Every relocation preserves its byte offset, write width, address space, typed
target and signed addend. Routine targets retain `RoutineId`.

`byte_index: None` writes the complete value at the declared width in target
byte order. `Some(0)`, `Some(1)` and `Some(2)` select the numeric low, high and
bank byte respectively, **after adding the addend**. Selection occupies one
output byte. The original target tag is checked by NIR verification against
the selected program target; wrong-target and out-of-range selectors fail
before MIR lowering. Absolute targets retain their data/code address space.

In static initializer address constants, a pointer-backed array's address names
its initial element storage. The relocation therefore targets its backing item
or fixed element address. Executable accesses to the descriptor cell retain
the global identity. Addends survive this target resolution.

The eventual linker must apply relocations and diagnose final address/range
overflow. Lowering does not truncate a bank, apply a byte selector early, or
resolve an object by its printed name.

## Checks and remaining boundary

NIR verification continues to reject invalid fragment extents, placeholder
bytes, pointer widths, address spaces and target-byte selectors. Data lowering
also diagnoses inconsistent descriptor layouts, initialization/storage extents,
duplicate data identities and relocation targets without storage.

[The contract tests](../tests/mir65816_contract.rs) exercise both native models
and raw/optimized NIR, including the signedness regressions, zero-fill and
partial initialization, aliases, descriptor/backing identities, section and
mutability facts, size words, image-end values and byte-selected relocations.
Selectors and descriptor corner cases without a native source spelling are
constructed as NIR fixtures and passed through the real verifier/backend entry.

The [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md) has generated constants,
verified call/frame plans and a [native scalar emitter](MIR65816_EMISSION_CONTRACT.md).
The emitter allocates invocation slots, checks concrete accesses and links
freestanding images. Indirect calls, contexts and G1–G6 for the advertised
subset are covered by [initial Exec acceptance](MIR65816_EXEC_ACCEPTANCE.md).
The [implementation plan](MIR65816_IMPLEMENTATION_PLAN.md) records the slices.
