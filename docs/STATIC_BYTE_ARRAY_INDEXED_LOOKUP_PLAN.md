# Static Byte Array Indexed Lookup Plan

## Goal

Compile dynamic byte indexes into directly allocated byte arrays with 6502
absolute-indexed addressing. This applies to inline global, routine-local,
static-data, and fixed-address arrays. Pointer- and descriptor-backed arrays
remain indirect.

The motivating cartridge-compiler shape is:

```text
index = (cnt + VCOUNT) & 3
LDX index
LDA col,X
```

`actionc` currently constructs a 16-bit pointer for the same `col(index)`
expression when the index is computed rather than loaded from one scalar.

## Ownership

- SemIR continues to own array identity, element type, and inline versus
  pointer-backed storage facts.
- NIR continues to represent the evaluated base address, typed byte index, and
  indexed load or store. No target addressing mode is added to NIR.
- Classic and MIR6502 lowering select absolute indexed X or Y when the base is
  direct storage and the index is byte-sized.
- Emission resolves the selected local/global/static storage identity to its
  final address.

## Vertical slices

1. Add global and local initialized-byte-array tests with computed indexes.
2. Extend classic lowering beyond `direct_scalar_slot(index)` while preserving
   index evaluation order and exactly-once behavior.
3. Preserve routine-local direct-storage identity through MIR6502 lowering so
   the existing absolute-indexed materializer can consume it.
4. Cover direct loads and stores; retain indirect lowering for pointer and
   descriptor storage.
5. Restore the timing-sensitive `logo.act` lookup after `WSYNC` and verify that
   all compiler variants emit a direct indexed lookup.

## Initial scope and invariants

- Element width is one byte.
- Index width or proven range fits one byte.
- Mutable arrays are supported; this is addressing selection, not value
  caching.
- Calls, machine blocks, absolute hardware accesses, and memory effects are not
  reordered.
- Index and stored-value expressions are evaluated once in source order.
- Word-element scaling is a separate follow-up.

## Acceptance checks

- Eligible local and global reads contain `LDA base,X` or `LDA base,Y` and no
  array-base pointer staging.
- Eligible stores contain `STA base,X` or `STA base,Y` with the same safety
  properties.
- Pointer- and descriptor-backed negative fixtures remain indirect.
- `logo.act` writes `COLPF2` from a direct indexed lookup after `WSYNC`, cycles
  through all four colors, and does not re-enter its DLI handler.
- Required checks pass:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```
