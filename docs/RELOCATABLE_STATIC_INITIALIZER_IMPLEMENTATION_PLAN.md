# Relocatable Static Initializers

## Goal

Allow static Action! data to contain addresses that are resolved after final
layout. The motivating example is a self-referential ANTIC display list:

```action
BYTE ARRAY dlist(9)=[$70 $70 $70 $52 0 0 $41 <dlist >dlist]
```

The same mechanism should support address tables without making either classic
code generation or MIR6502 reinterpret source text:

```action
CARD ARRAY handlers=[@Draw @Update]
```

`<name` and `>name` select the low and high byte of the final address. A word
element stores an Action! address in the normal low-byte/high-byte order.
Constant addends, such as `<table+4`, are part of the relocation rather than
being folded before layout.

## Boundary ownership

- The parser owns the initializer-list grammar and source spans.
- SemIR owns name resolution, addressability, element-width legality, and the
  distinction between a value and an address.
- NIR owns target-independent initialized data: literal bytes plus stable-ID
  address relocations.
- MIR6502 owns conversion of NIR storage targets into MIR storage/routine
  identities, but does not recover facts from SemIR.
- Emission owns label binding and final relocation patching after layout.

An array address always means its element backing address. It must not silently
change to the address of a pointer or descriptor cell.

## Data contract

All data-bearing NIR initializers should use one shared shape:

```text
NirDataImage {
    bytes,
    relocations
}

NirDataRelocation {
    offset,
    kind: Low8 | High8 | Word16,
    target: Storage(stable ID) | Routine(stable ID) | Absolute,
    addend,
    source span
}
```

The byte payload contains placeholders at relocation positions. Relocation
width contributes to the initialized length; zero-fill begins only after the
entire data image.

## Implementation slices

Status as of 2026-08-06: slices 1-10 are implemented. Relocatable images are
verifier-clean NIR, and MIR6502 plus both classic paths resolve them after
layout. Numeric-only initializers continue through all backends unchanged.

### Slice 1: stop silent initializer loss

Status: complete.

- Reject initializer-list tokens that the compiler cannot represent.
- Never turn a present but unsupported initializer into implicit zero-fill.
- Add a regression for symbolic list elements and malformed list syntax.

### Slice 2: structured parser representation

Status: complete.

- Replace raw bracket text with a structured initializer-list AST node.
- Preserve numeric, character, `TRUE`, `FALSE`, `NIL`, optional signs, and
  optional commas.
- Parse `<name`, `>name`, and address-word elements with precise spans and an
  optional constant addend.

### Slice 3: SemIR resolution

Status: complete.

- Lower initializer lists to structured SemIR elements.
- Resolve targets to `SemSymbolRef`, including self references and static
  forward references.
- Reject unresolved, non-addressable, runtime, or width-incompatible elements.
- Keep raw initializer text out of downstream executable IR.

### Slice 4: target-independent NIR data images

Status: complete.

- Introduce `NirDataImage` and stable-ID `NirDataRelocation` types.
- Use the common image in byte initializers, descriptor backings, local
  storage initializers, and static objects.
- Lower SemIR initializer elements to literal bytes and relocation records.
- Treat relocation targets as address-observable storage roots.

### Slice 5: NIR verification and printing

Status: complete.

- Verify relocation bounds, width, overlap, stable target identity, scope,
  addend range, and initialized-data ownership.
- Reject verifier-clean NIR that retains a raw initializer dependency.
- Print relocations readably in NIR snapshots.

### Slice 6: MIR6502 lowering

Status: complete.

- Add the corresponding MIR data-image relocation representation.
- Translate NIR stable IDs without consulting SemIR.
- Preserve array-backing rather than descriptor-address semantics.

### Slice 7: MIR6502 layout and emission

Status: complete.

- Bind storage, backing, static, and routine labels before emitting data.
- Emit literal spans and low/high/word fixups in source order.
- Support self references, forward references, fixed addresses, and addends.

### Slice 8: classic emission

Status: complete.

- Extend AST/classic and SemIR-native classic storage initializers with the
  same low/high/word fixups.
- Use labels rather than prematurely resolved numeric addresses.
- Cover legacy and modern layouts without changing numeric initializer bytes.

### Slice 9: sample and documentation

Status: complete.

- Put `<dlist >dlist` directly in the fine-scroller display list.
- The first version removed the runtime stores to `dlist+7` and `dlist+8` but
  retained the runtime LMS patch because it used `SAVMSC`. The sample now uses
  a static `SCREEN`-encoded buffer and relocates the LMS address to that buffer
  as well.
- Document symbolic byte and word initializers.

### Slice 10: validation

Status: complete.

- Test self, forward, routine, fixed-address, and addend relocations at
  multiple origins.
- Test unknown targets, invalid widths, overlap, overflow, and malformed lists.
- Compare legacy/classic, modern/classic, SemIR-native classic, and MIR6502.
- Confirm existing numeric initializer output remains byte-identical.
- Rebuild the fine-scroller ATRs.

Validation covers self, forward, routine, fixed-address, local, parameter, and
addend targets, including multiple origins. Existing semantic and verifier
tests cover unknown targets, invalid element widths, malformed elements,
out-of-bounds relocations, and overlap. The final validation also found and
fixed a MIR6502 interaction where write-only parameter-home elision did not
count initializer relocation references as address-observable uses.

The fine-scroller was compiled and packed into temporary ATRs through
legacy/classic, modern/classic, SemIR-native classic, and MIR6502. No generated
ATR is checked into the repository.

Required final checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

## Invariants

- Unsupported initialized data is a diagnostic, never implicit zeros.
- NIR and MIR never parse initializer source strings.
- Relocations use stable identities, not display names.
- Calls, machine blocks, and memory effects are unrelated to static-data
  relocation; taking an address is not a memory read.
- Numeric-only initializers retain their existing layout and bytes.
