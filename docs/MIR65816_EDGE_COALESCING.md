# Compatible native edge-home coalescing

Implementation `12d13e1` coalesces compatible private word sources with existing
successor block-parameter homes and omits direct physical self-copies. It follows
the [committed plan](MIR65816_EDGE_COALESCING_PLAN.md). Test follow-up `7b5a2a7`
keeps the coalescing preemption artifacts separate from ordinary frame probes.
Public ABI v1, image v3, o65 profile v1, stack guards and interrupt reserves remain.

## Allocation and emission contract

The existing first-fit allocation and complete frame verification run first.
One deterministic pass considers directly scheduled word edges and proposes
all compatible source-home changes for an edge together. Source temps must be
ordinary operation results; every block parameter stays at its original home.
Conflicting repeated-source proposals reject the transaction. The unchanged
closed-operation interference graph remains authoritative.

Every trial must pass full stack verification, including third-party conflicts,
successor live-ins, unused parameters, incoming bounds and exact frame/staging
accounting. It must reduce copy cost without increasing any edge's bytes or
cycles. A branch-arm regression test confirms that a cheaper arm cannot make
the other arm more expensive. Rejected trials leave the allocation intact.
There is no new frame compaction, staging reduction or DP/register allocation.

All logical operands still pass preflight. Direct full-word self-copies omit
LDA/STA pairs; other assignments retain their safe schedule. If the last emitted
assignment is not the original final logical assignment, a final LDA restores
its full A/N/Z. Thus a single self-copy or all-self edge still executes a load,
with no self store. Selective and complete staging retain their existing paths.
Value/flag permissions do not survive successor labels. See the
[emission](MIR65816_EMISSION_CONTRACT.md) and
[allocation](MIR65816_TEMPORARY_ALLOCATION.md) contracts.

## Measured result

The [saved comparison](benchmarks/65816-edge-coalescing/after/tables.md) and
[exact delta](benchmarks/65816-edge-coalescing/delta.json) confirm the frozen
forecast. Only optimized rotation changes; all 27 other Action builds and all
vbcc outputs retain their code. For input 13:

| Measurement | Before | After |
| --- | ---: | ---: |
| Code bytes | 138 | 130 |
| Cycles | 916 | 896 |
| Instructions | 222 | 218 |
| Stack-byte reads | 127 | 123 |
| Stack-byte writes | 140 | 136 |
| Fixed frame / observed peak | 16 | 16 |

`t0` moves from S+$06 to `t16`'s S+$0A; `t2` moves from S+$08 to `t17`'s S+$06.
Their producer captures remain, with changed operands. Both initialization copies
become identities and vanish; the final immediate assignment still sets A/N/Z.
The cyclic backedge retains its one staging word. Across six vectors per incoming
I state, 12 coalesced copies save 24 instructions, 120 cycles, 24 stack-byte reads
and 24 writes. Static savings are eight bytes in one build.

Optimized `sum_loop(13)` remains 120 bytes / 1,212 cycles / 12 stack bytes. Existing
logical edge, temp/frame/parameter forwarding counts, guard costs, DP traffic,
frames and results remain. New Action-only `coalesced_word_copies` and
`coalesced_word_copy_sites` report omitted assignments; existing word counters
continue to describe logical edge arity. The known optimized vbcc `unlink`
vector-0 failure remains visible.

## Qualification

The [exact checker](../tools/compare65816/check_edge_coalescing.py) starts from
frozen old images, patches only two producer-store operands, deletes exactly four
copy instructions and changes only the two selected temporary-home entries.
It relocates all later positions and checks complete images, including uncounted
routines and control-transfer metadata, plus every field of all 264 records.
References into removed copies are rejected. The reviewed emission snapshot
remains unchanged; no NIR fixture or IR contract changed.

Independent native decoders derive logical copies from verified MIR and actual
allocated homes, then validate the retained machine sequence and final A/N/Z
repair. They reject malformed geometry, stale or missing repairs, mutated bytes
and alternate label entries. Independent assembly and simultaneous-byte oracles
cover self, last-self, all-self, repeated sources, reordered copies and partial
overlaps. Existing mixed-width and cyclic fallbacks remain qualified.

Actual source programs execute in raw and optimized modes. Verified target
probes retain optimized rotation MIR in both frontend modes to ensure positive
coalescing coverage even where raw lowering produces no block arguments.
Serialized images and two o65 placements run zero, sign and wrap values, both I
masks and all incoming C/V combinations. Full register and stack checks verify
logical copies. Existing state traces check simultaneous home generations and
A/N/Z, with trace-on/off bytes, labels and fixups unchanged.

Two task domains with different values receive IRQ and NMI at every retained
coalesced-edge instruction and its successor. Independent uninterrupted steps
check full register and live frame/argument/return-memory restoration; seeded
schedules also complete correctly. Call/helper clobbers, recursion, stack guard
faults and authoritative mutable parameter homes remain covered by the native
suite.

[Qualification](abi/action65816-edge-coalescing-qualification.json) records the
93 compiler emitter/proof tests, 60 affected integration tests, 61 comparison-tool
tests and the 14-kernel generator check. All 107 native tests pass in each host
profile, with 432 identical inputs and 626 identical artifacts. All 264 corpus
records agree across host profiles; all 28 Action builds match their LF/CRLF
versions. An isolated CRLF checkout passes the emission snapshot and 37 native
tests, with 550 artifacts identical to LF. Historical inventories and qualification
records remain immutable. The full root suite and NIR sweep were unnecessary
for this native-target-only change.

Reproduce after building `actionc-65816` in release mode:

```sh
python3 -B tools/compare65816/build.py --output target/edge-coalescing-after --verify-crlf
A816_COMPARISON_MANIFEST="$PWD/target/edge-coalescing-after/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/edge-coalescing-after/debug.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -- --ignored
```

Repeat with `--release` and `release.json`. Both observers save all records before
reporting the known vbcc failure. Then run:

```sh
python3 -B tools/compare65816/check_edge_coalescing.py \
  target/parameter-forwarding-after target/edge-coalescing-after \
  --baseline docs/benchmarks/65816-edge-coalescing/baseline.json \
  --output docs/benchmarks/65816-edge-coalescing/delta.json
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```

Scalar DP allocation remains the next distinct strategy to investigate for the
sum loop's memory traffic. Broader coalescing needs new measurements and proofs;
the five interfering historical pairs remain outside this slice.
