# Native call cleanup and result forwarding

Phase 1 implements unused call-result cleanup, then direct forwarding of an
immediately returned native call result. Each slice retains allocated homes,
the public ABI, stack guards and declared callee effects.

## Slice 1: unused result cleanup

Outgoing-area release omits TAY/TYA when no result is captured. Discarding a
non-void result still keeps its declared ABI effects. Used results preserve the
complete A/X value during cleanup; direct and indirect calls use the same rule.

The frozen Exec workload is unchanged from compiler `9bff177b`: 631 routines
and all 120 frozen input hashes. Compiler code shrinks **389,725 → 388,095 B**,
a **1,630-byte saving**. The 813 changed call spans save 1,626 bytes; four
BRL-to-BRA relaxations save the remaining four. There are 291 smaller routines
and no larger ones.

All 2,676 guards remain identical at 72,252 bytes. Guard-subtracted compiler
code is **315,843 B**. Compiler initialized data remains 951 bytes. Frames,
temporary homes, ABI metadata, local stack peaks and bank-zero reservations
are unchanged. Guard subtraction is an accounting estimate, not a separately
compiled guard-disabled release image.

[Size and input hashes](cleanup/exec-summary.json),
[routine deltas](cleanup/exec-routines.csv), and
[changed spans](cleanup/exec-spans.csv) retain the measurement.

Focused checks: nine emitter call tests; 22 emission integration tests; the
reviewed state-boundary snapshot with LF and CRLF expected text; 17 native
debug tests across `call_copies`, `call_padding`, `replay` and `state_tracking`;
and both `call_copies` tests in release. Direct/indirect discarded results cover
all four widths, used results, padding, register/scratch clobbers, repeated
calls, actual LF/CRLF source compilation and relocated calls. The snapshot
update was independently derived by deleting TAY/TYA at its two void calls
and remapping later positions; no other instruction changed.

Full backend and hosted Exec qualification were not run.

## Slice 2: immediate BYTE/word returns

A final direct native call whose result is used only by the adjacent Return
keeps its canonical A value through outgoing cleanup and frame teardown.
Matching ABI result lanes, complete call preflight and typed occurrence counts
are required. Indirect calls, other uses, casts and intervening operations keep
the existing path. Call and Return retain separate source spans. The reserved
temporary remains allocated, but its capture and reload disappear; no stored
home definition is published for the omitted capture.

Against `39016a89`, compiler code shrinks **388,095 → 386,222 B**: **1,873 B**
saved. All 198 audited pairs qualify: 98 BYTE results save 15 bytes each and
100 word results save four each, totalling 1,870 bytes. Three additional
BRL-to-BRA relaxations save three bytes. There are 98 smaller routines and no
larger ones. Guard-subtracted compiler code is **313,970 B**; the cumulative
phase saving is **3,503 B**.

The same 120 frozen inputs, MIR operations, guards, initialized data, frames,
temporary homes, ABI metadata and local stack peaks remain unchanged.
[Size and hashes](byte-word/exec-summary.json),
[routine deltas](byte-word/exec-routines.csv), and
[changed spans](byte-word/exec-spans.csv) retain the measurement.

Focused checks: 12 emitter call tests and 34 emission/o65/state-boundary
integration tests pass. Twenty native debug tests pass across `call_returns`,
`captured_byte_returns`, `word_returns`, `replay` and `state_tracking`; the three
new `call_returns` tests also pass in release. Independent ca65 callers/callees
check exact cleanup bytes, register/scratch clobbers, zero extension, and the
absence of private result reads/writes. Recursion executes at two o65 placements.
Both frontend modes and actual LF/CRLF compilation are covered.

IRQ and NMI checks cover all 13 cleanup/return instructions in both task domains,
both I states and every admitted source type. They compare complete registers,
live stack bytes and the suspended DP domain with one independently executed
instruction; released stack bytes are correctly excluded after TCS/RTL. The IRQ
dispatcher reenters the same forwarding routine. The reviewed snapshot is
unchanged. Full backend and hosted Exec qualification remain excluded.
