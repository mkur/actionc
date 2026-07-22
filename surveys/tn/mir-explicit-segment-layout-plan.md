# MIR6502 explicit segment layout implementation note

## Problem

The MIR6502 backend still lets one value carry too many meanings: the native
emitter position is used as an emitted-byte cursor, a logical runtime address,
and a proxy for the program high-water mark. Deferred storage made this safer for
large zero-filled byte arrays, but it still depends on synthetic labels bound past
the emitted buffer. That is compatible with the old backend, but it is too easy
to reintroduce subtle layout bugs.

`SET buffer=*` is the important invariant: it must store the runtime high-water
address after all program code and all allocated runtime storage, including large
tables that are not present in the load file.

## Direction

Introduce an explicit MIR layout model with named segments:

- `LoadData`: bytes emitted before routines.
- `Code`: routine bytes emitted into the load image.
- `DeferredData`: runtime-allocated storage that is omitted from the load image.
- Existing absolute and zero-page placements remain address spaces, not load
  segments.

The final layout should be the only source of truth for:

- storage symbol addresses,
- routine addresses,
- skipped ranges,
- emitted load-file bytes,
- and `runtime_high_water`.

## MODULE requirement

ACTION! `MODULE` is a source-level boundary, not a separate MIR load segment.
NIR/MIR currently flatten declarations and routines after semantic analysis. The
new layout must preserve that flattened order across modules:

1. global declarations from module 0, then module 1, etc.,
2. routine frame storage for routines in flattened routine order,
3. routine code in flattened routine order,
4. deferred storage in flattened allocation order.

Regression tests must include `MODULE` separators so future refactors do not
accidentally reset layout state, high-water state, skipped ranges, or run-address
selection per module.

## Implementation slices

1. Add segment/layout structs with behavior kept equivalent.
   - `MirSegmentKind`
   - `MirAllocation`
   - `MirLayoutPlan`
   - helpers for `emitted_end`, `runtime_high_water`, `skipped_ranges`

2. Route current storage placement through layout allocations.
   - Existing `storage_items` become emitted `LoadData` allocations.
   - Deferred globals/locals become `DeferredData` allocations.
   - Absolute and zero-page placements stay outside emitted segments.

3. Replace ad hoc skipped-range storage with layout-derived skipped ranges.
   - `CodegenOutput.skipped_ranges` should come from `MirLayoutPlan`.
   - The map should use the same vector.

4. Replace the synthetic program-end label dependency with layout high-water.
   - Keep native label patching only for code labels.
   - Patch `ProgramEndWord` storage from `layout.runtime_high_water()`.
   - This makes `SET buffer=*` independent of emitted byte length.

5. Preserve the current two-pass code-size measurement initially.
   - First pass measures code size.
   - Final pass builds a complete layout with code and deferred addresses.
   - Later, this can become a fixed-point pass if branch relaxation changes code
     size based on layout.

6. Add focused tests.
   - `SET buffer=*` after code, large global table, and large local table.
   - Same with `MODULE` between declarations and routines.
   - Load file omits deferred ranges.
   - Uninitialized local arrays of every size are deferred; initialized local
     arrays remain emitted.
   - The existing global threshold remains distinct: 256-byte global arrays
     remain emitted and 257-byte global arrays are deferred.
   - Initialized byte arrays remain emitted.
   - `map.skipped_ranges == output.skipped_ranges == layout.skipped_ranges`.

## Expected outcome

The emitted load image and the runtime allocation high-water become separate,
explicit concepts. This keeps compatibility with ACTION! style `SET buffer=*`
while giving MIR6502 a cleaner model than the original 1980s load layout.
