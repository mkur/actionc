# Native 65816 outgoing argument padding

Completed on 2026-09-23. Native calls now clear only alignment gaps and tail
padding before the existing argument copies. Each outgoing byte receives one
write. The frozen Exec shell loses **13,756 optimized executable bytes (2.69%)**
with all 2,320 guards retained.

The [implementation plan](../../MIR65816_CALL_PADDING_PLAN.md) was completed in
three slices: baseline `ea422df3`, selector and focused tests `835f0cb0`, and
this qualification/report slice. ABI v1, image v3, o65, frame/home allocation,
stack extents, cleanup, results and interrupt contracts are unchanged. Added
stack, DP and bank-zero reservations: **0 bytes**.

## Implementation and observations

The private padding helper in the
[call selector](../../../src/mir65816/emit/select.rs) walks verified argument
homes, checks their ranges and returns checked one-based stack displacements.
It runs before any call emission. The existing A8 request and zero load remain,
including calls without padding; only redundant `STA d,S` instructions disappear.
No optimizer pass, IR form, allocation policy or feature flag was added.

For `(BYTE, CARD, BYTE POINTER, LONGINT)`, payload occupies ten of thirteen
outgoing bytes. Only displacements 2, 8 and 13 are cleared. The other ten bytes
receive their existing argument writes, saving 20 code bytes and 40 cycles per
execution. The [baseline](baseline.json), [selected output](selection.json) and
[68 execution deltas](probe-deltas.json) record actual bus writes, including
zero-valued payloads, in raw/optimized modes with either incoming I state.

Independent assembly callees inspect the complete incoming area and clobber
all 64 DP scratch bytes and caller-saved registers. Coverage includes direct
and indirect calls, holes without a tail, adjacent three-byte pointers,
captured locals, mutable parameters, nested calls and volatile alias reads.
No-argument calls retain their mandatory one-byte clear. The largest accepted
source signature tested has 252 payload bytes and one tail byte: the final
incoming payload displacement is 255. A separate helper test checks outgoing
padding displacement 255; existing source and indirect-target limits remain.

## Frozen Exec comparison

The workload is the previous audit's Exec `c3500c8`, eight-task shell with
console, MyDOS and stack checks enabled. Compiler production sources before
the change equal `9f16e08b`; the measured implementation is `835f0cb0`.
Baseline hashes were revalidated. The same audit-only adapter asserts
`stack_checks:true` and removes that unsupported layout field for main, which
always emits guards. Live Exec sources and its `2d73c03a` pin were untouched.

| Measurement | Raw before → after | Optimized before → after |
| --- | ---: | ---: |
| Compiler routine bytes | 534,815 → 520,969 | 503,453 → 489,697 |
| All executable bytes | 542,380 → 528,534 | 511,018 → 497,262 |
| Executable bytes saved | 13,846 | 13,756 |
| XEX file bytes | 559,888 → 545,790 | 527,916 → 513,926 |
| Guards, unchanged | 2,328 | 2,320 |

Every call shrinks by exactly twice its argument payload width; every non-call
MIR span keeps its length. There are **zero secondary layout savings**. All 551
routine contracts, frames and homes, guard amounts/order and platform/ABI
inputs match in each mode. Generated sources match after build-directory
normalization. Runtime bank-zero budgets remain 24,672 bytes excluding OS and
61,536 including OS. XEX deltas include the resulting packing/metadata change.

See [totals and hashes](exec-results.json),
[all 3,546 call sites](exec-call-sites.csv) and
[per-routine deltas](exec-routines.csv). These are build measurements;
hosted Exec boot qualification and compiler-pin integration remain separate.

## Corpus and Dijkstra

Only `direct_calls` and `recursive_sum` change in the small corpus: eight and
four bytes saved respectively, in each mode. The other 24 Action images are
byte-identical. `direct_calls` falls from 436 to 420 cycles, with stack writes
26 → 22 and peak 14 unchanged. Recursive vector 3 saves 104 cycles and 26 writes
in each mode; its stack peak remains 190. DP traffic and stack reads/peaks are
unchanged across all records. See [image sizes](corpus-sizes.json),
[routine sizes](corpus-routines.csv), [execution deltas](corpus-execution.csv)
and [execution status](corpus-execution.json).

Dijkstra retains all 22 guards and saves 88 code bytes in each mode:

| Measurement | Raw before → after | Optimized before → after |
| --- | ---: | ---: |
| Executable bytes | 5,878 → 5,790 | 5,290 → 5,202 |
| Original benchmark cycles | 1,817,441,150 → 1,816,542,330 | 1,690,355,361 → 1,689,456,541 |
| Original benchmark stack writes | 106,583,391 → 106,358,686 | 68,099,075 → 67,874,370 |
| Stack peak, unchanged | 96 | 86 |

All 33 Dijkstra cases pass: 132 records and 264 executions including both
incoming I states. Each removed stack write saves four cycles. Stack reads,
DP traffic, guard cycles and stack peaks stay equal; vbcc bytes/counters are
unchanged. See [sizes](dijkstra-sizes.json),
[routine deltas](dijkstra-routines.csv),
[execution deltas](dijkstra-execution.csv) and
[execution attestation](dijkstra-execution.json).

## Qualification and review notes

The [qualification manifest](../../abi/action65816-call-padding-qualification.json)
records compiler/fixture hashes, VM revision/patch, artifacts and run manifests.
Validation followed the plan's backend-scoped commands with incremental
compilation and development/test debug info disabled:

- Native unit tests: **195 passed**, one ignored.
- Seven native root integration/CLI/o65 targets: **63 passed** after the
  reviewed emission snapshot update.
- Full native runtime suite: **155 passed, four ignored** in both debug and
  release; all 482 compiler/fixture inputs and 794 generated artifacts match.
- IRQ/NMI: **1,016 injections** across every reached boundary of the direct and
  indirect call spans, in both modes and task domains. Immediate register and
  outgoing/frame restoration and full task completion pass.
- Mixed calls in relocated o65 execute at both existing placements, with moved
  data arguments and imported indirect targets. Exact-floor success and
  floor/ceiling/underflow faults preserve guard behavior and failure A/X/S.
- An isolated CRLF checkout rebuilds affected embedded fixtures: **19 native
  tests plus one root snapshot test pass**, with 186 artifacts identical to LF.
  Corpus and Dijkstra generators also verify actual LF/CRLF build equality.
- Small-corpus execution records agree between debug/release: all **132 Action
  records pass** in each. Both comparison commands retain exit 101 because the
  existing optimized vbcc `unlink` vector 0 fails at `$12ffff` (expected `$00`,
  observed `$fc`, store PC `$010030`). The failure is unchanged and not waived.

The state-boundary fixture intentionally changes emitted bytes and rebases
labels/fixups/spans in four routine records: accumulator forwarding `Main`
436 → 420 raw / 432 → 416 optimized, and recursive sum `Work` 206 → 202 raw /
191 → 187 optimized. The removed bytes are exactly payload clear stores;
all frames and other routine records are unchanged. This is an emission
contract update, with no NIR or printer change.

Retained local artifacts are under `target/call-padding/`, and frozen Exec
builds under `exec816/build/code-size-detail-20260923/exec/build/` in
`shell-call-padding-raw` and `shell-call-padding-opt`. The CRLF run manifest is
preserved as `target/call-padding/crlf-manifest.json`; its temporary checkout
was removed after validation. No 6502/68k suite or repository-wide NIR sweep
was required for this native selector change.
