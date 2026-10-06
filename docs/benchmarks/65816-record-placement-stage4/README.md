# Native 65816 record/value placement: stage 4

Stage 4 extends complete typed captures through acyclic branches and joins.
The [placement contract](../../MIR65816_PLACEMENT_CONTRACT.md#acyclic-branch-and-join-residence)
defines entry obligations, complete interference and simultaneous mixed transfers.
The [implementation plan](../../MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md) leaves
loop-carried residence, call splitting and broader private-storage promotion
for subsequent stages.

The frozen stage-0 inputs retain compiler `d7d536c9`, Exec816 `57df0d7`, 256
source files and 1,156 emitted routines. This scorecard also compares against
the qualified [stage-3 images](../65816-record-placement-stage3/README.md) from
compiler commit `10ce9332`; preceding CLI/image hashes are authenticated before
comparison. The candidate working tree is identified by complete input hashes
in [compiler-inputs.json](compiler-inputs.json).

## Contract and scope

Every inherited capture must survive in the same complete home on every incoming
edge. Block parameters instead receive their own simultaneous argument binding;
parallel edges retain distinct ordinals, including equal targets with different
arguments. Closed live regions and full byte extents determine DP interference.
Unsupported windows, calls and cyclic regions retain existing storage choices.
Edge-only captures entering loops retain stack affinities when the target cannot
retain DP residence. Trial allocation includes actual transfer staging and
refuses frame or local-peak growth relative to block-local placement.

Mixed transfers use typed schedules that consume a complete overlapping source
before overwriting it, or capture that whole source into an invocation-owned
slot. Identities, repeated sources, partial overlap, different widths and
independent cycles are covered. Public maps contain actual homes and staging;
replay emits the canonical schedule and verifies every logical edge.

Source-language private-storage admission is unchanged. The ordinary branching
record source test verifies its existing private-memory fallback and exact field
protocol. Independently authored, verifier-clean MIR exercises complete captures
through diamonds, same-target edges and separate returns. A volatile access in
one predecessor forces the inherited captures to stack; fresh join parameters
receive their complete incoming bindings. These cases do not claim that all
source locals are promoted. An authored Form routine also carries complete
pointer and word values through mixed transfers while two task domains and reentrant IRQ/NMI
execute it. Injection covers each distinct reachable enabled instruction in
that routine in both task domains.

## Qualification and measurements

Backend library tests pass: 408 tests, with one existing opt-in test ignored.
All fourteen root backend integration targets pass: 88 tests, four existing
opt-in tests ignored. Full native qualification passes 371 tests across 93
targets, with six existing opt-in tests ignored. Eleven baseline/accounting and
consumer-report tool tests also pass. Shared frontend/NIR contracts are unchanged;
other backends' suites were not executed.

Eight new compiler tests cover entry/region forgery, independent copy-schedule
byte oracles, distinct parallel bindings, resource/loop refusal, loop-preheader
stack affinity and selected transfer/staging forgery. Three new native tests
cover ordinary source fallback, independently authored CFG forms and reentrant
task/IRQ/NMI execution. Actual LF and CRLF record sources produce identical
images in raw and optimized modes; frozen vector builds cover all three profiles.
The existing emission-boundary snapshot remains unchanged.

| Profile | Stage-3 code | Stage-4 code | New saving | Fixed representatives, stage 0 → stage 4 | Branch homes / entry tables / mixed edges |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 434,502 | 434,444 | 58 | 10,262 → 9,932 | 32 / 27 / 4 |
| Optimized guarded | 584,179 | 584,121 | 58 | 12,613 → 12,283 | 32 / 27 / 4 |
| Raw guarded | 652,424 | 652,416 | 8 | 13,908 → 13,621 | 3 / 5 / 0 |

Both optimized profiles meet the new branching-representative gate:
`COOKEDLINE.Recall` shrinks from 917 to 899 release bytes, and from 1,059 to
1,041 guarded bytes. Raw mode retains its conservative private-storage policy;
it must avoid growth but has no new branching-representative requirement.
Across all 1,156 routines, frame, spill and local-peak demand do not grow against
either stage 0 or stage 3. ABI, data placement and bank-zero reservations are
unchanged. Release savings since stage 0 total 10,504 bytes (2.36%), and the
fixed representatives shrink by 3.22%.

The optimized net saving comprises 104 bytes saved in four routines and 46
bytes of growth in seven others. `SDFSWRITE.Validate` saves 46 bytes,
`PROGRAMFILE.Load` 24, `COOKEDLINE.Recall` 18 and `MYDOSFILE.Open` 16.
Individual growth ranges from 2 to 12 bytes; the largest is
`FSREGISTRY.Lookup`. These are measured costs of moving captures between
different homes at edges. The stage admits verified residence with bounded
frame demand; it does not yet choose each location by emitted byte cost.
The raw saving is eight bytes in `MYDOSFILE.Open`, with its frame reduced from
16 to 14 bytes. No other raw routine changes size.

| Profile | Stage-3 cycles | Stage-4 cycles | Stage-3 private accesses | Stage-4 private accesses | Stage-3 / stage-4 peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 84,919 | 85,099 | 25,263 | 25,299 | 39 / 39 |
| Optimized guarded | 89,107 | 89,287 | 25,263 | 25,299 | 39 / 39 |
| Raw guarded | 88,163 | 88,163 | 24,480 | 24,480 | 45 / 45 |

These totals use the same 147 independent record/list vectors per profile.
Every vector executes with I=0 and I=4: 882 candidate and 882 archived-baseline
executions. Results, ABI restoration, canaries and ordered external byte-access
traces match. Every vector stays within the 5% cycle-regression limit and keeps
its stack peak against both stages 0 and 3. The table records one observation
per vector after requiring both mask states to agree.
The largest stage-3 cycle increase is 2.72% in release and 2.54% in guarded mode.

The optimized increase is confined to the independent `Flow` probe: canonical
mixed byte transfers add 180 cycles and 36 private accesses over stage 3.
Release `Flow` therefore uses 14,104 cycles and 4,008 private accesses, compared
with stage-0 totals of 15,406 and 4,272. All three profiles retain the stage-0
independent-probe benefit. Private accesses count stack and DP reads/writes;
they are separate from the exact external trace.

Final numerical and qualification results are published in [results.json](results.json).
Representative rows are in [representatives.csv](representatives.csv). Seven
compressed manifests retain exact compiler, fixture and pinned VM inputs;
[evidence-sha256.json](evidence-sha256.json) authenticates the compact publication.
Bulky images, native measurements and logs remain under
`target/record-placement-stage4/`.

## Compiler cost

| Profile | Stage-3 median seconds | Stage-4 median seconds | Wall ratio | Peak-RSS ratio |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 53.912 | 54.006 | 1.002 | 0.895 |
| Optimized guarded | 39.511 | 39.754 | 1.006 | 1.384 |
| Raw guarded | 44.685 | 44.680 | 1.000 | 1.046 |
| Guarded-only repeat | 38.698 | 38.870 | 1.004 | 0.975 |

Serial warm-cache CLI measurements use the authenticated stage-3 binary and the
stage-4 binary, both built with opt-level 3, debug 0 and incremental compilation
disabled. Each profile has one warm-up and three measured rounds, alternating
compiler and profile order. Every run must reproduce its compiler-specific
pinned image. Per-child `wait4` accounts for CPU and peak RSS; observer, build
and native qualification work is excluded.

Initial wall-time medians differ by less than 1%. The guarded peak-RSS median
is 38.4% higher: stage-3 samples range from 881 to 1,315 MiB, while stage-4
samples range from 1,326 to 1,328 MiB. A separate guarded-only repeat uses the
same inputs, binaries and pinned images: stage 3 ranges from 1,317 to 1,337 MiB
and stage 4 from 1,278 to 1,384 MiB, with a median ratio of 0.975. It does not
reproduce the earlier increase. Both complete runs are retained; this shared-host
RSS variation limits conclusions about memory growth. The initial run does not
establish the foundation stages' 10% RSS bound.

## Reproduction

Preserve the stage-0 inputs and qualified stage-3 CLI/images. Build the ordinary
CLI and `record_probe --features placement-analysis` with opt-level 3, debug 0
and incremental compilation disabled. Pin the CLI at
`target/record-placement-stage4/actionc-65816`, then use the stage-3 reproduction
commands with `--output target/record-placement-stage4` for probe, vectors and
archived references. Qualify the backend library and fourteen root integration
targets, the full native suite, and each candidate/reference profile through
`tools/native65816-runtime-tests/qualify.py`.

After builds, observer and native qualification finish, run serial paired costs
and publish:

```sh
python3 -B tools/compare65816/exec_record_consumer.py host \
  --stage 4 --base target/record-placement-stage0 \
  --previous target/record-placement-stage3 --output target/record-placement-stage4 \
  --binary target/record-placement-stage4/actionc-65816 --rounds 3
python3 -B tools/compare65816/exec_record_consumer.py publish \
  --stage 4 --base target/record-placement-stage0 \
  --previous target/record-placement-stage3 --output target/record-placement-stage4
```

The stage-0 unchecked hosted-provider and standalone fixture gaps remain recorded
obligations. This native qualification makes no new hosted-release or hardware
claim. The remaining whole-plan targets belong to stages 5–7.
