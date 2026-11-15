# MIR6502 callee-fact repairs

The original [audit](../6502-tn-leaf-facts/README.md) remains unchanged.
ActionCart TN at `$3000`, optimized NIR/MIR6502, is **9,862 bytes** after the
two correctness repairs, versus the original 9,791 bytes. Subsequent precision
gains are measured against 9,862.

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
