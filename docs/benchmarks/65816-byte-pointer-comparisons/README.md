# Native 65816 BYTE and pointer comparisons

Completed on 2026-09-23: native BYTE predicates and three-byte Eq/Ne/null tests,
two-arm Boolean results and adjacent branch fusion. Optimized Exec shell code
shrinks **574,884→502,845 bytes (12.5%)**; XEX shrinks **592,708→519,393 bytes**.
ABI v1, image v3, o65, all frame contracts and stack guards are preserved.
No new DP or bank-zero reservation is introduced. The
[qualification record](../../abi/action65816-byte-pointer-comparisons-qualification.json)
and final results below delimit the measured scope.

The [implementation plan](../../MIR65816_BYTE_POINTER_COMPARISONS_PLAN.md)
froze main `c6484952`, with unchanged production emission. New semantic
probes pass through serialized raw and optimized machine code in the qualified
native VM, both incoming I states and LF/CRLF compilation. Input pairs are
supplied at runtime. Call tests clobber A/X/Y/P and all 64 DP scratch bytes;
volatile record fields provide legal BYTE and three-byte pointer captures.

## Frozen measurements

| Probe | Mode | Code bytes | Cycles | Stack reads/writes | DP scratch reads/writes |
| --- | --- | ---: | ---: | --- | --- |
| BYTE consumer matrix | raw | 2,998 | 3,312 | 112 / 89 | 238 / 239 |
| BYTE consumer matrix | optimized | 2,986 | 3,288 | 109 / 86 | 238 / 239 |
| Pointer consumer matrix | raw | 2,909 | 2,993 | 155 / 154 | 176 / 177 |
| Pointer consumer matrix | optimized | 2,901 | 2,967 | 148 / 150 | 176 / 177 |

Cycles/traffic include the independent caller and all probe routines. BYTE uses
inputs `$80,$7F`; pointer uses `$010000,$000000`. Code includes all routines and
guards. [before.json](before.json) records per-routine sizes/frames and the
qualification manifest with compiler/test/artifact hashes. Full baseline images
and sources are retained under `target/byte-pointer-comparisons/baseline`.

Before implementation, set ceilings of **120 bytes** for the 133-byte BYTE `Ret`
routine and **170 bytes** for the 181-byte pointer `Equal` routine, both modes.
Require each complete matrix to shrink when its selector is implemented, with
unchanged frames/guards and observable memory accesses. Reached fused probes
must eliminate their Boolean-home store/reload and use no comparison scratch.
These are acceptance ceilings, not measured after results.

## Exec inventory

The [Exec baseline](exec-before.json) records the optimized eight-task shell's
574,884 executable bytes and 592,708 XEX bytes. Its pinned `ae1f555` and the
current compiler produced identical output. See the implementation plan for
the exact configuration and the original local artifact directory.

The [verified-MIR inventory](exec-comparisons-before.json) partitions the generic
comparisons independently of the disassembly's result-arm count:

| Form | Raw materialized / sole branch | Optimized materialized / sole branch |
| --- | --- | --- |
| BYTE, all six unsigned relations | 42 / 976 | 42 / 957 |
| Three-byte Eq/Ne | 19 / 346 | 19 / 338 |
| Three-byte ordering and all four-byte relations | 17 / 343 | 17 / 341 |

The optimized generic total is **1,714**, matching the final machine-code
inventory. Of these, 1,356 have a width/predicate admitted by the plan; physical
operand checks still govern selection. The remaining 358 retain fallback.
Sole-branch counts use the existing routine-wide use proof and exact adjacency;
they do not infer eligibility from a nearby conditional opcode. No signed BYTE
comparison occurs in this Exec source.

Reproduce the inventory with the ignored
`mir65816::emit::select::narrow_compare_tests::external_comparison_inventory`
library test. Set `A816_COMPARE_SOURCE` to the generated `kernel-program.act`,
`A816_COMPARE_MODULES` to the path-list of its `task-kernel`, generated output,
Exec `examples` and `lib` directories, and `A816_COMPARE_INVENTORY` to an output
JSON path. The test reports both raw and optimized MIR without changing it.

The small corpus was independently rebuilt using the frozen baseline compiler
with `tools/compare65816/build.py --verify-crlf`; its manifest and bytes are in
`target/byte-pointer-comparisons/corpus-before`. Historical corpus and Dijkstra
snapshots remain unchanged. No hosted Exec execution qualification is claimed
by these compiler baseline tests.

## BYTE slice

The [BYTE qualification and measurements](byte-after.json) record 52 passing
native tests, including existing word comparisons, o65, stack faults and task
preemption. New byte probes exercise 360 reached instruction/status/domain
sites per mode with both IRQ and NMI, plus two seeded schedules. Focused
selector tests check exact byte sequences, all predicates, atomic preflight and
the omitted fused Boolean store/reload; the existing 182 native library tests
also pass before the three new selector checks are added.

The BYTE matrix shrinks by 516 bytes in each mode: 2,998 to 2,482 raw and 2,986
to 2,470 optimized. Its measured execution drops by 331 cycles. `Ret` falls
from 133 to 113 bytes with its four-byte frame intact. Pointer probes retain
their original sizes and execution measurements in this slice.

On the fixed Exec `8e1ff57` workload, executable segments fall from **574,884 to
529,972 bytes**, a **44,912-byte reduction**. All 547 routine contracts/homes
and all 2,279 guards remain unchanged. The live Exec checkout advanced to
`d788d44` while the first measurement ran; that mixed-input result was excluded.
An isolated `8e1ff57` worktree reproduced the original baseline XEX exactly
before producing this comparison. No Exec pin or live source was changed.

## Pointer slice

The [pointer qualification and measurements](pointer-after.json) record 34
passing native tests covering pointer comparisons, control flow, state tracking,
preemption and stack faults. The pointer interrupt sweep restores full state at
630 reached instruction/status/domain sites per mode, separately with IRQ and
NMI, including low-word mismatch and bank-byte decisions. Relocated o65 probes
execute at two placements. Exact private-frame traces check that no fourth
pointer byte is read, with no comparison scratch traffic.

The pointer matrix falls from 2,909 to 2,204 bytes raw and 2,901 to 2,192 optimized.
`Equal` shrinks from 181 to 135 bytes with its eight-byte frame intact. Measured
cycles fall from 2,993 to 2,854 raw and 2,967 to 2,825 optimized. Stack reads
increase from 155 to 182 raw (148 to 175 optimized): the low-word-first comparison
reads both private low words before the bank mismatch, while the former generic
comparison could stop on the high byte. Writes fall by four and DP reads/writes
by sixteen in each mode. Observable source accesses remain unchanged.

The optimized frozen Exec shell now contains **502,845 executable bytes**, an
additional **27,127-byte reduction** after BYTE selection, or **72,039 bytes
(12.5%)** against baseline. XEX size is 519,393 bytes. All 547 routine contracts
and all 2,279 guards still match baseline. `EXECLISTS.IsListEmpty` falls from 262
to 220 bytes; `SHELLAPP.ShellParse` falls from 7,793 to 5,539 bytes. No frame or
DP/bank-zero reservation changes accompany these reductions.

## Final execution and size results

The full native suite passes **142 tests in debug and 142 in release**, with
identical qualification input hashes and all emitted artifact hashes equal.
The root checks pass **187 MIR65816 unit tests** (state-proof feature enabled)
and **61 integration/CLI/o65 tests**. Four manual artifact/inventory native tests
and one external root inventory test remain ignored in the ordinary suites;
the corpus and Dijkstra execution tests are invoked separately with their inputs.
LF/CRLF is exercised through actual probe, corpus and Dijkstra compilation.
No unrelated backend suite was run.

The [final probe counters](final-probes.json) agree between host profiles:

| Probe | Mode | Code before → after | Cycles before → after | Stack reads/writes after | DP reads/writes after |
| --- | --- | ---: | ---: | --- | --- |
| BYTE matrix | raw | 2,998 → 2,482 | 3,312 → 2,981 | 106 / 83 | 221 / 222 |
| BYTE matrix | optimized | 2,986 → 2,470 | 3,288 → 2,957 | 103 / 80 | 221 / 222 |
| Pointer matrix | raw | 2,909 → 2,204 | 2,993 → 2,854 | 182 / 150 | 160 / 161 |
| Pointer matrix | optimized | 2,901 → 2,192 | 2,967 → 2,825 | 175 / 146 | 160 / 161 |

All four probe images retain their four routine contracts and seven exact guard
sequences. The pointer read increase is the deliberate low-word-first tradeoff
described above, restricted to captured private values.

### Exec images

All six images use the frozen source/configuration described above. Generated
`kernel-program.act` hashes match for every before/after pair. The
[image/module measurements](exec-final.json) retain compiler overrides, build
and image hashes, guard counts and bank-zero budgets; the
[routine table](exec-routine-sizes.csv) compares all 2,038 routine/mode/profile
records. No routine grows. All frame/home/ABI metadata match apart from linked
addresses and code size.

| Profile | Mode | Executable before → after | Saved | XEX before → after | Guards, unchanged |
| --- | --- | ---: | ---: | ---: | ---: |
| Core | raw | 202,512 → 177,249 | 25,263 | 212,557 → 186,864 | 776 |
| Core | optimized | 188,425 → 164,446 | 23,979 | 198,220 → 173,827 | 768 |
| Console | raw | 274,727 → 239,400 | 35,327 | 286,390 → 250,435 | 1,029 |
| Console | optimized | 257,279 → 223,222 | 34,057 | 268,602 → 233,969 | 1,021 |
| Shell | raw | 607,177 → 533,672 | 73,505 | 625,593 → 550,794 | 2,287 |
| Shell | optimized | 574,884 → 502,845 | 72,039 | 592,708 → 519,393 | 2,279 |

The final shell's generic three-arm materializations count 1,714→715→358 across
baseline/BYTE/pointer stages. Exact result-arm encodings and their common final
target were checked in emitted executable segments; this agrees with removal of
all 999 BYTE and 357 pointer Eq/Ne sites in the MIR inventory. Four-byte
comparisons (344) and three-byte ordering (14) retain fallback. Reduced enclosing
blocks can also let the existing conditional dispatcher select its short form;
this does not extend branch-relaxation policy. Code-bank relocation addresses
move without changing physical storage contracts.

These are compiler builds and compiler-VM qualification. Hosted Exec task/IRQ,
console and filesystem acceptance was not rerun, so this is not a hosted Exec
qualification or a compiler-pin update.

### Small corpus and Dijkstra

The [28 Action corpus image comparisons](corpus-size-delta.json) are all exactly
equal to the independently rebuilt baseline. Existing native word paths,
32-bit fallback, copies, calls and pointer-transfer kernels retain their bytes.
All 264 comparison records agree between debug and release, each checking both
incoming I states. All 132 Action records pass. The unchanged known vbcc
optimized `unlink` vector-0 corruption remains visible in the
[compact results](corpus-results.csv) and [check record](corpus-checks.json);
both complete comparison commands return 101 for this existing failure.

Dijkstra's [whole-module sizes](dijkstra-sizes.json) fall 6,787→6,365 raw and
6,197→5,779 optimized. Its independently rebuilt baseline listings exactly match
the immutable [earlier comparison](../65816-dijkstra/README.md). The
[per-routine table](dijkstra-routine-sizes.csv) attributes optimized savings to
`Enqueue` (154 bytes), `Dequeue` (80) and `Main` (184). `Find` stays at 2,132
bytes: its signed ordering/address-generation work remains outside this slice.
Frames, all 22 guards and vbcc's 1,697/1,477 code bytes remain unchanged.

The full 33-case Dijkstra corpus passes for both compilers and modes in the
qualified release VM, with both incoming I states: **132 records / 264
executions**, no errors. The [execution record](dijkstra-checks.json),
[all cases](dijkstra-results.csv) and [routine profile](dijkstra-routine-profile.csv)
retain the evidence. Original-benchmark Action counters are:

| Mode | Cycles before → after | Stack reads+writes before → after | DP reads+writes before → after | Peak stack, unchanged |
| --- | ---: | ---: | ---: | ---: |
| raw | 2,038,730,702 → 1,971,316,896 | 243,671,581 → 241,793,991 | 351,381,506 → 347,596,380 | 96 |
| optimized | 1,908,568,205 → 1,841,244,249 | 178,702,872 → 176,825,282 | 351,321,586 → 347,536,460 | 86 |

Optimized Dijkstra improves 6.7% in code size and 3.5% in cycles. Its optimized
code remains 3.91× vbcc's 1,477 bytes; the deferred size backlog remains relevant.

Large sources, images, full listings, logs and instruction-PC measurements remain
under `target/byte-pointer-comparisons/`; compact facts and artifact hashes are
committed here. Reproduce the corpus with `tools/compare65816/build.py
--verify-crlf`, then the explicit `code_quality` test in both profiles; reproduce
Dijkstra with `dijkstra.py --verify-crlf` and `run_dijkstra.py`. Freeze the built
compiler executable before an authenticated run so concurrent Cargo builds
cannot replace the binary whose hash the manifest records.
