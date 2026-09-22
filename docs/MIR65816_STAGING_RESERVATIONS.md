# Native 65816 compact staging reservations

Implemented on main in `49a0aae`, qualified on 2026-09-22. This slice removes
unused staging reservations separately from copy scheduling.
The [frozen forecast](benchmarks/65816-staging-reservations/baseline.json) was
committed in `c323a8f` against qualified main `25eaef7` before implementation.
It preserves public ABI v1, image v3, o65 profile v1 and stack guard logic.

## Allocation contract

Allocation and selection share the existing word-copy planner. Direct word
edges, including acyclic multi-word copies, require no staging. Every staged
edge still saves all arguments before any destination write. Slot i reserves
the maximum actual argument width at i across staged edges, rather than an
unconditional four bytes for every block parameter. Multi-byte slots retain
even alignment; temp homes and frame objects do not move.

Planning starts with the smallest frame containing all private homes. Immutable
incoming arguments are above this extent and cannot alias a destination. Adding
staging moves them farther away without changing dependencies. Mutable parameters
use their fixed object homes. The final allocation independently rechecks staging
requirements, byte ranges, alignment, incoming argument bounds and frame/peak
accounting. Selection validates required capacity before emitting copies.

No source saving, self-copy, store or final A/N/Z restoration is removed here.
Cycles retain full staging, including independent assignments on a cyclic edge.
No new CPU/DP residence or alias assumption is introduced. Stack guards use the
new reservation size and still fault before writes on failed entry checks.
See the [emission contract](MIR65816_EMISSION_CONTRACT.md) and
[allocation contract](MIR65816_TEMPORARY_ALLOCATION.md).

## Measured result

| Optimized kernel | Staging bytes before → after | Frame/observed peak before → after |
| --- | ---: | ---: |
| `sum_loop` | 4 → 0 | 16 → 12 |
| `byte_sum` | 4 → 0 | 22 → 18 |
| `loop_rotation` | 12 → 6 | 26 → 20 |

The [exact delta](benchmarks/65816-staging-reservations/delta.json) confirms the
frozen forecast. All three retain their instruction counts, sizes, cycle counts
and memory access counts. Their reservation/teardown immediates, incoming displacements and used
staging offsets change. The other 25 Action raw/optimized corpus builds
remain byte-identical. Raw `sum_loop(13)` remains 146 bytes / 1,587 cycles / 14
stack bytes; optimized remains 120 / 1,212 with 12 stack bytes.

The exact checker compares every final instruction against frozen operand
patches, entire serialized images (including metadata), artifact contracts and
every measurement field. Historical snapshots and the known optimized vbcc
`unlink` failure remain visible and unchanged.

```sh
python3 -B tools/compare65816/check_staging_reservations.py \
  target/acyclic-edges-after target/staging-reservations-after \
  --baseline docs/benchmarks/65816-staging-reservations/baseline.json \
  --output docs/benchmarks/65816-staging-reservations/delta.json
python3 -B tools/compare65816/report.py --input target/staging-reservations-after \
  --output docs/benchmarks/65816-staging-reservations/after
```

## Qualification

- 72 emitter/proof tests and 60 affected compiler integration tests pass. New
  tests cover direct edges without slots, mutable/incoming parameter homes,
  cycles, mixed-width per-index capacities, corrupt allocations, high pressure,
  last-byte incoming bounds and the retained 254-byte limit.
- All 95 native execution tests pass in both debug and release hosts, producing
  374 identical artifacts from 420 identical compiler/fixture inputs. Coverage
  includes calls/helpers, aliasing, mixed-width fallback, o65 relocation and
  instruction-boundary IRQ/NMI injection. The new guard test exercises the exact
  floor and one-byte-short failure for all three affected kernels in both modes
  and both incoming I states, checking that no write precedes the reservation.
- The decoded-edge tests represent absent staging explicitly. They check the
  full stack outside expected writes, instead of placing canaries in bytes that
  are no longer reserved. Independent ca65 reference-copy tests remain.
- All 28 Action instruction streams match exactly with only 21 frozen operand
  patches. Full serialized images and metadata match their independently
  predicted changes. All 264 debug/release corpus records are identical, with
  both incoming I states checked per vector. Only the preexisting optimized
  vbcc `unlink` vector-0 failure remains; all Action results are correct.
- LF/CRLF corpus builds are identical. The emission snapshot changes only the
  optimized sum-loop frame and five known operand patches; these were derived
  from the frozen baseline rather than regenerated from compiler output. No
  NIR fixture or contract changes. An isolated CRLF checkout rebuilt and passed
  the boundary snapshot test and all three stack-fault tests using CRLF embedded
  fixtures. All 35 comparison-tool tests pass.

The [measured snapshot](benchmarks/65816-staging-reservations/after/tables.md)
and `target/staging-reservations-after` are the new immutable quality baseline.
Qualification provenance is recorded in
[the manifest](abi/action65816-staging-reservations-qualification.json).

Selective staging within cycles and home coalescing remain separate work.
