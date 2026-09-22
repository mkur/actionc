# Compatible native edge-home coalescing implementation plan

Status: implemented in `12d13e1`, with qualification artifact isolation in
`7b5a2a7`, and qualified on 2026-09-22. See the
[measured results](MIR65816_EDGE_COALESCING.md). This original plan was committed
as `bfc058e` against qualified main `49ecf77`, compiler `4d7fcb3`. The
[frozen baseline](benchmarks/65816-edge-coalescing/baseline.json) refreshes the
historical [movement inventory](MIR65816_MOVEMENT_INVENTORY.md) against
[incoming-parameter forwarding](MIR65816_PARAMETER_FORWARDING.md).

## Objective and bounded scope

Place compatible word temporaries directly in their successor block-parameter
homes, then omit physical self-copies in directly scheduled word edges. Preserve
closed-operation interference, invocation-owned storage, all non-edge captures,
frame extent, staging reservations, stack accounting, guards, ABI v1, image v3,
o65 profile v1, interrupt reserves and Exec816's compiler pin.

This slice changes target allocation and emission only. No NIR pass, semantic
lookback, register/DP allocation, cross-call residence, general graph coloring,
frame compaction or global relaxation of liveness is included. Pointer-leaf DP
allocation remains on its existing separate path.

## Refreshed evidence and forecast

Recheck qualified compiler/fixture inputs, native artifact hashes in both host
profiles, all comparison artifacts and identical 264 debug/release records.
A fresh read-only typed export recompiles all 28 Action images against saved
bytes and runs 56 LF/CRLF compilations. Save it as
`target/coalescing-plan-facts.json`; keep historical inventory files immutable.
The existing exporter probes layouts through verification only, never emission.

Seven private edge pairs remain: two compatible and five interfering. Only
optimized rotation's initialization is selected:

- Move `t0` from S+$06 to the existing `t16` home at S+$0A.
- Move `t2` from S+$08 to the existing `t17` home at S+$06.

Both source changes together pass full frame verification. Moving the second
source alone fails because the first still occupies its new home. Block-parameter
homes are anchors; do not change them. The two producer capture STA operands
change accordingly. At the initialization edge, delete only:

```asm
010038 LDA $06,S
01003A STA $0A,S
01003C LDA $08,S
01003E STA $06,S
```

The retained final `LDA #0; STA $08,S` establishes the original full A/N/Z.
It moves to $010038 and the successor starts at $01003D. Later code, labels,
branches, fixups and proof positions move eight bytes. The cyclic backedge,
its one staging word and all frame/parameter forwarding remain intact.

| Optimized rotation, each vector | Before | Forecast |
| --- | ---: | ---: |
| Code bytes | 138 | 130 |
| Cycles | 916 | 896 |
| Instructions | 222 | 218 |
| Stack-byte reads | 127 | 123 |
| Stack-byte writes | 140 | 136 |
| Fixed frame / observed peak | 16 | 16 |

Both copies execute once for each of six vectors: 12 coalesced assignments,
24 fewer instructions, 120 cycles, 24 reads and 24 writes per incoming I state.
Static savings are eight bytes in one build. All 27 other Action builds, every
raw build and all vbcc outputs should remain unchanged. `sum_loop(13)` stays
120 bytes / 1,212 cycles / 12 stack bytes optimized. These are forecasts to
check, not acceptance inferred from pair compatibility alone.

## Allocation transaction

Keep the existing deterministic first-fit stack allocation and staging plan.
After constructing and verifying that complete frame, run a bounded affinity
pass in stable block order, with then/else arms in a fixed order. Consider only
already supported, directly scheduled all-word edges.

For each edge, collect source TempId → destination TempId affinities when:

- Both are two-byte stack temporaries with complete aligned valid homes.
- The source is an ordinary operation result, never a block parameter; all
  block-parameter homes throughout the routine stay fixed.
- Source and destination are distinct and do not interfere under the unchanged
  whole-routine closed-operation graph.
- The proposal changes a physical home. Conflicting proposals for a repeated
  source reject the transaction; do not choose one destination arbitrarily.

Apply all eligible affinities for that edge simultaneously to a cloned frame,
using destination homes from the pretransaction frame. This admits mutually
necessary moves without combinatorial subset search. Check the resulting map
against every interfering pair, including third-party temps, unused block
parameters, successor live-ins, both branch arms and unreachable blocks.
Do not merge on numeric home equality alone or weaken verification to admit it.

Require the unchanged full frame verifier to accept the transaction, including
exact extent/spill/peak accounting, private/fixed-object disjointness, incoming
bounds and final staging requirements. Reject changes requiring new/reduced
staging or frame accounting in this slice. This deliberately leaves some legal
coalescing opportunities for later work.

Require a strict routine-wide reduction in direct word-copy instruction cost
without increasing any edge's emitted bytes or cycles. Cost includes retained
final-A reloads and selective/complete captures. Unsupported/mixed edges retain
their bytewise shape; changing stack displacement must not change their widths.
Revalidate every edge after each accepted transaction. Failed transactions leave
the original frame byte-for-byte unchanged. One deterministic pass is sufficient;
no retry loop, global optimum or exact savings outside the frozen corpus is
promised. Final emission reuses all normal allocation checks.

## Direct self-copy emission and flag contract

Continue preflighting every logical source and destination, even when its copy
will emit no store. Keep the existing direct scheduling/dependency proof and
word geometry checks. For a direct move whose full stack source equals its full
destination, omit its LDA/STA pair. Retain every non-self assignment in the
existing safe schedule. Selective and complete staged fallbacks are unchanged.

Track the last actually emitted assignment. If it is not the original final
logical assignment, emit one LDA16 from the original final destination after
all stores. This preserves A and full N/Z for last-self and all-self edges,
including single-word edges; C/V, X/Y, S and memory ordering remain unchanged.
An all-self edge therefore retains a final LDA. Do not use flag-dead assumptions,
cache witnesses across the edge, or remove REP/transfer/guard behavior as part
of this optimization. Successor labels continue to clear value permissions.

Keep the cost calculation and emitted direct schedule derived from one checked
representation so the profitability gate cannot overlook flag repair. Preserve
logical edge arity for proof metadata and existing counters. Add separate
Action-only coalesced-copy counts/sites; old `acyclic_edge_words` continues to
count logical assignments, including identities.

## Independent evidence and coverage

Extend native test-only word-edge decoding to recognize omitted self moves from
verified MIR and exact allocated homes. Decode retained instructions independently
of the compiler's scheduling decisions. Validate unchanged sources for every
logical assignment, actual writes/reads and final A/N/Z. An all-self edge is
identified by its retained LDA and typed transfer, never by an invented zero-byte
execution point. Reject mutated operands, partial overlaps, missing repairs,
interior labels and wrong identities. Rebase proof positions for o65.

Focused allocator/selector tests cover simultaneous moves, third-party conflicts,
interfering pairs, repeated sources, immutable block-parameter anchors, mixed
widths, deterministic ordering, staging/accounting rejection and unchanged
fallback frames. Include last-self/all-self, immediate tails, reordered direct
copies, branch arms, cycles, unused parameters and successor live-ins. Exercise
maximum stack offsets and malformed homes before partial emission. Keep the
closed-operation arithmetic input/output conflict intact.

Execute actual serialized raw/optimized source images plus verified target probes
where source optimization removes the shape. Cover zero/sign/wrap words, both
incoming I masks, full-register/frame checks, recursion and call/helper clobbers,
flat images and two o65 placements. Inject IRQ/NMI at each retained instruction
of coalesced edges in two task domains with different values; compare full
registers and live stack memory with an independent uninterrupted step. Include
seeded schedules and actual tracker trace/home-generation checks. Rebuild affected
fixture consumers in an isolated CRLF checkout.

Add an exact comparison checker constructed from the frozen old images: patch
only the two declared producer-home operands, delete the four declared copy
instructions, update only the two temporary home maps, and relocate all later
positions. Check complete images, uncounted routines, branch/PER/call metadata,
all 264 records, unchanged guards/DP traffic/forwarding counts and the stated
read/write reductions. Reject transfers into deleted instructions. All other
builds must retain complete image equality. Keep the known optimized vbcc
`unlink` vector-0 failure visible. Derive any reviewed snapshot update from the
frozen expected transform; do not refresh unrelated fixtures blindly.

## Delivery and checks

1. Commit this plan and its baseline before modifying the compiler.
2. Implement the transaction, direct self-copy selection and independent decoder
   together with focused tests and contract updates. Commit the completed slice
   after its affected checks pass.
3. Measure a fresh `target/edge-coalescing-after` corpus in both host profiles,
   run the exact delta checker, full native and CRLF qualification, save portable
   results, and update the quality-plan baseline in a separate commit.

Use the established focused validation scope:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --lib mir65816 --features native65816-state-proof
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --test mir65816_state_boundary --test mir65816_abi \
  --test mir65816_contract --test mir65816_emission --test mir65816_o65 \
  --test actionc_65816_cli --test actionc_65816_o65_cli
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py
CARGO_INCREMENTAL=0 python3 -B tools/native65816-runtime-tests/qualify.py --release
python3 -B -m unittest discover -s tools/compare65816 -p 'test_*.py'
python3 -B tools/compare65816/corpus.py --check
```

Finish comparison-tool edits before the final corpus build. Build the release
compiler, use `build.py --verify-crlf`, and run ignored `code_quality` with the
new manifest and separate debug/release results. The full root suite and NIR
sweep are unnecessary while changes remain entirely in native target strategy,
emission and its tests. Broaden checks for any discovered contract change.
Preserve unrelated worktree files and stage only owned paths.
