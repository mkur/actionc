# Native LONGCARD/LONGINT equality

MIR65816 selects Eq/Ne over captured four-byte stack homes and U32 constants
as two A16 word decisions. It compares the low words first and reads the high
words only when needed. Zero on either side uses the loaded words' Z flags
without CMP-zero. Signedness does not change bit equality.

Value consumers receive one canonical BYTE 0/1. An adjacent, sole-use Branch
consumes the decisions directly through the existing edge-copy machinery.
Full operand/result preflight precedes emission. Source loads still capture all
four bytes in order; only private-home reads may short-circuit. Long ordering
retains its existing path. See the [plan](../../MIR65816_LONG_EQUALITY_PLAN.md)
and [emission contract](../../MIR65816_EMISSION_CONTRACT.md).

## Frozen Exec measurement

The baseline is compiler `26146332`, including the completed
[BYTE-return slice](../65816-byte-returns/README.md). The workload remains Exec
`c3500c8`, eight tasks, console, MyDOS and mandatory stack checks enabled.

| Measurement | Raw before → after | Optimized before → after |
| --- | ---: | ---: |
| Compiler routine bytes | 475,095 → 452,593 | 443,153 → 419,789 |
| Selected comparisons | 231 | 228 |
| Fused / materialized sites | 218 / 13 | 215 / 13 |
| Code bytes saved | **22,502** | **23,364** |
| All executable bytes | 482,660 → 460,158 | 450,718 → 427,354 |
| XEX file bytes | 499,090 → 476,140 | 466,556 → 442,778 |
| Guards retained | 2,328 | 2,320 |

The optimized saving is **5.18%** of the previous executable size. Comparison
and fused-branch replacements account for 22,490 raw / 23,352 optimized bytes.
Both modes also omit six now-redundant `SEP #$20` instructions after materialized
results, saving 12 bytes through existing mode tracking. No dispatch outside
those replacements changes size. XEX totals additionally reflect packaging.

All 551 routine contracts match in each mode: frames, homes, arguments, results,
calls and stack costs. The audit accounts for every instruction outside the
selected replacements, including the six mode omissions, relocation rebasing,
labels, fixup targets, MIR spans, short branches and PER continuations. Guard
counts, amounts and order are unchanged. Full packaged compiler segments match
the inventory images, with identical generated sources, platform/ABI inputs and
bank-zero budgets. Added stack, DP and bank-zero reservation: **0 bytes**.
See [totals and hashes](exec-results.json), [routine sizes](exec-routines.csv)
and [individual sites](exec-sites.csv).

The 28 small-corpus Action images and both Dijkstra images are byte-identical
to the BYTE-return baseline. Dijkstra remains 5,438 raw / 4,850 optimized code
bytes. Actual LF/CRLF builds agree. See [corpus sizes](corpus-sizes.json) and
[Dijkstra sizes](dijkstra-sizes.json). Their unchanged binaries retain previous
execution measurements; the external vbcc execution comparison was not rerun.

## Execution and qualification

Before/after probes use the same source, with raw and optimized compilation,
both long types and both incoming I states. They cover boundary cross products,
all 32 individual bits, deterministic random pairs, zero on either side,
nonzero constants, mutable parameters and returned/passed/reused Booleans.
Focused tests also cover volatile and bank-crossing loads, pointer aliases,
direct/indirect calls clobbering A/X/Y and all 64 DP scratch bytes, nonempty
parallel edges, loop backedges and two relocated o65 placements. Independent
ca65 encoding and execution checks verify complete materialized spans, exact
private word reads, one BYTE result store and no comparison DP traffic.

For the recorded input `a=$00010000, b=0, I=0`, the full probe measurements are:

| Type / mode | Code bytes before → after | Cycles before → after |
| --- | ---: | ---: |
| LONGCARD raw | 4,189 → 3,013 | 4,457 → 4,095 |
| LONGCARD optimized | 4,085 → 2,867 | 4,237 → 3,813 |
| LONGINT raw | 4,257 → 3,021 | 4,528 → 4,103 |
| LONGINT optimized | 4,153 → 2,875 | 4,308 → 3,821 |

Each returned Eq/Ne routine shrinks from 185 bytes for LONGCARD or 189 for
LONGINT to 115 bytes, retaining its ten-byte frame. For this input, low-word-first
word access increases private stack reads: 266→316 raw and 241→278 optimized.
Stack writes fall by four and DP reads/writes by 31 each. Observable source
reads are unchanged. Timing is input-dependent; these are representative probe
measurements, not a universal cycle saving. See [paired metrics](runtime-metrics.json).

IRQ/NMI probes check full restored state at every reached task/PC/status site
in the selected equality and branch routines, plus both seeded schedules.
Each type covers 660 raw / 634 optimized sites across both task domains,
including equal, low-mismatch, high-mismatch and zero decisions. Actual LF/CRLF
instrumentation produces identical source.

Native checks pass: **201 library tests** (one opt-in test ignored), **69 root
integration/CLI checks**, and **178 runtime tests in each of debug and release**
(four external inventory/comparison tests ignored in each). All **493 input
hashes and 917 artifact hashes** match across the runtime runs. The
[qualification record](qualification.json) binds the checked source, measurement
tools, compiler binaries, pinned VM/patch and run manifests. The existing
emission snapshot is unchanged.

```sh
cargo test --lib mir65816::
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission --test mir65816_state_boundary --test mir65816_o65 --test mir65816_arithmetic --test actionc_65816_cli --test actionc_65816_o65_cli
python3 tools/native65816-runtime-tests/qualify.py --test long_equality
python3 tools/native65816-runtime-tests/qualify.py --test preemption long_equality
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

Local measurement scripts and full inventories are retained under
`target/long-equality/`. Exec builds use the existing audit adapter, which asserts
`stack_checks:true` and removes only that obsolete layout field; the compiler
emits mandatory guards. Live Exec sources and its compiler pin are unchanged.
Hosted Exec adoption still requires its own qualification. This slice changes
no NIR, semantic, printer, public ABI or other backend contract.
