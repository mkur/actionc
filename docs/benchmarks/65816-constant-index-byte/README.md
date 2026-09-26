# Constant-index, BYTE-consumer and immediate-push measurements

Follow the [five-slice plan](../../MIR65816_CONSTANT_INDEX_BYTE_SIZE_PLAN.md).
Baseline: compiler `a25fa91d`, frozen Exec `622b139-dirty`, 631 routines and 120
verified input hashes. Loaded code plus initialized data excluding guards is
estimated at **282,138 bytes**, a **19,994-byte** gap to 256 KiB.

The estimate subtracts guard ranges from a frozen guarded build; it is not a
separately linked release. Full/final backend and hosted Exec qualification are
deferred. Each slice uses focused 65816 checks and the same frozen inventory
probe. [measure.py](measure.py) verifies unchanged inputs, ABI, frames, local
stack peaks, initialized data and guard amounts, and retains routine/span deltas.

## 1. Constant indexes through captured pointers

Code shrinks **343,783 → 339,440 B**, saving **4,343 B** in 33 routines, with
none larger. The loaded-size estimate becomes **277,795 B**, leaving **15,651 B**
to the cap. All input hashes, guards, frames, peaks, ABI and data are unchanged.
[Summary](01-constant/summary.json), [routines](01-constant/routines.csv),
[changed spans](01-constant/spans.csv).

Six address integration checks, 22 emission checks and the unchanged boundary
snapshot pass. Seven indexed runtime tests and two replay tests pass in debug;
the two new constant-index tests also pass in release. Coverage includes exact
1/2/3/4-byte traffic, poison neighbors, zero/scaled indexes, last-fitting Y offsets
and one-byte overflow fallback, bank/bus crossing, raw/optimized LF/CRLF paths,
flat/two-placement o65 execution, and IRQ/NMI at reached instructions with
same-routine reentry in both task domains and interrupt-mask states.

## 2. Equality zero tests

Code shrinks **339,440 → 337,895 B**, saving **1,545 B** in 310 routines, with
none larger. The estimate becomes **276,250 B**, leaving **14,106 B** to the cap.
All frozen inputs and layout-independent ABI/frame/peak/data/guard facts match.
[Summary](02-zero/summary.json), [routines](02-zero/routines.csv),
[changed spans](02-zero/spans.csv).

Focused emitter tests pass after updating the exact word-comparison expectation;
22 emission checks and the reviewed boundary snapshot pass. The snapshot only
removes CMP #0 in optimized sum_loop, forward_copy and recursive_sum and remaps
later positions. Four BYTE, five word, four accumulator-forwarding and two replay
runtime tests pass, plus exhaustive reached-window IRQ/NMI zero-test coverage.
The new runtime matrix includes both operand orders, signed/unsigned words,
materialized/fused results, raw/optimized LF/CRLF and flat/rebased o65 execution.
The test-only forwarding adapter now recognizes typed zero-test branches and
accounts separately for previously excluded shared-return sites in historical
counts; its physical A/home/NZ checks remain intact.
The new zero-test matrix and BYTE/word reentry tests also pass in release, along
with the existing LONG zero-test reentry regression. No full qualification ran.

## 3. Adjacent BYTE load/comparison consumers

Code shrinks **337,895 → 335,421 B**, saving **2,474 B**. The cumulative saving is
**8,362 B**. The estimate becomes **273,776 B**, leaving **11,632 B** to the cap.
[Summary](03-byte/summary.json), [routines](03-byte/routines.csv),
[changed spans](03-byte/spans.csv). Frames, stack peaks, guards, ABI, data and
all frozen input hashes remain unchanged; no routine grows.

All 269 active emitter tests pass (one existing ignored), including refusal of
volatile, multi-use, ordering and unsupported operands. Six address integration
checks, 22 emission checks and the unchanged boundary snapshot pass. The new
runtime test checks direct-indirect, constant-index and dynamic-index paths,
volatile fallback, exact ordered reads and neighboring canaries, both operand
orders, raw/optimized LF/CRLF input and flat/two-placement o65 execution. Four
BYTE comparison, one home-definition and two replay runtime checks also pass.
Reached-window BYTE/word zero-test IRQ/NMI reentry passes in debug.
The new consumer runtime test and four BYTE return/comparison/zero-test reentry
regressions also pass in release. Full qualification remains deferred.

## 4. Advance Y within wide indirect accesses

Code shrinks **335,421 → 333,712 B**, saving **1,709 B**. The estimate becomes
**272,067 B**, leaving **9,923 B** to the cap. The cumulative saving is **10,071 B**.
Each replaced second LDY saves one byte and adds one cycle. Zero-offset starts
remain eligible for the existing LDY-zero elimination.
[Summary](04-y/summary.json), [routines](04-y/routines.csv),
[changed spans](04-y/spans.csv).

All 270 active emitter tests pass (one existing ignored), including exact second
pieces, zero offsets, volatile fallback and upper Y bounds. Six address and 22
emission checks and the unchanged boundary snapshot pass. Seven indexed, eight
memory, two pointer-preemption and two replay tests pass in debug. The two new
constant-index boundary/reentry tests plus eight memory and two pointer-preemption
tests pass in release. This covers bank/Y boundaries, exact three-byte traffic,
constant stores, canaries, flat/o65 placement and IRQ/NMI restoration. Frames,
peaks, ABI, guards, data and input hashes remain unchanged; no routine grows.

## 5. Immediate word argument pushes

Code shrinks **333,712 → 331,914 B**, saving **1,798 B** in 206 routines, with
none larger. PEA's mode independence and packing across adjacent known
argument/padding bytes exceed the simple 533-byte load/PHA cohort model.
[Summary](05-pea/summary.json), [routines](05-pea/routines.csv),
[changed spans](05-pea/spans.csv).

All 270 active emitter tests pass (one existing ignored). The independent stack
oracle covers 1,024 mixed-width layouts, exact descending writes, dynamic source
displacements and final A16; invalid stack-phase pushes are rejected atomically.
All 22 emission and 11 o65 integration checks and the unchanged boundary snapshot
pass. Debug runtime checks pass for two call-copy, four padding, two push, eleven
state, three physical-effect and two replay tests. Independent ca65/VM checks
verify PEA encoding, high/low write order, both M/X widths, all unwritten bits,
flags, stack depth and canaries. Call tests check guard faults before any payload
write, maximal outgoing areas, symbolic byte fixups, rebasing, and IRQ/NMI reentry
at every reached construction instruction in both task domains and I states.
The immediate interrupt cases explicitly supply verifier-clean numeric operands
where raw frontend casts would otherwise retain captures.
All six affected runtime targets also pass in release: 24 tests covering calls,
padding, pushes, state, physical effects and replay. Full qualification was not run.

## Completed series

| Slice | Saving |
|---|---:|
| Constant indexes through captured pointers | 4,343 B |
| BYTE/word equality zero tests | 1,545 B |
| Adjacent BYTE load/comparison consumers | 2,474 B |
| Y advancement within wide indirect transfers | 1,709 B |
| Immediate word argument pushes | 1,798 B |
| **Total** | **11,869 B (11.6 KiB)** |

Compiler code is **331,914 B**, or **259,662 B** after subtracting the unchanged
72,252 guard bytes. Adding 8,300 B of package assembly and 2,307 B of initialized
data gives **270,269 B (263.9 KiB)**, leaving **8,125 B (7.9 KiB)** to 256 KiB.
The 951 compiler data bytes are already included in initialized data. No routine
grows in any slice; frames, local stack peaks, ABI, guards and data are unchanged.
This remains a guard-subtracted estimate, not a separately linked release.

[Completed summary](completed-summary.json) is generated by [summarize.py](summarize.py).
The [frozen input manifest](../65816-epilogue-index-casts/frozen-inputs.json)
identifies the same 120 files checked at every measurement. Bulky image/inventory
and runtime artifacts remain under ignored target directories. Full/final
backend and hosted Exec qualification remain deferred.
