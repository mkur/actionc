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

## Final qualification (slices 4–7)

All seven slices are complete. TN remains **9,846 bytes** after the entry and
metadata repairs: no additional savings beyond slice 3's 24 bytes. Executable
bytes, relocations and every routine's code/ordinary/ZP footprint are identical
to that baseline. This is 16 bytes below the 9,862-byte correctness baseline
and 55 above the original 9,791-byte image with the unsound facts; the intervening
private-storage boundary accounted for eight bytes of growth.

[Final facts](final/facts.compact.json), [static audit](final/audit.json),
[assembly](final/leaves.asm), [comparison](final/comparison.json), and
[receipt](final/receipt.json) preserve the evidence. The raw image, map and
relocations are under `build/callee-facts-repair/final/`. The measured compiler
is `af087445` plus the diagnostic-only caller-eligibility event committed with
this qualification. The receipt records source/compiler/probe hashes and the
pre-hook compiler diff; the temporary probe was removed after generation.

The inventory remains 101 routines and 40 no-MIR-call candidates: 18 emitted
call-free routines, one empty Error declaration, fifteen transferring assembly
wrappers, and six menu data blocks. The unchanged emitted bodies preserve the
original classification. All entry requirements and both logical exit-summary
variants were compared against slice 3. The only entry changes are:

| Routine | Before | After | Independent call-contract checks |
| --- | --- | --- | --- |
| Fnamecmp | A/X/Y, `$A3`, incoming C/Z | A/X/Y, `$A3`; no flags | Sort's call accepted |
| Instr | Unknown entry | A/X/Y; no flags or extra fixed bytes | Both Handle calls accepted |

The direct-memory byte CMP recipe used by Instr needed the same definite-flag
certification as the indirect compare recipe. Its array-reference parameter
retains Array storage with an explicit two-byte pointer ABI descriptor. All
other register, flag and fixed-byte entry requirements remain unchanged.
Logical exit summaries are unchanged from slice 3: Key/Instr/Fnamecmp retain
Exact writes and Unknown reads; Next and Value remain controls. Position/GetD
still have no unsupported N/Z provenance. CalcAdr/Internal/Ascii still have
Unknown indexed reads. Native exit N/Z remains withheld from rewriting.

The instruction scan reports no uncovered direct accesses, unresolved identities
in Exact footprints, or contradicted X/Y preservation claims. It remains an
audit aid: indirect accesses, transfers and menu data are not semantic proofs.
Independent execution controls establish the repaired contracts.

Caller eligibility is a separate decision from these independent call checks:

| Caller | Actual selector outcome | Independent leaf check | Selected code / mapped ordinary / mapped ZP bytes |
| --- | --- | --- | --- |
| FindItem | Proposed and accepted with Ord/Items | Key and Next accepted | 53 / 0 / 3 |
| PopUp | Rejected by caller eligibility; `c` is address-taken via `@c` | Key accepted | 261 / 11 / 0 |
| Sort | Rejected by caller-body eligibility; array writes remain unsupported | Fnamecmp accepted | 250 / 3 / 0 |
| Handle | Rejected by caller-body eligibility; includes machine blocks and indirect writes | Both Instr calls accepted | 791 / 5 / 0 |

The trace's `cross-call-ineligible` event records that first gate separately
from callee rejection and binding proposal. Its operation-effect indicators
describe the body, including call effects; the source/eligibility inspection
above identifies concrete restrictions. This work does not relax them.
Mapped home counts include parameter/local map entries, not all working storage
or source globals. Sort's source globals `i`/`j` remain at `$E0/$E1`; Next has no
mapped parameter home. Ord stays 51 code bytes with five mapped ZP bytes, and
Items stays 37 with three; Next stays 30 code bytes.

Map changes are confined to `routine_signatures`. Twenty of the forty audited
candidate signatures change; PROC/FUNC and source types now come from verified
callable identity. TaggedFiles and Key report BYTE results; Next reports CARD.
Pointer/array-reference parameters retain their pointer types. Generated/MIR-only
routines with no source signature use UNKNOWN instead of an invented PROC result.

The completed trace records three placement trials: leaf homes, argument handoff
and **one combined ordinary-call batch**. Within this compilation that last batch
is accepted at 9,926 → 9,846 bytes with Ord/Items/FindItem bindings. These phase
savings are existing selection behavior, not additional savings over slice 3.
The trace is complete and records six materializations, 145 emission invocations,
870 inner emission probes, 17 preservation replays and 67 pipeline attempts.
Its 50,356 analysis-build count covers shared rewrite drivers only. Diagnostic
layout replay and fact collection occur afterward and are excluded from those
work counts. Replay reproduced the executable and relocations exactly.

One source-to-executable sample with tracing took 20.63 seconds; an end-of-generation
resident-memory sample was 543,360 KiB. This includes the test process and trace
collector, excludes Rust compilation, and is neither peak RSS nor an untraced
performance benchmark. No repeated performance runs were made.

Focused validation for the remaining slices:

- Slice 4: two new grouped compare/entry checks, all 11 machine-liveness tests,
  six entry-flag tests, the independent compare VM group, and one existing
  entry-flag integration control. The optional MADS case remained ignored.
- Slice 5: seven callee-fact unit groups, all five independent VM groups,
  and 15 existing parameter-capture/handoff tests. VM controls cover default and
  optimized ActionCart/Standalone, page crossing and live caller values.
- Slice 6: the whole-pipeline callable-map group and existing elided-parameter
  signature control; only diagnostic map fields change.
- Slice 7: one TN generation/static audit and seven existing controls: four
  multiple-leaf cases (including later-site rejection and exact rollback), two
  caller lifetime/call-input cases, and selected-input/executable rollback.

No full suite, other backend suite, or TN application execution was run. Broader
ZP placement and nested-call support remain separate work.
