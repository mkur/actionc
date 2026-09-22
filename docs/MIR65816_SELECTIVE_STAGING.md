# Native 65816 selective staging

Implemented and qualified on 2026-09-22. Independent machine references were
committed in `3308cb7`; compiler, allocation and evidence changes are in
`8af3541`. The [implementation plan](MIR65816_SELECTIVE_STAGING_PLAN.md) and its
[frozen forecast](benchmarks/65816-selective-staging/baseline.json) remain the
pre-change evidence. Public ABI v1, image v3, o65 profile v1 and stack guards are
unchanged.

## Selection and allocation

After preserving the existing single-word and acyclic schedules, a cyclic edge
with disjoint word destinations and no partial source/destination overlap uses
selective staging. It saves each source that an earlier original-order
assignment would overwrite. All captures precede all assignments. Assignments
then execute in original order, reading saved sources only when needed. The
last assignment preserves full A and N/Z without another reload.

Captured move indices and physical scratch slots are distinct: moves [1, 4]
use pool slots [0, 1]. Allocation reserves each capture ordinal's maximum width
across all edges. The final frame verifier rechecks the plan, capacities,
physical mappings, separation from objects/live homes, incoming last-byte bounds
and frame/peak accounting. Multiple cycles and reverse rotations can require
multiple captures. Repeated endangered sources keep separate slots.

Partial overlaps retain complete word staging; mixed-width and unsupported word
forms retain complete bytewise staging. No self-copy elimination, coalescing,
source deduplication, register/DP residence, helper-clobber assumptions or
source-memory reordering is introduced. Scratch remains invocation-owned stack
memory. See the [emission contract](MIR65816_EMISSION_CONTRACT.md) and
[allocation contract](MIR65816_TEMPORARY_ALLOCATION.md).

## Measurements

The [exact comparison](benchmarks/65816-selective-staging/delta.json) confirms
every frozen forecast. Only optimized `loop_rotation` changes among 28 Action
builds. All six input vectors execute the selected backedge eight times.

| Optimized `loop_rotation(13)` | Before | After |
| --- | ---: | ---: |
| Code bytes | 148 | 140 |
| Cycles | 1,116 | 956 |
| Instructions | 262 | 230 |
| Stack-byte reads | 175 | 143 |
| Stack-byte writes | 172 | 140 |
| Staging bytes | 6 | 2 |
| Fixed frame / observed peak | 20 | 16 |
| Spill bytes | 16 | 12 |
| Incoming argument body displacement | 24 | 20 |

The backedge captures only its second source at S+$0E, then performs the three
assignments. It begins at $010068; its JML moves to $010078 and still targets
$010045. The guard and teardown reserve/release 16 bytes. Incoming ABI offset
zero and all object/temp homes remain unchanged.

All 27 other Action builds remain byte-identical, as do the vbcc machine
artifacts and results. `sum_loop(13)` remains 120 bytes / 1,212 cycles / 12 stack
bytes optimized, and 146 / 1,587 / 14 raw. The preexisting optimized vbcc `unlink`
vector-0 failure remains explicit; all Action outputs are correct.

New Action-only counters distinguish selective edges from direct acyclic edges.
Each rotation vector records 8 selective edges, 24 assignments, 8 staged words
and 16 direct words. Existing logical edge, fusion and forwarding counts retain
their meanings. Across all six vectors per incoming I state, the slice removes
192 instructions, 960 cycles, 192 stack-byte reads and 192 writes. The static
saving is eight bytes once.

## Qualification and reproduction

- 76 emitter/proof tests and 60 affected compiler integration tests pass. Small
  graphs are checked against a simultaneous byte-copy oracle, including full A.
  Tests cover sparse capture indices, mixed per-slot capacities, corrupt maps,
  partial overlaps, transient offsets and the exact 254-byte frame boundary.
  Forward 64-word rotations now fit; truly overflowing reverse rotations fail.
- All 98 native tests pass in both debug and release hosts: 374 identical
  artifacts and 421 identical compiler/fixture input hashes. Independent ca65
  selective/full-staging references check boundary values, full registers/status,
  exact cycles, stack/DP access traces and unchanged bytes outside expected writes.
- Verified cyclic MIR exercises both frontend modes, rotations in both
  directions, ordinary/fused arms and mixed-width fallback. Typed final-byte
  evidence rejects missing/late/reused captures, changed operands, stale targets,
  truncated windows and interior suffixes. Existing o65 placements carry the
  evidence through relocation. Existing helper, call, alias, volatile and
  bank-crossing coverage remains enabled.
- IRQ injection covers every selected capture and assignment instruction in
  both task domains, checking complete CPU and invocation-frame restoration.
  Seeded IRQ/NMI tests pass. Exact-floor and one-byte-short guard cases check
  both frontend modes and incoming I states before any frame writes.
- The strict checker validates all 28 complete Action images, including later
  uncounted driver code, every instruction and all metadata. It independently
  predicts capture reordering/deletion, scratch/frame/argument operands and
  address/branch/PER remapping. Every existing measurement field matches or has
  its declared delta. Both hosts save identical 264 records (528 executions
  each), preserving the known vbcc failure rather than treating it as success.
- LF/CRLF corpus builds match. An isolated CRLF checkout rebuilt and passed the
  unchanged emission-boundary snapshot and 18 native edge, guard and preemption
  tests. All 38 comparison-tool tests and the 14-kernel generator check pass.
  No NIR or semantic contract changed.

```sh
python3 -B tools/compare65816/check_selective_staging.py \
  target/staging-reservations-after target/selective-staging-after \
  --baseline docs/benchmarks/65816-selective-staging/baseline.json \
  --output docs/benchmarks/65816-selective-staging/delta.json
python3 -B tools/compare65816/report.py --input target/selective-staging-after \
  --output docs/benchmarks/65816-selective-staging/after
```

Full build and paired-host commands remain in the
[comparison workflow](../tools/compare65816/README.md). The
[qualification record](abi/action65816-selective-staging-qualification.json)
and [measured snapshot](benchmarks/65816-selective-staging/after/tables.md)
establish the new baseline. Home coalescing, broader forwarding, CPU/DP
allocation and Exec816 integration remain separate work.
