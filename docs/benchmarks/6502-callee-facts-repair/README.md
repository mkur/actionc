# MIR6502 callee-fact repairs

The original [audit](../6502-tn-leaf-facts/README.md) remains unchanged.
ActionCart TN at `$3000`, optimized NIR/MIR6502, is **9,862 bytes** after the
two correctness repairs, versus the original 9,791 bytes. The subsequent
[private-storage boundary](../6502-private-storage-boundary/README.md) produced
a 9,870-byte baseline for the directional-write review below.

## Correctness baseline (slices 1–2)

[Full facts and routine footprints](correctness/facts.compact.json),
[static cross-check](correctness/audit.json), [assembly](correctness/leaves.asm),
[hashes and configuration provenance](correctness/receipt.json), and
[routine deltas](correctness/comparison.json) record the selected program.
Large image/map/relocation files remain under
`build/callee-facts-repair/correctness/`; their hashes are in the receipt.
Diagnostic layout replay reproduced executable bytes and relocations exactly.

All 101 routines and 40 no-MIR-call candidates remain. Logical summary changes
are confined to Position/GetD losing unsupported N/Z provenance and
CalcAdr/Internal/Ascii widening indexed reads to Unknown. Their independent
write and register facts remain. No candidate leaf body changes size.

Growth is confined to callers of the newly conservative machine routines:
Putchar gains four code bytes and one inline parameter byte; FormatDirectoryRow
gains 42 code bytes; SetWin gains 24. Assembly comparison shows retained
store/reload sequences (respectively one, eight, and four additional stores).
Putchar replaces its Y carrier with capture/reload. The old base-byte read
footprints incorrectly allowed these storage observations to be excluded.
This is conservative correctness growth, not evidence that TN observes those
bytes at runtime. Bounded indexed-read inference remains deferred.

Ord remains 51 code bytes with five ZP home bytes; Items remains 37 with three;
Next remains 30 with no inline homes. FindItem remains 74 with three inline
home bytes.

Focused checks: the slice-1 summary group and default/optimized LF/CRLF VM
control; all 29 tests under `mir6502::analysis::known_callees` for the shared
machine decoder; the slice-2 indexed/offset VM group; one TN facts generation.
No full test suite or other backend suite was run.

## Directional-write patch review (slice 3)

The reviewed patch derives native write summaries from write-specific effects,
retaining Unknown for indirect/opaque writes, unresolved homes and unexpanded
logical temporaries. It removes the isolated pointer-offset exception. Unknown
pointee reads and existing entry restrictions remain independent. Implicit CPU
stack traffic follows the existing ABI contract; balanced PHA/PLA staging is
checked separately from explicit data writes.

ActionCart TN at `$3000` shrinks **9,870 → 9,846 bytes**, a 24-byte saving.
FindItem accounts for the entire change: code shrinks 74 → 53 bytes, and three
inline-home bytes move to ZP. All other routine code sizes are unchanged.
Key remains 34 bytes; Ord, Items and Next retain their previous footprints.

Key, Instr and Fnamecmp change from Unknown to Exact writes; their reads remain
Unknown. The emitted-instruction scan finds no uncovered direct writes or
contradicted register-preservation claims in the leaf inventory. Key writes
`$A0,$AC–$AF`, and both its FindItem and PopUp call sites pass residence checks.
PopUp receives no new homes. Instr remains blocked by its unknown entry contract;
Fnamecmp remains blocked by extra entry flags/stack requirements. Next and Value
retain their existing exact write summaries. No selection or cost guard changes.

The facts probe reproduced final bytes and relocations exactly. Full facts,
assembly, map, relocations, receipt and static audit are under
`build/callee-facts-repair/directional-review-final/`. The 9,846-byte XEX SHA-256
is `2419293a359d4b20b4d8af576b5b7efc7d2b84a0e4f682e1ef137679d92167ae`.
The measured compiler is based on `2f7ecd9e` plus the directional-write patch.
The copy test's destinations were corrected after measurement; production code
was unchanged. TN application execution is outside this measurement.

The independent reader/writer VM group passes in default and optimized modes
for ActionCart and Standalone. It checks live caller values, page crossing,
result bytes, source sentinels, stack balance and reader-only home promotion.
All 30 callee-analysis tests pass, including the grouped summary/emission check
for six read recipes and conservative write/opaque/unresolved-home negatives.
No full suite or other backend suite was run.
