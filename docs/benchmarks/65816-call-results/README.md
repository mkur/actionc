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
