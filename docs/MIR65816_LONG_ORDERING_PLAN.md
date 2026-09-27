# Native 32-bit ordering

Status: all three slices implemented and focused checks passed. Final backend
and Exec qualification remains intentionally unrun.

Implement and commit three MIR65816 selector slices:

1. Captured LONGINT sign tests: `x<0`, `x>=0` and reversed equivalents.
   Validate the complete four-byte source, then inspect its private top byte.
   Materialized results are canonical BYTE 0/1; sole-use branch conditions use
   N directly. `x<=0` and `x>0` also need a zero decision and are handled
   by the general signed ordering in slice 3.
2. Unsigned LONGCARD ordering: compare high words first, and low words only
   when the high words match. Normalize `>`/`<=` by swapping captured operands.
   Share one preflight between materialized results and branch-only conditions.
3. Signed LONGINT ordering: subtract low words, propagate borrow into high-word
   subtraction, and use corrected N (N xor V) for the signed decision. Retain
   the sign-only specialization and unsigned equality behavior.

All slices accept complete private stack temps/authoritative parameter homes and
U32 constants. Preflight both inputs and the BYTE result home before emission.
Unsupported geometry keeps the existing path; malformed homes remain errors.
Raw lowering may retain an explicit widening temp instead of a U32 zero, in
which case general ordering consumes those captured operands without constant
propagation.

Keep every external source capture, volatile access, source evaluation order,
call barrier, frame allocation, DP reservation, ABI and guard policy intact.
Use typed tracked instructions and the existing sole-condition-use proof for
branch fusion. Both successor edges retain their parallel-copy semantics.
No flags or register values are assumed to survive calls.

Run focused selector, emission/relocation/replay and emitted-code tests. Cover
raw/optimized compilation, canonical Boolean consumers, signed extremes,
low-word borrow, unsigned word boundaries, incoming I states, hidden B,
complete source reads, independent o65 placements and IRQ/NMI restoration.
Check new sequences with ca65. Normalize host text before instrumentation and
exercise LF/CRLF through the source/instrumentation path.

Use the frozen 631-routine Exec workload for size-only comparisons. The baseline
compiler is `2f52fc65`: 411,160 compiler bytes, including 72,252 guard bytes.
See [measurements](benchmarks/65816-long-ordering/README.md). Candidate operation
costs are not savings forecasts. The user requested that final qualification
remain off; no full backend/Exec qualification or pin change is part of this
series. Commit each slice after its focused checks and measurement.
