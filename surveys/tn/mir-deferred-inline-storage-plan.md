# MIR Deferred Inline Storage Plan

## Problem

TOMS Navigator initializes both panels by calling `InitPanels`, which in turn
calls `SetWin(1, ramdisk)` when MyDOS reports a RAMdisk in `$070A`. The MIR
binary reaches that call, but the program now reports I/O error 137 on startup.

The visible `InitPanels` control flow matches the old backends. The major
difference is object layout: MIR emits large routine-local inline tables and
spill slots into the loaded image before code, pushing `SET BUFFER=*` to about
`$83F8`. The old compatible codegen omits large inline byte-array backing ranges
from the load image, records them as skipped ranges, and lets `SET buffer=*`
point at the high-water address after code and deferred backing storage.

## Goal

Teach MIR layout to handle large inline byte-array storage like the old
compatible codegen:

1. Keep small inline data in the load image.
2. Place large inline byte-array storage after code as deferred/skipped storage.
3. Record skipped ranges so load-file emission omits those zero-filled ranges.
4. Keep symbol addresses stable for code generation and listings.
5. Make `ProgramEndWord` / `SET symbol=*` resolve to the high-water address that
   includes deferred backing storage, matching the old codegen behavior.

## Reference Behavior

Old compatible codegen paths to mirror:

- `src/codegen/storage.rs` decides when a sized byte array becomes
  `StorageInit::Skip`.
- `src/codegen/data.rs` records skipped ranges without emitting zero bytes.
- `src/codegen/program.rs` emits routines, final RTS, then array backing storage
  metadata.
- `src/codegen/output.rs::format_load_file` currently writes one contiguous
  load segment, so MIR must either avoid holes in `bytes` or extend load output
  handling when skipped ranges are introduced.

## Slices

1. Tests only:
   - Add a MIR fixture with a large local byte array and `SET buffer=*`.
   - Assert the large local range is reported as skipped/deferred and omitted
     from emitted bytes/load size.
   - Assert small byte arrays remain inline.

2. MIR layout model:
   - Add skipped/deferred storage representation to MIR emission summary/output.
   - Mark large uninitialized inline byte-array slots as deferred using the old
     compatible threshold.
   - Keep normal absolute addresses for references to the slot.

3. MIR emission:
   - Do not emit zero bytes for deferred ranges in the primary byte stream.
   - Bind labels/addresses for deferred ranges.
   - Bind `PROGRAM_END_LABEL` to the high-water address after deferred ranges.

4. Load/listing integration:
   - Ensure `CodegenOutput.skipped_ranges` and map skipped ranges are populated.
   - If necessary, teach load-file formatting to split around skipped ranges
     rather than serializing holes.
   - Keep listings intelligible for skipped storage.

5. TN verification:
   - Compare MIR `BUFFER`/image high-water against old/semir-native.
   - Re-run targeted MIR tests and TN stability checks available locally.

Commit after each slice that leaves the compiler in a coherent, tested state.
