# Direct 24-bit loads into local storage

An adjacent three-byte load/copy into an unexposed local now loads into the
local's final home. The intermediate temporary is omitted before allocation;
the source read keeps its original site, width and order. Eligibility reuses
the existing private-local ownership proof and requires one definition and one
use. Volatile/indexed accesses, escaping locals and nonadjacent/multiple
consumers retain captures. Closed DP allocation profiles retain their schedules.
See the [emission contract](../../MIR65816_EMISSION_CONTRACT.md).

Baseline: actionc `b9b35d30`, Exec `134130b`, with the same 133 generated
source/layout inputs as the [base-preservation measurement](../65816-base-preservation/README.md).

| Measurement | Before | After |
| --- | ---: | ---: |
| Exec routine code, 960 routines | 472,442 bytes | 472,114 bytes |
| AddHead code | 154 bytes | 146 bytes |
| AddHead frame / local peak | 8 / 8 bytes | 4 / 4 bytes |
| AddHead temporary spill storage | 4 bytes | 0 bytes |

**328 bytes saved across 34 routines; none grows.** AddHead and AddTail have
smaller frames; no frame or local peak grows. All 960 frame maps pass Exec's
validator. The [AddHead listing](addhead.asm) shows `LDA [$80] / STA $01,S`
followed by the bank-byte load and `STA $03,S`, with no intermediate copy.
The [routine table](exec-routines.csv) records code, frame and stack-peak changes;
[provenance](provenance.json) records source/compiler/input/image hashes.

Reserved bank-zero delta: **0 fixed bytes and 0 bytes per task**, including
guards, alignment and unused capacity. Individual calls need less stack, but
task-stack reservations, DP scratch and interrupt headroom are unchanged.
Counts exclude data, alignment and container costs. This is a compile-only
local compiler override, not hosted-system qualification; the compiler pin
and play image remain unchanged.

Validation: 338 MIR65816 unit cases pass across the backend batch and a focused
fixture follow-up (one existing ignored); 39 emission/o65/state-boundary
integration tests pass; 17 native cases pass across `address_consumers`,
`pointer_forwarding` and `stack_checks`. Coverage includes raw/optimized NIR,
exact source-read/write traces, overlapping list nodes and the empty-list
sentinel, bank crossings, fixed and two relocated placements, source mutation,
pointer/ADDRESS/SIZE snapshots, volatile/escaping/multiple-use fallbacks,
stack guards and IRQ/NMI/task reentry. New test fixtures were corrected to
retain the local through NIR optimization and use supported source syntax.

Reproduce using the compile command and module paths from the
[address-consumer measurement](../65816-address-consumers/README.md), with
`/tmp/exec816-local-loads.a816.json` as output. The listing uses the existing
decoder with an in-memory version-3 envelope; version-4 arithmetic-fault
metadata does not change instruction encoding.
