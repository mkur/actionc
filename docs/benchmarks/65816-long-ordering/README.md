# Native LONGCARD/LONGINT ordering measurements

The [three-slice plan](../../MIR65816_LONG_ORDERING_PLAN.md) uses the frozen
optimized Exec `622b139-dirty` workload: 631 routines, eight task slots, shell,
console/windows and MyDOS. All 120 frozen input hashes are rechecked. These are
compiler-only size builds; no final backend or Exec qualification is run.

| Checkpoint | Compiler code, including guards | Saved in slice |
| --- | ---: | ---: |
| Baseline `2f52fc65` | 411,160 B | — |
| LONGINT sign tests | 410,152 B | 1,008 B |

All 2,676 compiler guards remain (72,252 bytes). Compiler initialized data stays
951 bytes; routine contracts, frames and zero-fill are unchanged. Reserved
bank-zero delta is zero, including per-task capacity. Assembly and packaging are
not rebuilt. Subtracting guards is not an actual guard-free release measurement.

The sign-only materialized form uses 12 bytes for `< 0` and 14 for `>= 0`,
including entry SEP. It reads only the captured top byte and writes canonical
BYTE 0/1. Branch-only conditions use the top byte's N directly. Complete source
captures remain intact, and comparisons requiring a zero decision keep fallback.
Raw lowering can retain a widening temp for zero; this slice deliberately does
not add constant propagation.

Slice 1 checks: five selector tests; 34 emission/o65/boundary integration tests;
four new native execution tests in debug and release, two debug replay tests,
and one release IRQ/NMI test. Coverage includes all 256 top-byte patterns,
Boolean consumers, mutable values, complete bank-crossing volatile captures,
call mutation, ca65 encodings, private access traces, independent o65 placements,
and LF/CRLF compilation/instrumentation. The [measurement record](long-sign.json)
contains artifact hashes, changed routines and focused check manifests.
