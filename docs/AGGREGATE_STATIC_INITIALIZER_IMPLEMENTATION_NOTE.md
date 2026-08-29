# Aggregate Static Initializers Implementation Note

Status: Slices 1-4 complete; legacy width cleanup is next.

Snapshot date: 2026-08-29.

## Goal

Generalize static initializer lists so their meaning comes from the declared
semantic layout rather than from one uniform array-element width. The immediate
use case is a table of effect parameters:

```action
TYPE EffectParams=[
  BYTE phaseX
  CARD stepX
  BYTE phaseY
]

EffectParams ARRAY effects(2)=[
  10 $0123 20
  30 $0456 40
]
```

This must be ordinary support for records and arrays of records. It must not be
a special case for `EffectParams`, twelve-byte records, or the unlimited-bobs
sample.

The implementation should cover:

- direct record variables;
- sized and initializer-sized arrays of records;
- global and routine-local storage;
- arbitrary packed record sizes;
- mixed scalar leaf widths;
- recursively nested record layouts already accepted by semantic analysis;
- literal data and relocatable address elements where the destination leaf type
  permits them.

Existing `BYTE`, `CHAR`, `CARD`, `INT`, and native `REAL` scalar-array
initializers must retain their current layout and emitted bytes.

## Current Limitation

The parser already represents an initializer list as structured elements, but
the rest of the pipeline interprets every element using the type and width of
the enclosing declaration:

- `src/semantic.rs::validate_initializer_elements` computes one
  `element_width` from the declared array element type and applies it to every
  initializer element;
- `src/nir/lowerer.rs::initializer_data_image` accepts only one- and two-byte
  elements, plus the special six-byte native `REAL` case;
- `src/codegen/data.rs::structured_array_initializer_storage` has the same
  `1 | 2 | 6` width restriction;
- global and local storage planning use initialized byte length divided by the
  enclosing element width to infer array length and backing size.

For a record such as `[BYTE, CARD, BYTE]`, the enclosing element width is four
bytes, but each initializer token belongs to a different scalar leaf with a
width of one or two bytes. Treating the record width as the token width cannot
describe this layout. Adding another allowed record size would merely move the
same error.

The semantic layer already has the information required to solve this:

- stable record identities;
- ordered fields with stable `FieldId` values;
- packed field offsets;
- field `ValueType` values;
- total record sizes;
- array element types and declared lengths.

Those facts should become the sole source of aggregate initializer meaning.

## Source-Language Contract

### Flat scalar-leaf stream

The first implementation keeps the existing initializer-list grammar. It does
not require nested brackets or introduce a second record-literal syntax.

An initializer list for an aggregate is consumed as a stream of scalar leaf
values:

1. Record fields are visited in declaration order.
2. A scalar field consumes one initializer element.
3. A record field is visited recursively.
4. An array of records repeats the record shape for each element.
5. Each value is written at the byte offset of its destination scalar leaf.

For example:

```action
TYPE Inner=[BYTE flag CARD address]
TYPE Outer=[BYTE tag Inner value BYTE tail]

Outer ARRAY table(2)=[
  1 2 $3456 3
  4 5 $789A 6
]
```

has the conceptual mapping:

```text
table(0).tag           <- 1
table(0).value.flag    <- 2
table(0).value.address <- $3456
table(0).tail          <- 3
table(1).tag           <- 4
table(1).value.flag    <- 5
table(1).value.address <- $789A
table(1).tail          <- 6
```

This is brace elision over a recursively defined aggregate shape. Nested list
grouping and designated field initializers may be useful later, but they are a
separate syntax and compatibility decision.

### Partial and inferred initialization

Omitted trailing storage is zero-filled, consistent with current initialized
array behavior. If an initializer-sized array ends partway through a record,
the backing includes the complete containing record and zero-fills its
remaining leaves.

Before implementation, characterization tests must lock down the current rule
for an initializer containing more data than a declared array size. Aggregate
support must initially preserve that rule. Tightening excess-initializer
handling is a separate source-language change and must not be hidden inside
this migration.

### Leaf legality

Semantic validation is performed against the destination leaf, not against the
enclosing record:

- integer, character, boolean, and `NIL` literals keep their current conversion
  rules;
- a native `REAL` literal requires a native `REAL` destination leaf;
- `<target` and `>target` require a one-byte destination leaf;
- `@target` requires a two-byte destination leaf;
- address targets must resolve to addressable storage or an allowed routine;
- runtime expressions are not static initializer elements;
- unsupported elements produce a diagnostic and must never become implicit
  zeroes.

Diagnostics should identify the declaration and, when available, the aggregate
path such as `effects(1).stepX`.

## Boundary Ownership

- The parser owns list syntax, element spelling, and source spans. No parser
  change is required for the initial flat-list implementation.
- SemIR owns aggregate traversal, field order, destination type, byte offset,
  initializer compatibility, addressability, and inferred aggregate extent.
- NIR owns the normalized initialized-data image: bytes, zero-fill extent, and
  stable-ID relocations at explicit offsets.
- MIR6502 owns target storage identities and layout strategy, but never walks a
  record or consults SemIR to reinterpret an initializer.
- Classic code generation consumes the same resolved semantic plan through its
  projection/fact bridge. It must not implement an independent record-flattening
  language rule.
- Emission owns final byte writing, label binding, and relocation patching.

Initializer plans are declaration metadata, not executable operations. They
must not appear as metadata operations inside NIR blocks.

## Semantic Initializer Model

Add a layout-resolved, non-executable initializer representation to SemIR. The
exact Rust names may change, but the contract should resemble:

```text
SemStaticInitializer {
    initialized_extent: u16,
    writes: Vec<SemStaticInitializerWrite>,
}

SemStaticInitializerWrite {
    offset: u16,
    destination: ValueType,
    width: u16,
    value: SemStaticInitializerValue,
    span: Span,
    display_path: optional source metadata,
}

SemStaticInitializerValue {
    Literal(typed constant),
    Real(AtariReal),
    Address {
        selector,
        target: SemSymbolRef,
        addend,
    },
}
```

The plan records one write per scalar leaf value. It does not pre-expand a
record into one synthetic integer, and it does not retain field names as
executable identity. `display_path` is diagnostic/printing metadata only;
offsets, types, and stable identities are authoritative.

The semantic builder should take:

- the declaration's scalar, array, or record shape;
- `SemanticLayoutFacts` for record fields and array facts;
- the structured initializer elements;
- the declared array length, when any.

It should return either a complete plan or diagnostics. A present initializer
that cannot be planned must not fall back to zero-filled storage.

## NIR Contract

NIR already has the appropriate final data shape:

```text
NirDataImage {
    bytes,
    relocations,
}
```

NIR lowering should encode each semantic write at its explicit offset:

- scalar values become target-order bytes;
- native `REAL` values use their existing six-byte representation;
- relocation destinations are zero placeholders in `bytes` plus a
  `NirDataRelocation` at the write offset;
- holes before the initialized extent are explicitly zeroed;
- storage after the initialized extent remains the owning initializer's
  `zero_fill`.

After this conversion, MIR6502 and emission do not need to know whether the
bytes originated from a scalar, record, or array of records.

NIR verification must ensure:

- write-derived image length and declared backing extent agree;
- every relocation lies completely within the data image;
- relocation ranges do not overlap;
- relocation width agrees with its kind;
- every relocation target uses a valid stable storage or routine identity;
- image length plus zero-fill does not overflow the compiler's storage range;
- no raw initializer text or unresolved semantic element survives lowering.

The existing relocation verifier guarantees remain in force.

## Classic Backend Bridge

The classic backend currently reconstructs record layout from AST declarations
and builds initializer bytes in `structured_array_initializer_storage`. That
function must stop being the authority for aggregate meaning.

Thread resolved initializer facts through the existing SemIR-to-classic
projection:

1. SemIR supplies the ordered writes, offsets, widths, and resolved targets.
2. Projection converts stable semantic targets to the classic label/storage
   identities already used by `StorageInit`.
3. Classic storage planning uses the plan's initialized extent for inline or
   descriptor-backed allocation.
4. Classic emission translates writes to `StorageInit::Byte` and relocation
   entries without walking record fields.

Compatibility compilation that still selects the AST code-generation path
must receive the same fact bundle from the semantic model already constructed
by `compile_classic`. Public AST-only codegen entry points should use an adapter
that builds the semantic facts or report unsupported aggregate data; they must
not grow a second initializer type system.

The existing scalar initializer helpers may remain as compatibility adapters
during migration. Once all initialized-data paths consume the shared plan, the
`1 | 2 | 6` width gates and raw numeric fallback should be removed or narrowed
so unsupported aggregates cannot silently reappear.

## Implementation Slices

### Slice 1: characterize and specify existing behavior

Status: complete.

- Add focused tests for sized and initializer-sized scalar arrays.
- Lock down partial initialization, zero-fill, excess data, negative values,
  native `REAL`, and relocation behavior.
- Add semantic tests for record field traversal order and nested packed offsets.
- Document any observed legacy behavior that aggregate initialization must
  preserve.

Characterization confirms that a sized initializer shorter than its declared
storage is zero-filled, while an initializer longer than the declared byte
array currently extends the allocated storage. The aggregate implementation
must preserve both behaviors until an explicit source-language change says
otherwise.

This slice changes no emitted data.

### Slice 2: add the semantic aggregate plan

Status: complete.

- Introduce the SemIR initializer plan and scalar-leaf write model.
- Build aggregate shapes from `SemanticLayoutFacts`, not from byte-size
  allow-lists.
- Resolve initializer elements against successive scalar leaf destinations.
- Compute initialized extent and complete-record rounding for inferred arrays.
- Move width/type/address diagnostics to the destination leaf.
- Print the plan readably in SemIR fixtures while keeping stable IDs and offsets
  authoritative.

Keep the old initializer expression temporarily as source/debug metadata only
if required for migration. New lowering must use the typed plan.

### Slice 3: complete one vertical backend slice

Status: complete.

Support a global array of mixed-width records through both output paths:

```action
TYPE Pair=[BYTE tag CARD word]
Pair ARRAY pairs(2)=[1 $2345 2 $6789]
```

- Lower the plan to `NirDataImage` and MIR6502 output.
- Project the same plan to classic `StorageInit` output.
- Verify exact bytes, descriptor/backing shape, and field reads from the second
  element.
- Cover at least two non-special record sizes, including three bytes and a
  larger size such as twelve bytes.

The slice is complete only when compatibility/classic, modern/classic, and
MIR6502 agree on data bytes and runtime behavior.

### Slice 4: complete aggregate storage coverage

Status: complete.

- Direct initialized record variables.
- Routine-local initialized records and arrays of records.
- Sized and initializer-sized record arrays.
- Partial final records with zero-filled trailing leaves.
- Nested record fields supported by semantic layout.
- Relocations in one- and two-byte record leaves, including self and forward
  targets where already legal.
- Qualified module targets and readable projected names.

Add explicit diagnostics for invalid leaf types, invalid address widths,
unsupported elements, and any unrepresentable storage extent.

### Slice 5: remove legacy width assumptions

- Replace `structured_array_initializer_storage` as an initializer authority.
- Remove aggregate dependence on `numeric_array_initializer_storage`.
- Remove the `1 | 2 | 6` enclosing-element gates from NIR lowering.
- Tighten verification or assertions so a present initializer cannot be
  discarded and replaced with zero-fill.
- Confirm MIR6502 never consults SemIR or record layouts for initialized data.

Existing scalar helpers may remain only as wrappers around the general plan or
as narrowly documented support for non-list scalar aliases.

### Slice 6: documentation and motivating sample

- Add the aggregate initializer contract to `docs/SEMANTIC_INVARIANTS.md`.
- Add record and record-array initialized backing shapes to
  `docs/ACTION_STORAGE_LAYOUT.md`.
- Update `docs/NIR_TARGET_SHAPE.md` with the verifier-clean initialized-data
  invariant.
- Cross-reference `docs/RELOCATABLE_STATIC_INITIALIZER_IMPLEMENTATION_PLAN.md`;
  aggregate relocations reuse that mechanism rather than replacing it.
- Convert the unlimited-bobs parametrization to an `EffectParams ARRAY` only
  after compiler support and regression coverage are complete.

## Test Matrix

The implementation needs focused coverage at each boundary:

| Area | Required cases |
| --- | --- |
| Parser | Existing flat lists remain byte-for-byte represented; malformed/nested lists remain diagnosed according to the existing grammar |
| Semantic | Mixed widths, declaration order, nested layout, partial final record, inferred extent, invalid literal type, invalid address width |
| SemIR | Explicit write offsets/types, stable relocation targets, readable field paths as metadata only |
| NIR | Exact image bytes, relocation offsets, zero-fill, no raw source or field-name dependency |
| Verifier | Out-of-bounds/overlapping relocations, invalid targets, extent overflow, malformed initialized storage |
| Classic | Global/local, inline/descriptor-backed, sized/inferred, relocation parity |
| MIR6502 | Same cases and exact data-image parity with classic |
| Runtime | Read every field from at least two initialized record elements and verify expected values |
| Regression | Existing BYTE/CARD/INT/REAL initializer fixtures emit unchanged data |

A record with all one-byte fields is not sufficient coverage. At minimum, the
runtime and byte-layout fixtures must contain mixed one- and two-byte leaves so
they detect the original uniform-width bug.

## Required Validation

After each vertical slice, run the focused semantic, codegen, MIR6502, and
runtime tests added by that slice. Before considering the migration complete,
run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

If NIR fixtures change, classify each change as an intentional initialized-data
contract change, a printer-only change, or a bug fix.

## Non-Goals

- No special case for the unlimited-bobs sample or a particular record size.
- No designated field initializer syntax.
- No new nested-bracket aggregate syntax in the first implementation.
- No runtime record-constructor expressions.
- No record assignment or record-copy semantics; table consumers may read
  fields directly.
- No arrays or pointer fields inside records unless those declarations become
  independently supported by the semantic language rules.
- No MIR6502 recovery of record facts from SemIR.
- No change to array descriptor ABI or existing initialized-array placement.
- No unrelated optimizer work.

## Completion Criteria

The work is complete when:

- arbitrary supported record layouts can be initialized without a byte-width
  allow-list;
- direct records and global/local arrays of records compile in all supported
  backend modes;
- scalar leaf types and relocation widths are checked in SemIR;
- verifier-clean NIR contains only explicit data bytes, zero-fill, offsets, and
  stable-ID relocations;
- classic and MIR6502 output agree on initialized backing bytes and runtime
  field values;
- unsupported aggregate initializers are diagnostics, never implicit zeroes;
- existing scalar array initializer output remains unchanged;
- the unlimited-bobs effect table can be expressed as ordinary record-array
  data with no startup assignment workaround.
