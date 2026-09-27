# Native LONGCARD/LONGINT ordering measurements

The [three-slice plan](../../MIR65816_LONG_ORDERING_PLAN.md) uses the frozen
optimized Exec `622b139-dirty` workload: 631 routines, eight task slots, shell,
console/windows and MyDOS. All 120 frozen input hashes are rechecked. These are
compiler-only size builds; no final backend or Exec qualification is run.

| Checkpoint | Compiler code, including guards | Saved in slice |
| --- | ---: | ---: |
| Baseline `2f52fc65` | 411,160 B | — |
| LONGINT sign tests | 410,152 B | 1,008 B |
| LONGCARD ordering | 404,149 B | 6,003 B |
| LONGINT ordering | 403,239 B | 910 B |

Total compiler-code reduction: **7,921 bytes (1.93%)**.

All 2,676 compiler guards remain (72,252 bytes). Compiler initialized data stays
951 bytes; routine contracts, frames and zero-fill are unchanged. Reserved
bank-zero delta is zero, including per-task capacity. Assembly and packaging are
not rebuilt. Subtracting guards is not an actual guard-free release measurement.

The sign-only materialized form uses 12 bytes for `< 0` and 14 for `>= 0`,
including entry SEP. It reads only the captured top byte and writes canonical
BYTE 0/1. Branch-only conditions use the top byte's N directly. Complete source
captures remain intact. Comparisons requiring a zero decision initially kept
fallback and now use the general signed form from slice 3.
Raw lowering can retain a widening temp for zero; this slice deliberately does
not add constant propagation.

Slice 1 checks: five selector tests; 34 emission/o65/boundary integration tests;
four new native execution tests in debug and release, two debug replay tests,
and one release IRQ/NMI test. Coverage includes all 256 top-byte patterns,
Boolean consumers, mutable values, complete bank-crossing volatile captures,
call mutation, ca65 encodings, private access traces, independent o65 placements,
and LF/CRLF compilation/instrumentation. The [measurement record](long-sign.json)
contains artifact hashes, changed routines and focused check manifests.

Unsigned ordering compares the high words first and the low words only on a
tie. Swapping captured operands implements `>` and `<=` with the same C-based
decision. Both canonical Boolean materialization and sole-use branches select
this form without additional scratch storage.

Slice 2 checks: six selector tests; 34 emission/o65/boundary integration tests;
seven ordering/sign execution tests, five equality regressions and two replay
tests in debug; three unsigned execution tests and one IRQ/NMI test in release.
Ordering coverage includes all four predicates, 240 boundary/seeded pairs,
constants on either side, mutable parameter homes, ca65 encodings, exact private
read traces, independent o65 placements and LF/CRLF inputs. The
[unsigned measurement](long-unsigned.json) records the unchanged guards, data,
routine contracts and zero bank-zero delta.

Signed ordering subtracts both word pairs, propagates low-word borrow, then
corrects high-word N for overflow. It handles all four predicates, including
`<= 0` and `> 0`, while retaining the top-byte sign specialization. Existing
typed instructions cover the sequence without new opcodes or scratch storage.

Slice 3 checks: seven selector tests; 34 emission/o65/boundary integration tests;
13 ordering/sign tests and five equality regressions in debug; nine ordering
tests and both signed/unsigned IRQ/NMI cases in release. The ordering tests now
also check complete bank-crossing volatile captures around calls, same-target
parallel edges, loop backedges and exact direct/replayed output and proof
observations. Signed boundary/seeded pairs cover borrow and overflow, with
independent Rust comparisons and ca65 encodings. Raw/optimized compilation,
LF/CRLF inputs, both incoming I states and independent o65 placements remain
covered. See the [signed measurement](long-signed.json).
