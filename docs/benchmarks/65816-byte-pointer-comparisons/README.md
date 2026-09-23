# BYTE and pointer comparison baseline

The [implementation plan](../../MIR65816_BYTE_POINTER_COMPARISONS_PLAN.md)
starts from main `c6484952`, with unchanged production emission. New semantic
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
