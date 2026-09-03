# NIR Target-Independence Implementation Plan

Snapshot date: 2026-09-03.

Status: complete. Slices 0 through 10 were implemented and verified on
2026-09-03.

This plan prepares verifier-clean NIR to feed independent MOS 6502, WDC
65816, and Motorola 68k backends. It also revises the initial record-layout
proposal: packed records remain part of the classic Action! compatibility ABI,
but native targets may use target-natural record alignment. Portable source
should use compile-time layout queries instead of assuming byte offsets or
array strides.

The intended pipeline becomes:

```text
Action source -> AST -> semantic model -> SemIR
              -> target-parameterized NIR
              -> MIR6502 | MIR65816 | MIR68K
              -> target emission and linking
```

Target-independent does not mean target-free. NIR uses one common operation,
type, CFG, storage, and effect model, while each `NirProgram` carries the
resolved data-layout contract under which its storage facts were produced.
No backend may consult SemIR to reconstruct those facts.

## Goals

- Preserve fixed Action! scalar meaning: `BYTE` and `CHAR` are 8-bit, while
  `CARD` and `INT` are 16-bit.
- Separate pointer and address width from `CARD` width.
- Make data-pointer and code-pointer representation target-configurable.
- Represent storage sizes, offsets, absolute addresses, address spaces, and
  relocations without a 16-bit machine assumption.
- Keep scalar computation and typed memory operations independent of byte
  order, alignment, register sets, and instruction encodings.
- Allow SemIR to resolve target-dependent aggregate layout while preserving
  source-level record and array meaning.
- Keep NIR self-contained by retaining the final field offsets, element
  strides, storage extents, pointer types, signatures, and effects required by
  every MIR backend.
- Preserve byte-exact Atari 6502 output until a slice explicitly declares an
  intentional output change.
- Prove the boundary with both candidate targets. 68k exposes endian,
  alignment, and 32-bit-pointer assumptions; 65816 exposes 24-bit addresses,
  banks, direct-page selection, and near/far call choices.
- Introduce compile-time layout queries before native record layout can differ
  from classic packed layout.

## Non-goals

- Do not turn NIR into a source-language-independent IR. It remains the final
  Action!-aware normalized representation.
- Do not widen `CARD` or `INT` to hold native pointers.
- Do not put 65816 bank registers, M/X state, direct-page placement, or
  near/far instruction choices into NIR.
- Do not put 68k D/A registers, condition-code strategy, effective-address
  modes, or unaligned-load sequences into NIR.
- Do not generalize MIR6502 into a shared 65xx instruction IR. MIR65816 is a
  separate backend even if implementation utilities can be shared safely.
- Do not implement complete 65816 or 68k runtimes as part of the NIR cleanup.
- Do not silently change record layout for existing Atari or compatibility
  builds.
- Do not add `LET`, `CASE`, or unrelated language features during this work.

## Target Matrix

The initial target descriptions should express at least the following facts:

| Property | Atari 6502 | WDC 65816 native | Motorola 68000 |
| --- | ---: | ---: | ---: |
| Byte order | little | little | big |
| `BYTE` / `CHAR` | 8 bits | 8 bits | 8 bits |
| `CARD` / `INT` | 16 bits | 16 bits | 16 bits |
| Default data pointer | 16 bits | 24 bits | 32 bits |
| Default code pointer | 16 bits | 24 bits | 32 bits |
| Architectural address | 16 bits | 24 bits | 32 bits |
| Optional linkable-address limit | 16 bits | 24 bits | 24 bits for a 68000 platform profile |
| Fast low-memory form | zero page | relocatable direct page | none |
| Default compatibility record layout | packed | n/a | n/a |
| Default native record layout | n/a | target-natural | target-natural |

The 65816 target should also permit an explicit small memory model with 16-bit
data or code pointers. The NIR type model must not assume that all pointer
classes have the same width. The backend decides whether a particular access
can use a same-bank short form; that is not a NIR type change.

For 68k, pointer storage width and the platform's implemented physical-address
limit are separate facts. A 68000 platform may store a 32-bit pointer while
requiring the linker to place it in the 24-bit physical address range.

## Record and Aggregate Layout Policy

### Compatibility layout

Classic Action! records remain packed in declaration order with no implicit
padding. Existing Atari source can continue to overlay OS structures, hardware
registers, binary data, and fixed memory without silent offset changes.

### Native layout

A native ABI may insert target-required padding and select a natural record
alignment. Array stride is the padded `SIZEOF(element)`, and whole-record
assignment copies the complete `SIZEOF(record)` extent, including padding.

Native alignment is a selected ABI policy, not an optimizer transformation.
The compiler must never change it after semantic layout or infer that a layout
is unobservable.

### Explicit representation

The language should eventually provide an explicit `PACKED` record form for
hardware layouts, file formats, protocols, and deliberately portable byte
representations. Packed representation controls padding but not byte order.
Exact cross-endian formats should use byte fields or later explicit-endian
types rather than relying on native `CARD` representation.

Existing unannotated source must retain its current layout under the classic
Atari ABI. A future native-language edition or explicit native ABI selection
may make naturally aligned records the default without changing compatibility
builds.

### 68000 unaligned access

A packed record can place `CARD` or `INT` at an odd byte offset. An original
68000 cannot perform a native word access there. MIR68K must inspect the
resolved address alignment and use byte operations when alignment is not
proven. This is a target selection problem; NIR continues to express one typed
load or store.

## Compile-Time Layout Queries

Land layout queries before enabling a native ABI whose layout differs from the
classic packed ABI:

```text
SYS.SIZEOF(type-or-object)
SYS.ELEMENTS(sized-array)
SYS.ALIGNOF(type-or-object)
SYS.OFFSETOF(record-type, field)
```

`SYS` owns the canonical intrinsic identities. The compatibility prelude may
make `SIZEOF`, `ELEMENTS`, `ALIGNOF`, and `OFFSETOF` available as convenient
unqualified aliases of the same symbols. These spellings are not new keywords
or reserved identifiers: normal Action! lookup applies, so a visible local or
global declaration shadows an unqualified alias. The qualified `SYS` spelling
remains the explicit way to select the intrinsic when an alias is shadowed.

The parser must preserve ordinary qualified and unqualified call syntax rather
than deciding that a call is intrinsic from its spelling. Semantic resolution
first binds the callee through the normal symbol rules. Only a callee bound to
one of the `SYS` intrinsic IDs gives its arguments type/object/field layout
meaning; a shadowing user routine, function, array, or other callable subject
retains ordinary Action! argument or indexing semantics.

Required semantics:

- `SIZEOF` returns the storage extent selected by the current target ABI,
  including record tail padding and target-width pointer fields.
- `ELEMENTS` returns the declared element count of a statically sized array.
  An unsized or pointer-backed array without a compile-time element count is a
  diagnostic.
- `ALIGNOF` returns the ABI-required base alignment.
- `OFFSETOF` returns the resolved byte offset and is intended for assembly and
  external-layout integration rather than ordinary field access.
- Array storage and pointer arithmetic use `SIZEOF(element)` as their stride.
- Results are arbitrary-precision compile-time integers until checked against
  their use context. They must not be forced into `CARD` merely because the
  Atari target historically has 16-bit sizes.
- The queries are resolved by semantic/layout lowering. NIR receives ordinary
  constants, field offsets, strides, and extents; it does not gain executable
  `SizeOf`, `Elements`, `AlignOf`, or `OffsetOf` operations.
- No runtime entry point or runtime binding is generated for a layout query.
  The `SYS` declarations identify compiler intrinsics even though they share
  the namespace and lookup behavior of other predefined symbols.

A target-sized source `SIZE` type and an integer-capable `ADDRESS` type may be
added later. The initial NIR migration does not require their source syntax,
but it must not prevent them.

## Pointer and Integer Compatibility Policy

The current language permits `CARD` values and pointers to interoperate because
both are 16 bits on Atari. That equivalence cannot silently extend to 24- or
32-bit pointers.

- The classic Atari ABI retains existing pointer/`CARD` compatibility.
- A numeric address literal may initialize a native pointer when its value fits
  the destination address space.
- Null has a pointer representation independent of `CARD` width.
- General native pointer-to-integer and integer-to-pointer conversions must be
  explicit and checked.
- Until a target-sized `ADDRESS` source type exists, a native target should
  diagnose dynamic `CARD`/pointer conversions that would truncate or invent
  address bits.
- Pointer offset computation is distinct from ordinary `U16` arithmetic in
  NIR.

## Proposed NIR Data-Layout Contract

The exact Rust names may change, but the ownership should resemble:

```rust
pub struct NirTargetLayout {
    pub target: TargetId,
    pub endian: Endian,
    pub address_bits: u8,
    pub link_address_bits: u8,
    pub address_spaces: Vec<NirAddressSpace>,
    pub data_pointer: NirPointerLayout,
    pub code_pointer: NirPointerLayout,
    pub aggregate: NirAggregateLayout,
}

pub struct NirPointerLayout {
    pub address_space: AddressSpaceId,
    pub size: ByteSize,
    pub alignment: ByteSize,
}

pub struct ByteSize(pub u32);
pub struct ByteOffset(pub u32);
pub struct AddressValue(pub u64);
```

Scalar constants such as `ConstU16` remain 16-bit language values. Absolute
addresses use `AddressValue`, and every absolute address carries an address
space. Arithmetic on sizes, offsets, ranges, and addresses is checked rather
than saturating.

## Implementation Slices

Each slice must be independently useful, verifier-clean, and committed before
the next begins.

### Slice 0: contract and baselines

1. Update `NIR_TARGET_SHAPE.md` with the target-parameterized definition.
2. Separate CPU, platform, data layout, runtime ABI, and output format in the
   documented compiler boundary.
3. Record byte hashes and size baselines for representative Atari programs,
   including records, pointer fields, callable pointers, arrays, inline
   assembly, standalone runtime, and cartridge runtime.
4. Add the record-layout and pointer-conversion policies from this plan.

Suggested commit:

```text
docs: define portable NIR data-layout contract
```

### Slice 1: compile-time layout queries

Status: complete. The four compiler-owned `SYS` symbols use ordinary semantic
lookup, their short aliases remain shadowable, and successful queries are
cached as compiler constants before SemIR/NIR lowering. The initial evaluator
uses the Atari compatibility layout; it retains the computed result in a wide
compiler integer until conversion to the current `CARD` result type. Semantic
array facts now retain declared element counts, including through SemIR layout
facts. The NIR fixture proves that no query operation or runtime call survives.

1. Declare `SYS.SIZEOF`, `SYS.ELEMENTS`, `SYS.ALIGNOF`, and `SYS.OFFSETOF` as
   stable compile-time intrinsic symbols and expose shadowable unqualified
   compatibility-prelude aliases.
2. Keep their source syntax on the ordinary call/subject parsing path; do not
   reserve the four identifier spellings or recognize intrinsics by name in
   the parser.
3. Resolve the callee first, then interpret and resolve its type, object, and
   field operands in the semantic model only when the selected symbol is a
   layout intrinsic.
4. Evaluate the queries from the current Atari layout first.
5. Retain arbitrary-precision compile-time values until contextual conversion.
6. Diagnose unsized `ELEMENTS`, incomplete record layout, invalid fields, and
   results that do not fit their eventual storage or call context.
7. Add shadowing coverage for local/global routines and arrays, plus qualified
   `SYS` access from a scope where the unqualified alias is shadowed.
8. Confirm that NIR contains only the folded result and no runtime call or
   intrinsic operation.

Suggested commits:

```text
language: add sizeof and elements layout queries
language: add alignof and offsetof layout queries
```

### Slice 2: target-description plumbing

Status: complete. `target.rs` defines stable CPU, platform, ABI, target,
endianness, pointer-layout, and record-policy facts for Atari 6502, 65816
native, 65816 small, and Motorola 68000. CLI/API requests and source settings
select a target before semantic analysis; the complete registered layout is
carried by SemIR and NIR and checked by the NIR verifier. Candidate targets can
be inspected with `--emit-nir` or `--emit-optimized-nir`; machine-code requests
receive an explicit unavailable-backend diagnostic. The MIR6502 boundary also
rejects non-Atari NIR defensively.

1. Add stable CPU, platform, ABI, and target IDs.
2. Define the Atari 6502, 65816 native, 65816 small, and 68k target layouts.
3. Thread the selected target through compiler request resolution, semantic
   analysis, SemIR layout, NIR lowering, verification, runtime selection, and
   backend dispatch.
4. Store the resolved layout or a stable complete layout ID in `NirProgram`.
5. Initially allow non-6502 targets to reach verified NIR inspection while
   producing a precise backend-unavailable diagnostic for code generation.

Suggested commit:

```text
compiler: thread target data layout into NIR
```

### Slice 3: typed sizes, offsets, and addresses

Status: complete. NIR storage extents, alignments, field and image offsets,
element strides, copy extents, effect ranges, and relocations now use checked
`ByteSize`/`ByteOffset` values. Absolute storage, places, runtime addresses,
and relocation targets use address-space-qualified `AddressValue`. NIR retains
32-bit layout quantities independently of Action! `CARD`; the verifier checks
absolute extents against the selected target, while MIR6502 performs explicit
checked narrowing at its backend boundary.

1. Introduce `ByteSize`, `ByteOffset`, `AddressValue`, and `AddressSpaceId`.
2. Migrate NIR storage extents, field offsets, element strides, copy extents,
   data-image offsets, relocation offsets, and effect ranges away from raw
   `u16` fields.
3. Replace `NirPlaceKind::Absolute(u16)` and other absolute-address variants
   with address-space-qualified `AddressValue` forms.
4. Keep Action scalar literals as `u8`/`u16`; do not mechanically widen every
   integer in `src/nir`.
5. Make the verifier reject overflow against the selected target instead of
   truncating or saturating.

Suggested commits:

```text
nir: introduce typed storage sizes and offsets
nir: introduce target-width absolute addresses
```

### Slice 4: layout-driven pointer and callable types

Status: complete. Data pointers now carry pointee and address-space facts, and
callable types carry a stable structural `SignatureId` plus their code address
space. Their widths come from the selected target layout. Nulls and numeric
address constants are typed values, pointer offsets are distinct from integer
binary operations, and casts carry an explicit pointer/integer conversion
class. Native targets reject dynamic `CARD`/pointer conversion until a
target-sized source `ADDRESS` type exists; Atari retains its compatibility
rule. MIR6502 explicitly consumes these forms only after its Atari layout
guard.

1. Replace `Ptr16` with a pointer kind carrying pointee and address-space facts.
2. Give callable values a stable `SignatureId` and code-address-space fact.
3. Query data- and code-pointer widths from `NirTargetLayout`.
4. Add explicit null, address constant, pointer offset, pointer cast,
   integer-to-pointer, and pointer-to-integer forms or facts as required by
   verified lowering.
5. Prevent generic `U16` binary operations or casts from silently standing in
   for native pointer computation.
6. Keep physical call argument and result placement below NIR.

Suggested commit:

```text
nir: make pointer and callable types layout driven
```

### Slice 5: target-aware semantic aggregate layout

Status: complete. Semantic layout now resolves pointer/callable widths, natural
field alignment, record alignment and tail padding, and array element stride
from the selected target contract. Classic Atari records and descriptors remain
byte-for-byte packed. Pointer and callable record fields are legal storage
values, and target-sized initialized-array descriptors reach NIR without an
Atari four-byte assumption. The semantic and NIR layout-matrix tests cover all
four registered targets.

1. Make semantic width and alignment queries consume the selected layout.
2. Preserve stable record and field IDs and compute final offsets, alignment,
   size, and tail padding according to the selected ABI policy.
3. Compute array element stride from the complete element layout.
4. Derive array descriptor shapes from data-pointer width rather than assuming
   the Atari four-byte descriptor.
5. Keep classic Atari packed layout and descriptor bytes unchanged.
6. Carry all resolved layout facts into NIR so MIR consumers never consult
   SemIR.

Add a layout matrix fixture containing:

- `BYTE` followed by `CARD`;
- a data-pointer field;
- a callable-pointer field;
- a nested record;
- an array of that record;
- a trailing byte that exposes pointer width and tail padding.

Suggested commit:

```text
semantic: derive aggregate layout from the target ABI
```

### Slice 6: endian-neutral static initializers

Status: complete. NIR data images now retain raw source bytes separately from
typed integer and address fragments. Integer serialization happens when a
target backend projects the image, and address fragments distinguish data and
code pointer spaces and widths. Compatibility low/high-byte selectors are
explicitly tagged as Atari 6502 conventions. The Atari backend maps the
generic image-end link value and logical descriptor contents to the existing
MIR6502 representation, preserving emitted bytes. Tests project the same
logical integer as little-endian 65816 and big-endian 68k data.

1. Replace already-serialized typed values in `NirDataImage` with logical data
   fragments such as byte sequences, typed integers, data addresses, code
   addresses, and zero fill.
2. Preserve explicit byte arrays and encoded strings as byte sequences.
3. Serialize integer fragments using the selected target byte order only below
   NIR.
4. Replace fixed `Word16`, low-byte, and high-byte generic relocation
   assumptions with pointer-, code-address-, integer-, or explicitly
   target-tagged relocation meaning.
5. Move `ProgramEndWord`, Atari descriptor materialization, and other load-file
   conventions into the Atari ABI/backend layer.
6. Verify big-endian 68k and little-endian 65816 initializer projections from
   the same logical constants.

Suggested commit:

```text
nir: retain typed static data until target lowering
```

### Slice 7: generic address-space effects and runtime symbols

Status: complete. NIR effect regions now use address-space-qualified absolute
ranges; page-zero recognition is an Atari MIR concern. Calls and machine
payloads expose a platform-neutral external/environment effect. Runtime calls
use stable symbol IDs backed by verified program-level late bindings, and
classic helper `SET` directives populate that binding table without creating
executable metadata operations or a synthetic top-level routine.

1. Replace the generic `ZeroPage` effect region with address-space-qualified
   absolute ranges. The 6502 backend may recognize page zero; the 65816 backend
   decides whether an address is usable through its current direct page.
2. Replace `may_call_os` with a platform-neutral external/environment effect.
3. Replace runtime helper names and `Option<u16>` addresses with stable runtime
   symbol IDs and late target binding.
4. Move executable `RuntimeHelperOverride` metadata out of routine blocks into
   verified program/runtime binding facts.
5. Keep calls, unknown external code, volatile storage, and absolute memory
   conservative unless structured effects prove otherwise.

Suggested commit:

```text
nir: generalize memory effects and runtime bindings
```

### Slice 8: target-tagged machine payloads

Status: complete. Both legacy machine blocks and assembled inline code now use
one `NirForeignCode` container carrying a target ID, source text/span, payload,
relocations, and conservative effects. Relocation encodings describe generic
width/address or explicitly target-tagged byte-selection meaning; the 6502
adapter maps those facts to its assembler and machine-state model. The NIR
verifier rejects foreign code whose target differs from the selected layout,
and `src/nir` no longer imports the integrated 6502 assembler.

1. Introduce a generic foreign-code container with target ID, bytes,
   relocations, source metadata, and conservative effects.
2. Tag legacy Action! machine blocks and current inline assembly as 6502 code.
3. Keep 6502 low/high selectors, zero-page requirements, opcode analysis, and
   machine-state summaries inside the 6502 adapter or MIR6502.
4. Reject a mismatched machine block at its source span when targeting 65816 or
   68k.
5. Move conversion from the integrated 6502 assembler's types outside
   `src/nir`.

Completion gate: verifier-clean NIR has no dependency on `crate::asm6502`.

Suggested commit:

```text
nir: isolate target-specific machine payloads
```

### Slice 9: independent backend boundary

Status: complete. `backend::VerifiedNir` now gates target lowering behind the
full NIR verifier and exposes only the verified program, resolved target
layout, and runtime bindings. MIR6502 consumes that token without consulting
SemIR. Independent MIR65816 and MIR68K modules advertise their own target
entry points and currently stop at explicit canary-not-yet-implemented
diagnostics; they do not extend or reuse MIR6502.

1. Extract a backend interface that consumes only verified NIR, target layout,
   and selected runtime bindings.
2. Keep `NIR -> MIR6502` unchanged behind that interface.
3. Add separate MIR65816 and MIR68K entry points rather than extending
   MIR6502.
4. Share target-neutral analyses only when their inputs and outputs contain no
   machine concepts.
5. Keep object format, linker policy, runtime package, and listing syntax out
   of NIR.

Suggested commit:

```text
compiler: define the verified NIR backend boundary
```

### Slice 10: dual-target canaries

Status: complete. Independent MIR65816 and MIR68K canaries lower the portable
scalar, memory, aggregate, control-flow, call, return, and relocation subset
listed below. Tests exercise 65816 native 24-bit pointers, the 65816 small
model on the same 24-bit architecture, 68k big-endian data and 32-bit
pointers, target-selected record/array extents, and the 68k bytewise fallback
for a word field at an odd or otherwise unproven address. These canaries stop
before register allocation and emission by design.

Implement only enough MIR65816 and MIR68K lowering to prove the NIR contract:

- byte and 16-bit constants and arithmetic;
- direct, absolute, and pointer loads/stores;
- a naturally aligned and a packed odd-offset record field;
- array indexing with target-selected stride;
- conditional branches;
- direct and indirect calls;
- data- and code-address relocations;
- record assignment and return.

The 65816 canary must exercise a 24-bit pointer even if its first usable
runtime chooses the small memory model. The 68k canary must exercise
big-endian data, a 32-bit pointer, and an odd packed word field.

Suggested commits:

```text
mir65816: add portable NIR lowering canary
mir68k: add portable NIR lowering canary
```

## Verification Matrix

After every slice that changes semantic lowering, NIR, verification, or the
backend boundary, run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Also keep the relevant MIR6502 fixture sweeps, runtime tests, and representative
Atari object comparisons green. Fixture changes must be classified as an
intentional IR contract change, a printer-only change, or a bug fix.

Add cross-target fixtures for:

- pointer fields and callable fields in records;
- nested and arrayed records under packed and native layout;
- static integer, pointer, and routine-address initializers;
- values above `$FFFF` used as native absolute addresses;
- checked native pointer/integer conversion diagnostics;
- packed odd-address word access;
- indirect calls with code-pointer width distinct from `CARD`;
- overlapping aggregate copies;
- target-mismatched 6502 machine blocks.

## Definition of Done

The NIR portability migration is complete when:

- the same high-level source reaches verifier-clean NIR under Atari 6502,
  65816 native, 65816 small, and 68k layouts;
- target differences are expressed through explicit data-layout and ABI facts,
  not different NIR operation families;
- `Ptr16`, `Absolute(u16)`, generic `ZeroPage`, `may_call_os`, and generic
  16-bit address relocations no longer encode target assumptions in NIR;
- `src/nir` does not import the integrated 6502 assembler;
- NIR verification uses checked target limits and does not truncate addresses,
  offsets, extents, or pointer conversions;
- MIR6502 consumes NIR without consulting SemIR and preserves established Atari
  behavior;
- the MIR65816 and MIR68K canaries consume the same NIR contract without
  introducing bank, endian, alignment, or register concepts above their MIR
  boundaries;
- existing Atari and compatibility record layouts remain unchanged;
- native layout differences are observable through `SYS.SIZEOF`,
  `SYS.ELEMENTS`, `SYS.ALIGNOF`, and `SYS.OFFSETOF` rather than undocumented
  constants, while their unqualified aliases remain shadowable;
- no complete new runtime or object format is required merely to declare the
  NIR boundary proven.

## Recommended Order Before Language Expansion

Complete slices 0 through 9 before adding broad source constructs such as
`LET` or `CASE`. Slice 10 can remain a small proof rather than becoming a full
backend project. Once the boundary is established:

- `LET` remains a SemIR binding that lowers to ordinary NIR locals or temps;
- `CASE` initially lowers to ordinary blocks and branches;
- a target-neutral `Switch` terminator should be added only when real programs
  justify preserving multiway structure for target selection.
