# Terminal pointer forwarding measurements

Follow the [five-slice plan](../../MIR65816_TERMINAL_POINTER_SIZE_PLAN.md).
Baseline: compiler `53dd2af0`, frozen Exec `622b139-dirty`, 631 routines and 120
verified input hashes. The loaded-code-plus-initialized-data estimate without
guards starts at **270,269 bytes**, **8,125 bytes** above 256 KiB.

This is guard subtraction from the frozen guarded image, not a separately
linked guard-disabled release. Full/final backend and hosted Exec qualification
remain deferred. Each slice runs focused 65816 checks and one compile-only
frozen inventory. [measure.py](measure.py) checks unchanged inputs, MIR, ABI,
frames, temporary placements, peaks, initialized data and guard shapes/amounts.

## 1. Incoming pointers into final store addresses

Compiler code shrinks **331,914 → 327,289 B**, saving **4,625 B** across 178
routines, with none larger. All **574 modeled captures** disappear. The loaded
estimate becomes **265,644 B**, leaving **3,500 B** to the cap.
[Summary](01-param-store/summary.json), [routine deltas](01-param-store/routines.csv),
[changed spans](01-param-store/spans.csv).

Ten planner tests, six address integration checks, 22 emission checks and the
unchanged state-boundary snapshot pass. Three pointer runtime tests and two
replay tests pass in debug; all three pointer runtime tests also pass in release. New coverage exercises exact 1/2/3/4-byte stores,
constant/captured payloads, constant/dynamic indexes, bank crossings and neighbor
canaries, raw/optimized LF/CRLF source, flat/two-placement o65 execution, and
IRQ/NMI restoration with same-routine reentry in both task domains. Existing
barrier, escape, geometry and last-fitting incoming-home tests remain passing.

## 2. Incoming pointers into final direct-call arguments

Compiler code shrinks **327,289 → 323,270 B**, saving **4,019 B** across 199
routines, with none larger. All **501 modeled captures** disappear. Cumulative
saving is **8,644 B**; the loaded estimate is **261,625 B (255.5 KiB)**,
**519 B below** the cap. The local-pointer slices continue for additional margin.
[Summary](02-param-call/summary.json), [routine deltas](02-param-call/routines.csv),
[changed spans](02-param-call/spans.csv).

Thirteen pointer planner tests and ten call-copy/push unit tests pass, as do 22
emission checks and the unchanged boundary snapshot. Focused debug execution
passes the new terminal-call test, two call-copy, two call-push, three call-return
and two replay tests. The new test poisons omitted capture slots, verifies they
are unread during argument packing, inspects the complete outgoing area and
checks native returned results. It covers repeated pointers, mixed widths and
padding, both pushes and zero-extension reservation/store fallback, raw/optimized
LF/CRLF and flat/two-placement o65 execution. Existing call-push and call-return
IRQ/NMI reentry probes pass. The planner separately checks last-fitting and
out-of-reach authoritative sources, indirect/width-changing consumers and uses
after calls. ABI, frames, stack peaks, data, guards and frozen inputs match.
The terminal-call and both call-push tests also pass in release.

## 3. Local pointers into final store addresses

Compiler code shrinks **323,270 → 319,274 B**, saving **3,996 B** across 152
routines, with none larger. All **495 modeled captures** disappear. Cumulative
saving is **12,640 B**; the loaded estimate becomes **257,629 B (251.6 KiB)**,
with **4,515 B** of estimated headroom.
[Summary](03-local-store/summary.json), [routine deltas](03-local-store/routines.csv),
[changed spans](03-local-store/spans.csv).

Fourteen planner tests pass, including fresh local windows after reassignment,
escaped sources, intervening writes and later uses. Three pointer runtime tests
and two replay tests pass in debug. The store matrix now includes local sources
for every width and address shape. A conditional local assignment keeps the
local storage present in optimized fixtures, so the test verifies actual local
capture removal rather than a frontend-eliminated local. IRQ/NMI reentry covers
both incoming and local store bases. Source modes, LF/CRLF, exact access traces,
bank crossings, canaries and flat/rebased o65 execution pass. ABI, frame/temporary
placements, stack peaks, data, guards and all frozen inputs remain unchanged.
Six address and 22 emission integration checks and the unchanged boundary
snapshot pass. All three pointer runtime tests also pass in release.
