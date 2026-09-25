# Pointer-copy coalescing and native bitwise operations

Two separately committed MIR65816 slices, following native long ordering.
Measurements use the same frozen optimized Exec `622b139-dirty` workload:
631 routines, eight task slots, shell, console/windows and MyDOS. All 120 frozen
input hashes are checked. These are compiler-only size builds. Final backend
and Exec qualification remain off at the user's request.

| Checkpoint | Compiler code including guards | Saved in slice |
| --- | ---: | ---: |
| Baseline `c4ea5514` | 403,239 B | — |
| Pointer-copy coalescing | 399,225 B | 4,014 B |
| Native 16/32-bit AND/OR/XOR | 397,368 B | 1,857 B |

Total compiler-code reduction: **5,871 bytes (1.46%)**.

Pointer coalescing eliminates the transfers at 497 three-byte cast sites. Total
three-byte cast spans decrease from 6,474 to 2,182 bytes; surrounding mode/layout
changes leave a net 4,014-byte saving. Eligibility requires a dying private
captured value and a bit-preserving three-byte cast. The stack-only liveness
exception preserves all other interference. Each trial is independently checked
for whole-home geometry and all third-party lifetimes; partial overlaps fail.
Edge argument/parameter locations, frame reservations and staging stay fixed.
The selector retains the cast and operation barrier, emitting no transfer for
identical stack homes. No source-memory read or write is eliminated.

The [pointer measurement](pointer-coalescing.json) records artifact hashes,
routine deltas and focused test manifests. Checks cover selector/preflight,
allocation/liveness/staging, emission/o65/boundaries, exact replay, empty cast
spans, every pointer bit, calls, mutable parameter homes, volatile and
bank-crossing accesses, independent relocation, parallel edges/backedges,
LF/CRLF inputs, both incoming I states and IRQ/NMI task restoration. Existing
pointer and stack-allocation runtime tests also pass.

All 2,676 compiler guards remain (72,252 bytes). Initialized compiler data stays
951 bytes. ABI contracts, frame extents, stack peaks and zero-fill are unchanged;
only private stack temp offsets may move. Reserved bank-zero delta is zero.
Assembly/packaging are not rebuilt, and this is not a guard-free release-size
measurement.

The second slice selects A16 AND/OR/XOR for captured 16/32-bit values, with one
word operation per 16-bit lane. It retains full operand preflight, exact source
captures and the existing fallback for unsupported homes. The 69 relevant MIR
spans shrink from 2,560 to 819 bytes, with surrounding mode/forwarding/layout
changes giving the net 1,857-byte reduction. No homes or frame allocations change
in this slice. See the [bitwise measurement](native-bitwise.json).

Bitwise checks cover signed/unsigned 16/32-bit values, walking bits, boundary and
seeded pairs, constants on both sides, mutable parameters and values across
calls. Independent ca65 encodings and exact memory traces check complete private
words with no DP staging; C/V and X/Y remain intact. Instruction-effect probes
check every declared read/write and preserved register/flag bit, including both
accumulator widths. Direct/replayed images and proof observations match.
Relocation, volatile bank-crossing captures, LF/CRLF source inputs and IRQ/NMI
restoration pass in focused runs. Existing word/long arithmetic and accumulator/
frame-forwarding regressions also pass; nine disassembler tests pass.

The emission snapshot changes only `wide_shift`'s raw/optimized 32-bit XOR
sequence (30/32 bytes smaller) and its subsequent MIR span offsets. This is an
intentional backend code-generation change, with no NIR contract/printer change.
