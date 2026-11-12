# MIR6502 home-value invalidation results

Selective invalidation removes the redundant request copy in the real checked
TN `Alloc`. Its body shrinks **99 → 87 bytes**, its spill count falls **3 → 1**,
and successful `Alloc(1)` takes **223 → 207 cycles**, including the same
17-cycle wrapper. Each complete TN/TNDBG image shrinks **10 bytes**. The
checked library and its arena semantics are unchanged.

The implementation is general MIR6502 behavior, with no routine-name or
address matching. It retains byte equalities only across writes proved disjoint
from both ends. Existing forwarding and dead-spill cleanup consume those facts.
The pre-call save of the incoming request remains necessary and remains emitted.
Both comparison lanes now read the request homes directly.

## Complete images and collateral changes

| Program | Runtime | Baseline XEX | Current XEX | Change |
| --- | --- | ---: | ---: | ---: |
| TN | cart | 10,308 | 10,298 | −10 |
| TN | standalone | 10,433 | 10,423 | −10 |
| TNDBG | cart | 13,403 | 13,393 | −10 |
| TNDBG | standalone | 13,528 | 13,518 | −10 |

`Alloc` saves 12 instruction bytes and two emitted spill bytes. `Init` grows
four instruction bytes: an indexed write with unproved overlap invalidates
source provenance, so its high byte stays staged in zero page. Every other
routine retains its size. Thus the complete saving is 14 − 4 = 10 bytes;
the 12-byte local opportunity is not the net image result. Shared `ArenaCursor`
and `ArenaStop` remain 69 and 6 bytes; generic `Strcat` remains unchanged.

The renamed neutral `CheckRoom` saves the same 12 instruction bytes and two
spill bytes: 101 → 89 bytes. Its complete raw-NIR image shrinks 416 → 402 bytes;
its optimized-NIR image shrinks 425 → 411 bytes. Both mutation controls remain
68 bytes. At the recorded representative input, `CheckRoom` takes 204 → 188
cycles; whole-source and high-byte mutation controls stay at 152 and 164 cycles.

## Storage, accepted rewrites and bounded work

| Program/runtime | Emitted named frame bytes | Declared emitted spills | Virtual ZP allocations |
| --- | ---: | ---: | ---: |
| TN/cart | 234 → 232 | 22 → 20 | 39 → 40 |
| TN/standalone | 236 → 234 | 22 → 20 | 40 → 41 |
| TNDBG/cart | 267 → 265 | 31 → 29 | 43 → 44 |
| TNDBG/standalone | 269 → 267 | 31 → 29 | 44 → 45 |

Frame bytes here count emitted `local_`, `param_` and `spill_` DATA labels,
including helper data; they exclude deferred array backing and absolute aliases.
The extra virtual zero-page allocation reuses the existing physical pool:
the union of allocated bytes stays five for TN and six for TNDBG. Source
zero-page storage stays at the same 13 distinct addresses. Deferred source
initializers remain unchanged; their MIR profiles and placement lines are bound
separately from emitted frame bytes. Neutral reservation lists remain empty.

The maximum retained memory equalities observed during real TN/TNDBG rewriting
is **three**, with **two** in `Alloc`, below the 256-fact limit. Exceeding that
limit discards memory-dependent facts and stops memory learning for the scan.
Unsupported storage and unknown writes stay conservative.

Actual accepted counts in the existing driver families are:

| Program, either runtime | Local consumer forwards | Cross-block forwards | Redundant reload removals |
| --- | ---: | ---: | ---: |
| TN | 3 → 6 | 2 → 3 | 18 → 17 |
| TNDBG | 3 → 6 | 2 → 3 | 22 → 21 |

These are whole-run accepted counts, not estimates or a byte-saving attribution.
The environment keeps the existing forward single-predecessor propagation;
it adds no joins, backedges, cross-call retention or expression analysis.
Ordinary scalar frame objects and canonical alias views supply disjointness.
Raw addresses, globals, initialized views and zero-page overlays supply no new
no-alias proof. Storage views are rebuilt after every rewrite generation.

Host generation observations with opt-level 1 and stripped debug information
were about 8.4–8.7 → 8.6–9.0 seconds for TN and 27.2–27.3 → 28.2–28.4 seconds
for TNDBG. These wall times include concurrent local work and are not a
controlled performance comparison. Deterministic checks exclude wall time and
host executable hashes; compiler sources, images, listings, profiles and cycles
remain bound.

## Qualification

- Full MIR6502 library suite: **1,261 passed**, 12 ignored.
- All 24 affected integration targets: **112 passed**, 8 ignored. Seven MADS
  tests were then selected explicitly and passed, covering 56 LF/CRLF assemblies.
- MIR6502 `--emit-load` sweep: **169 passed**.
- Neutral runtime matrix: **3,384 calls**, raw/optimized NIR, both runtimes,
  two origins and actual LF/CRLF source files, including whole-word and high-byte
  source mutations. Countdown restore controls add 24 calls.
- Frozen and current real TN/TNDBG images each pass **44 Alloc success/fault
  calls**. Return values, stack, fault reasons, cursor writes and arena guards
  are checked. Recorded successful paths save 16 cycles, including exact fit.
- Sixteen consumer VM targets: **36 tests passed**, one historical measurement
  ignored. They include all **1,120** existing checked-arena calls, ABI argument
  setup, pointer consumers, absolute readback, comparisons and TN dispatch/panels.
- All four real images reassemble exactly with LF and CRLF listings at `$2C00`
  and relocate to match fresh direct output at `$3100`: **16 assemblies**.
- Current image bindings and both detailed profile bindings pass their generator
  check commands. Historical checked-arena scorecards remain frozen.

One existing unit test required a literal zero-store restore in a countdown
loop. Conservative invalidation leaves its native increment from `$FF` to zero.
The test now accepts either valid restore; VM cases at starts 0, 1, 3, 127, 128
and 255 verify the final counter and iteration count. No NIR contract or fixture
snapshot changed. Validation was scoped to MIR6502 and its runtime consumers;
unrelated workspace changes were preserved.

See [README.md](README.md) for reproduction and the bound reports.
