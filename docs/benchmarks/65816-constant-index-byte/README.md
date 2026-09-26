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
