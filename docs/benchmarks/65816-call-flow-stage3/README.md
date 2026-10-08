# Call flow: native scalar zero tests

Stage 3 admits sole adjacent byte/word Eq/Ne comparisons against literal zero,
including branch and materialized Boolean consumers, with either operand order.
Every admitted consumer ends native output ownership and emits a fresh CMP at
the declared width. Callee and cleanup flags provide no permission. Boolean
results retain their normal capture/branch ownership. The common Call table also
retains the original signature and convention alongside its native ABI.

| Profile | Complete code | Incremental saving | Total saving vs stage 0 | Admitted zero tests |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 428,440 | 1,379 | 1,651 | 692 |
| Optimized guarded | 577,681 | 1,368 | 2,084 | 692 |
| Raw guarded | 647,257 | 1,312 | 1,706 | 654 |

All 692 optimized candidates from the investigation are admitted. Existing
438 optimized/441 raw native Return intervals remain admitted. Raw literal
widening/cast chains and other unsupported shapes keep capture/read fallback.
Different predicates/literals and hidden uses are rejected. Each family trials
its complete allocation against the preceding qualified demand; a refusal cannot
undo an earlier family. No routine grows in code or frame/spill/local peak versus
stage 2 or stage 0. Optimized frame/spill/peak sums each fall a further 26 bytes;
raw extents are unchanged. Complete sequence deltas include fresh CMP and modes.

Validation: 392 emission tests, 35 affected root integration tests and the new
positive/negative admission regression pass. Qualified `call_flow`,
`call_flow_interruptions`, `home_demand` and `replay` pass (11 tests). The assembly
fixture poisons unspecified flags/lanes and checks zero/nonzero extremes, argument
padding, canaries, exact external extent, native state and actual LF/CRLF output.
The interruption probe suspends every reached cleanup/compare/return instruction
in two domains, both interrupt masks, IRQ and NMI, comparing full restored
registers and live private memory to independent uninterrupted CPU execution.
Raw direct-word probes normalize only a verified literal-zero widening in test
MIR; ordinary raw source fallback remains covered separately.

All 441 frozen native vectors retain values, arguments, exact external accesses,
cycles/private traffic and stack peaks. Their corpus does not execute the newly
admitted call/zero forms, so it establishes regression safety. Independent native
fixtures exercise those forms. [Native results](native-results.json),
[provenance](provenance.json), [results](results.json) and compressed tables bind
application and qualification evidence. Serial compiler-cost acceptance and
remaining stage-7 application gates remain open for final qualification.

Use stage-0 reproduction with `target/call-flow-stage3`, stage `3` and the stage-0
reference. Focused native command:

```sh
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test call_flow --test call_flow_interruptions --test home_demand --test replay -j2
```
