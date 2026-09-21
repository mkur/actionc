# Native 16-bit returns: implementation and qualification

Completed on 2026-09-21 against actionc main. Direct A16 return preparation
reduces identity from **87 bytes / 108 VM cycles to 63 / 71**, and add/subtract
from **98 / 135 to 74 / 98** in both raw and optimized output. All three now
have zero DP scratch traffic. These measurements match the listing-derived
estimates and meet the approved budgets, including guards and RTL.

The [plan](MIR65816_WORD_RETURNS_PLAN.md) is complete: `5010b78` records the
verified baseline and semantic probes; `cac8aeb` implements selection; the commit
containing this report records final qualification. Existing local changes were
preserved throughout.

## Implemented boundary

Only an authoritative `NativeResult(A16)` home enables selection. U8/U16
immediates and exact two-byte stack temps/parameters use the existing checked
word classifier, including mutable parameter homes and complete extents after
transient stack movement. The emitter loads the result into A16 and converges
on the existing frame teardown and RTL. Nonzero-frame teardown preserves A
through Y; a zero frame needs no teardown. X remains unspecified for a word
result, as required by the existing ABI.

Classification errors and unsupported forms add no partial preparation.
Unsupported legal forms retain generic preparation; other result widths retain
their defined high-bit guarantees. The selected preparation uses no DP scratch,
helper, push, or memory write. Existing loads/casts keep memory access ordering
and signed conversion semantics. Arithmetic still stores its result in its
allocated home; return selection reloads it without relying on earlier A contents.

See the [emission contract](MIR65816_EMISSION_CONTRACT.md). Physical ABI v1,
image v3, the experimental o65 profile, stack guards, allocation and Exec816's
compiler pin are unchanged. No SemIR/NIR contract or general allocator changed.

## Measurements

The immutable [post-ADD/SUB snapshot](benchmarks/65816-word-arithmetic/after/tables.md)
is the baseline. Its saved compiler/input/artifact hashes were checked before
implementation; [baseline.json](benchmarks/65816-word-returns/baseline.json)
records verification and the passing pre-change semantic tests.

Representative optimized Action measurements, before → after:

| Kernel / input | Code bytes | VM cycles | DP byte reads + writes | Stack bytes below entry S |
| --- | ---: | ---: | ---: | ---: |
| identity(13) | 87 → 63 | 108 → 71 | 10 → 0 | 4 → 4 |
| add(13,41) | 98 → 74 | 135 → 98 | 10 → 0 | 8 → 8 |
| subtract(13,41) | 98 → 74 | 135 → 98 | 10 → 0 | 8 → 8 |
| constant chain(13) | 95 → 71 | 123 → 86 | 10 → 0 | 6 → 6 |
| maximum(13,41) | 234 → 186 | 210 → 173 | 14 → 4 | 6 → 6 |
| sum loop(13) | 276 → 252 | 2,866 → 2,829 | 66 → 56 | 16 → 16 |
| recursive sum(13) | 333 → 286 | 4,687 → 4,171 | 196 → 56 | 190 → 190 |
| direct calls(13,41) | 377 → 329 | 611 → 500 | 30 → 0 | 20 → 20 |

A selected stack-word return saves 24 bytes, 37 cycles and ten DP byte accesses.
The benefit repeats across calls: recursive sum saves 516 cycles and 140 DP
accesses, while the ordinary sum loop saves 37 cycles at its one return. Wide
shifts and the procedure kernels unlink/forward-copy are unchanged. Stack byte
traffic and pressure remain unchanged; the guard still costs 45 bytes / 32
cycles in identity and add/subtract.

The [full delta](benchmarks/65816-word-returns/delta.md) covers all 14 kernels in
both target modes. Its [checked invariants](benchmarks/65816-word-returns/delta.json)
confirm identical routine storage maps, arguments, results, frame/stack metadata,
observed stack peaks, stack byte traffic and guard costs for every vector, with
no Action code-size or cycle regression. The
[new snapshot](benchmarks/65816-word-returns/after/tables.md) includes CSV results,
provenance, and final identity/add/subtract/loop/unlink listings. Reproduction uses
the [comparison runner](../tools/compare65816/README.md).

The 66 vectors produce 264 records covering both incoming I states: 528 machine
executions per host build. All 264 Action executions pass in each build. All
112 LF/CRLF compilations produce equivalent paired binaries. Debug and release
measurements match exactly. The comparison still fails for the known optimized
vbcc unlink corruption in both I states; all vbcc measurements are identical to
baseline, and the invalid result is retained and excluded from performance claims.

## Validation

The [qualification record](abi/action65816-word-returns-qualification.json)
binds these results to compiler, fixture, VM, tool and artifact hashes:

- 18 native compiler unit tests and 57 ABI/emission/o65/CLI integration tests
  pass. Selector tests cover eligibility, fallback, errors, displacement/delta
  boundaries, unchanged allocation and shared zero/nonzero-frame teardown.
- All **52 native tests pass in debug and release**. The external comparison is
  ignored by default and was executed separately in both builds as above.
- Three word-return tests perform 104 machine runs per build. Independent callers
  check CARD/INT boundaries, casts, mutable parameters, branch returns, recursive
  and typed indirect calls, and mixed result lanes. Volatile traces, aliased
  bank-crossing data, and captured values across full A/X/Y/DP clobbers pass.
  Both newline forms compile identically through the new source probe.
- Executed return-tail probes verify exact source and RTL stack reads, no tail
  writes/DP reads, preserved X within the selected tail, and restored stack/domain
  state. This is an implementation check, not an ABI promise that word callees
  preserve X. Both immediate and stack sources and zero/nonzero frames execute.
- Exhaustive IRQ injection covers 2,332 raw / 2,180 optimized enabled addresses,
  including seven word-return tails / 56 tail boundaries in each mode. Result
  load, Y preservation, stack restoration and RTL are covered. A supplemental
  zero-frame probe interrupts LDA and RTL in each task (four task/PC sites per
  mode) and runs both seeded IRQ/NMI schedules. Existing ADD/SUB, pointer, context
  and stack-fault qualification remains green.
- All seven o65 execution tests pass at their independent placements, including
  preempted tasks. All **136 saved artifacts** match across host builds.

Execution used the qualification runner's pinned VM plus CPU timing correction,
Rust 1.95.0 and ca65/ld65 2.18 on macOS ARM64. Cycles are emulator measurements,
not board timings. Root checks were scoped to native compiler consumers; no full
root suite or NIR sweep was required for this emitter-only slice.
