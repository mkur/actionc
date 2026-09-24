# Native 24/32-bit returns in A/X

Baseline: compiler `aaf37d51`; frozen Exec `c3500c8`, eight tasks, console,
MyDOS and stack checks. These measurements continue the
[captured BYTE baseline](../65816-captured-byte-returns/README.md).
The [selection contract](../../MIR65816_WIDE_RETURNS.md) defines exact source
extents, register lanes and fallback behavior. Exec's live sources and compiler
pin are unchanged.

| Frozen Exec | Raw | Optimized |
|---|---:|---:|
| Executable bytes before | 425,516 | 392,957 |
| Executable bytes after | 418,476 | 385,834 |
| Executable bytes saved | **7,040** | **7,123** |
| XEX bytes before | 440,904 | 407,735 |
| XEX bytes after | 433,774 | 400,504 |
| Selected wide returns | 281 | 285 |
| Primary return bytes saved | 7,027 | 7,108 |
| Further branch/jump bytes saved | 13 | 15 |
| Guards retained | 2,328 | 2,320 |

| Return source | Raw sites | Optimized sites | Saved bytes/site |
|---|---:|---:|---:|
| 24-bit constant/null | 68 | 70 | 24 |
| 24-bit captured value | 91 | 91 | 21 |
| 32-bit constant | 54 | 79 | 28 |
| 32-bit captured value | 68 | 45 | 29 |

All 551 routine contracts compare equal: frames, temporary homes, arguments,
results and stack peaks. Every guard retains its bytes and order. Bank-zero
reservations are unchanged. The span audit replaces each old scratch-building
return with its exact native encoding and separately accounts for secondary
relaxation. All other instructions compare equal after relocation operands and
local transfer displacements are normalized; labels, spans, fixups and indirect
call resume targets are rebased correctly. Packaged image segments match the
independent inventory build. Generated sources, platform inputs and ABI hashes
match the baseline.

See [totals](exec-results.json), [routine sizes](exec-routines.csv), and
[changed sites](exec-sites.csv). XEX container overhead explains the additional
savings beyond executable code.

Only `wide_shift` changes in the 14-pair corpus: raw code shrinks 331→302 bytes,
optimized 189→160. Its five inputs pass for both compilers, modes and incoming
interrupt-mask states on debug and release hosts. All 20 paired-mask records
agree across hosts. Action cycles fall 667→623 raw and 458→414 optimized.
Each invocation removes four DP byte reads and eight writes; stack traffic,
stack peak, metadata reads and guard costs are unchanged. The other 26 Action
images are byte-identical, so their execution was not repeated. The unchanged
external optimized vbcc `unlink` failure retains its prior status; this focused
run covers `wide_shift`. See [artifact comparison](corpus-comparison.json) and
[execution](corpus-execution.json).

Dijkstra's two Action images and the external benchmark binaries are unchanged.
Action code remains 5,122 raw / 4,535 optimized bytes; prior cycle/traffic results
remain applicable without another execution run. See
[artifact equality](dijkstra-equality.json). Both corpus generators verify
LF/CRLF through actual builds; only documented host file paths are normalized.

Focused runtime coverage checks both complete A/X lanes with independent ca65
callers, direct/indirect calls, recursion, signed bit patterns, bank boundaries,
zero/narrow returns, zero/nonzero frames, mutable/incoming parameters and two o65
placements. Tails match ca65 and read exactly their private source and the RTL
frame, without scratch traffic, tail writes or a fourth-byte read for 24-bit
values. Volatile/alias probes retain exact source traces across callees that
clobber all 64 scratch bytes. LF/CRLF sources compile identically.

Targeted preemption covers 172 raw / 118 optimized task/PC/status sites for
24-bit returns and 140 / 98 for 32-bit returns. IRQ and NMI restore full machine
state and invocation-owned frames at every reached site, including live A/X
results and teardown. Both seeded schedules pass in each mode.

All 220 native unit tests pass (one opt-in test ignored), along with 69 tests
in eight integration/CLI targets. Full native debug and release qualification
passes 194 tests per host (four opt-in tests ignored). All 504 compiler/fixture
input hashes and 1,457 saved artifacts agree across hosts, including focused
return and interrupt artifacts. The full suite covers effects/replay, calls,
helpers, relocations, volatile accesses and stack guard success/failure.

The reviewed emission snapshot changes only two `wide_shift` return tails and
their span ends, saving 29 bytes each. This is an intentional target encoding
change; frame and IR contracts are unchanged. Final check counts, source hashes
and host artifact agreement are in [qualification.json](qualification.json).
