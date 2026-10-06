# Native 65816 record/value placement: stage 3

Stage 3 brings complete pointer and scalar residence into ordinary mixed blocks.
The common placement contract proves each producer, consumer, private transfer
and resource window. The [implementation plan](../../MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md)
keeps branches, loops and broader indexed/aggregate integration in later stages.

The frozen [stage-0 workload](../65816-record-placement-stage0/README.md) uses
compiler baseline `d7d536c9`, Exec816 `57df0d7`, 256 source files and 1,156
emitted routines. Candidate working-tree source hashes are retained in
[compiler-inputs.json](compiler-inputs.json).

## Size and resource gates

| Profile | Baseline code | Candidate code | Baseline representatives | Candidate representatives | Reduction | Mixed DP homes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 444,948 | 434,502 | 10,262 | 9,950 | 3.04% | 1,670 |
| Optimized guarded | 594,828 | 584,179 | 12,613 | 12,301 | 2.47% | 1,670 |
| Raw guarded | 661,861 | 652,424 | 13,908 | 13,621 | 2.06% | 2,378 |

Every profile meets the first-tranche gate of at least 2% reduction in the fixed
representative set. Benefits span `TASKPOLICY`, `COOKEDLINE` and `SDFSFILE`.
Release code falls by 10,446 bytes (2.35%). `COOKEDLINE.Recall` falls from 943
to 917 bytes, and `COOKEDLINE.Render` from 1,655 to 1,586 bytes with its frame
reduced from 26 to 24 bytes. Individual representatives are retained in
[representatives.csv](representatives.csv).

Across all 1,156 routines and all three profiles, fixed frames, spill bytes and
local stack peaks do not grow. Data object identities, extents and addresses,
task/IRQ headroom, native ABI and bank-zero reservations are unchanged. Existing
pointer-only leaf and closed scalar allocation families keep their admissions.

The block-local plan uses the existing `$A0..$BF` residence pool. Complete DP-only
values have truthful DP homes in public maps. Captures needed beyond a bounded
prefix retain invocation stack homes; an explicitly verified complete transfer
establishes the cached DP copy. Calls and unsupported resource windows end that
copy's usable interval. The captured pointer is retained independently of its
pointee, and no external field read is reused or reordered.

Pressure falls back to ordinary stack captures. Trial allocation includes
parallel edge staging; choices that increase the frame or local peak are refused.
The frozen Exec census admits DP-only intervals; no prefix meets the backed-cache
cost gate. The backed path is independently exercised by the native call-clobber
test. This tranche uses existing typed captures and leaves shared NIR promotion policy
and legality unchanged. A broader private-storage profitability admission needs
its own benefit evidence and shared-contract validation.

## Native execution and access gates

| Profile | Baseline cycles | Candidate cycles | Baseline private accesses | Candidate private accesses | Baseline peak | Candidate peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 86,797 | 84,919 | 25,707 | 25,263 | 42 | 39 |
| Optimized guarded | 91,417 | 89,107 | 25,707 | 25,263 | 42 | 39 |
| Raw guarded | 90,155 | 88,163 | 24,816 | 24,480 | 48 | 45 |

All 147 independent list/record vectors pass in every profile with I=0 and I=4:
882 candidate executions and 882 archived-baseline executions. Results, ABI
restoration, canaries and ordered external byte-access traces match exactly.
No vector exceeds the 5% cycle-regression limit or grows its stack peak. Private
accesses count stack and DP reads/writes separately from those external traces.

The independent `Flow` probe also benefits. In release mode its total cycles
fall from 15,406 to 13,924 and private accesses from 4,272 to 3,972. Guarded and
raw modes improve both measures too. This is independent of Exec816's layout
and routine names.

Archived reference manifests explicitly select archived-artifact execution.
The report authenticates their frozen images and reproduces every original
CPU cost; it claims no current selected-code proof for those old bytes. Candidate
images retain the runner's current compiler verification.

## Qualification

Backend library tests pass (400, one existing ignored), as do all 14 backend
integration targets (88 tests, four existing opt-in tests ignored). The full
qualified native suite passes: 368 tests across 92 targets, with six existing
opt-in tests ignored. Ten baseline/accounting and consumer-report tool tests
also pass. Shared frontend/NIR contracts are unchanged; other backends' suites
were not executed.

Seven focused compiler tests cover complete mixed captures, bounded stack-backed
prefixes, resource barriers, pressure, forged plans, repeated-base profitability,
unreachable producers and existing top-bit ownership. Four independent native
tests exercise BYTE/CARD/LONGCARD field accesses, far and bank-crossing records, wraparound, exact
external accesses, a callee overwriting cached DP bytes, task suspension and
reentrant IRQ/NMI. A twelve-pointer pressure case executes with eight complete DP
homes and four stack homes, preserving every destination and its padding. IRQ
injection covers each distinct reachable enabled instruction in the mixed test
routine in both task domains.

Actual LF and CRLF record sources compile to identical images in raw and
optimized modes; the frozen vector builds check all three profiles. The reviewed
emission-boundary snapshot intentionally changes to reflect actual DP homes,
remaining frame demand, guard/argument offsets and remapped emission positions.
This is a backend emission contract change; NIR shapes and printing are unchanged.

## Compiler cost

| Profile | Baseline median seconds | Candidate median seconds | Wall ratio | Peak-RSS ratio |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 55.630 | 55.217 | 0.993 | 1.005 |
| Optimized guarded | 40.680 | 41.264 | 1.014 | 0.881 |
| Raw guarded | 45.837 | 46.112 | 1.006 | 1.280 |
| Raw-only repeat | 43.845 | 43.915 | 1.002 | 0.983 |

These are warm-cache serial CLI runs on a shared macOS host. Both compilers use
optimized dev settings (opt-level 3, debug 0, incremental disabled). Each profile
has one warm-up and three measured rounds, with alternating compiler and profile
order; every output must match its compiler-specific pinned image hash. Per-child
`wait4` reports CPU and peak RSS. Observer and native qualification work is excluded.

All initial wall-time medians are within 1.5% of baseline. Initial raw RSS is
28% higher by the three-sample median. That baseline ranges from 944 to 1,113 MiB
and the candidate from 1,048 to 1,256 MiB. A targeted three-round raw-only repeat
uses the same binaries, inputs and pinned images: its baseline ranges from
1,239 to 1,246 MiB and candidate from 1,219 to 1,244 MiB, with a median ratio
of 0.983. It does not reproduce the earlier increase. Both complete runs are
retained; this shared-host RSS variation limits conclusions about memory growth.
The foundation stages' 10% RSS bound is not claimed for the initial stage-3 run.

[results.json](results.json) retains numerical gates, all host samples and
successful qualification summaries. Seven compressed qualification manifests
retain exact compiler/fixture and independent VM source hashes. The pinned VM
is `56ddc5c5` plus the committed status-timing correction.
[evidence-sha256.json](evidence-sha256.json) authenticates the compact publication.
Bulky artifacts and logs remain under `target/record-placement-stage3/`.

## Reproduction

Reproduce stage 0 with its documented tools, preserving the frozen inputs.
Build both the ordinary `actionc-65816` CLI and the `record_probe` with its
`placement-analysis` feature using the optimized-dev settings above. Pin the
CLI at `target/record-placement-stage3/actionc-65816`, then run:

```sh
python3 -B tools/compare65816/exec_record_consumer.py probe \
  --base target/record-placement-stage0 --output target/record-placement-stage3 \
  --binary target/record-placement-stage0/rust-target/debug/actionc-exec-record-probe
python3 -B tools/compare65816/exec_record_vectors.py \
  --base target/record-placement-stage0 \
  --output target/record-placement-stage3/native-vectors \
  --binary target/record-placement-stage3/actionc-65816
python3 -B tools/compare65816/exec_record_consumer.py references \
  --base target/record-placement-stage0 --output target/record-placement-stage3
```

Run the scoped backend library and 14 integration targets, followed by the full
native suite through `tools/native65816-runtime-tests/qualify.py`. For each
profile, run `--test code_quality -- --ignored` with `A816_COMPARISON_MANIFEST`
and `A816_COMPARISON_RESULTS` selecting its candidate manifest/measurements and
then its archived reference manifest/measurements. Retain the successful logs.
The [comparison tooling](../../../tools/compare65816/README.md) documents the
manifest modes and access observer.

After all builds and native qualification finish, measure the serial CLI costs
and publish the checked evidence:

```sh
python3 -B tools/compare65816/exec_record_consumer.py host \
  --base target/record-placement-stage0 --output target/record-placement-stage3 \
  --binary target/record-placement-stage3/actionc-65816 --rounds 3
python3 -B tools/compare65816/exec_record_consumer.py publish \
  --base target/record-placement-stage0 --output target/record-placement-stage3
```

The stage-0 unchecked hosted-provider rejection, standalone list/DOS fixture
gaps and raw port restoration failure remain recorded obligations. This native
consumer qualification adds no claim of unchecked hosted release or hardware
qualification. The final whole-plan targets remain for stages 4–7.
