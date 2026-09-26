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
