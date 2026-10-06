# Native 65816 record/value placement: stage 5

Stage 5 extends the common placement contract through call-free loops and
reestablishes invocation-backed pointer residence between calls. Header,
backedge and exit obligations use closed fixed-point liveness and complete
simultaneous bindings. The [implementation plan](../../MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md)
keeps indexed and aggregate integration in stage 6.

The frozen [stage-0 workload](../65816-record-placement-stage0/README.md) remains
compiler `d7d536c9`, Exec816 `57df0d7`, 256 source files and 1,156 routines.
Comparisons also authenticate the qualified [stage-4 artifacts](../65816-record-placement-stage4/README.md).
Current source hashes and compact qualification evidence accompany
[results.json](results.json).

## Size and placement

| Profile | Stage 4 code | Stage 5 code | Saved | Loop homes | Call segments |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 434,444 | 434,227 | 217 | 48 | 1 |
| Optimized guarded | 584,121 | 583,903 | 218 | 48 | 1 |
| Raw guarded | 652,416 | 652,416 | 0 | 3 | 0 |

Release savings include `EXECLISTS.FindName` (65 bytes), `Enqueue` (36),
`CONSOLETILING.Show` (32), `FSNAMES.Path` (26), `TASKMEMORY.Writable` (24),
and the call-containing `INPUT.Acquire` (16). `COOKEDLINE.Render` saves another
four bytes. `SDFSWRITE.RowIO` adds two bytes; the whole-program totals include
that cost. The fixed representative set now saves 3.61% against stage 0 in
release and 2.93% with guards. [representatives.csv](representatives.csv)
retains individual rows; the scorecard also retains the full new loop/call
benefit census.

Across every routine and profile, fixed frames, spill bytes and local stack
peaks do not grow against stages 0 or 4. Data identities, sizes and addresses,
native ABI, task/IRQ headroom and bank-zero reservations remain unchanged.

Loop admission preserves the acyclic plan as its fallback. Its size budget
accounts for transfer schedules, staging, mode changes and final A/NZ repairs,
crediting proved stack-pointer preparation savings. A barrier or unsupported
consumer anywhere in a complete live region refuses its DP home. Pressure
retains complete stack captures. Pointer comparisons, indexed forms, wide
arithmetic and other unqualified loop shapes keep their existing strategies.

A pointer needed across a call keeps its canonical invocation stack home.
Profitable local segments reload the complete earlier capture into the existing
`$A0..$BF` pool before their first consumer. No cached copy crosses an edge or
barrier, and no call-save area is added. Typed requests, resource checks and
fresh replay prove exact source, destination, point, extent and coverage.
Artifact maps retain the real stack home. Existing Native65816 private-storage
promotion legality and policy are unchanged; the backend consumes its already
available loop values.

## Native execution

| Profile | Stage 4 cycles | Stage 5 cycles | Stage 4 private accesses | Stage 5 private accesses | Stage 4 peak | Stage 5 peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 85,099 | 72,457 | 25,299 | 20,847 | 39 | 33 |
| Optimized guarded | 89,287 | 76,645 | 25,299 | 20,847 | 39 | 33 |
| Raw guarded | 88,163 | 88,163 | 24,480 | 24,480 | 45 | 45 |

All 147 independent list/record vectors pass in every profile with I=0 and I=4:
882 candidate executions and 882 archived-baseline executions. Results, ABI,
canaries and ordered external accesses match. Every vector meets the 5% cycle
limit and has no stack-peak growth against both stages 0 and 4. Release cycles
fall 14.86% and private accesses 17.60% against stage 4.

The independent `Flow` traversal also benefits: release cycles fall from 14,104
to 13,096 and private accesses from 4,008 to 3,744. Guarded mode improves the
same measures; raw mode is unchanged. Candidate and CRLF-source artifacts are
identical. Archived references reproduce the original stage-0 CPU costs and
retain explicit archived-artifact attribution.

## Qualification

Backend library tests pass (411, one existing ignored), as do all fourteen
root backend integration targets (88 tests, four existing ignored). The full
native suite passes 374 tests across 94 targets, with six existing opt-in tests
ignored. Twelve comparison-tool tests pass. The seven compressed native
qualification manifests authenticate one current source generation, the pinned
independent CPU and its status-timing correction, assembler tools and execution
artifacts. Shared frontend/NIR contracts are unchanged, so local validation is
scoped to the 65816 backend and its consumers.

Focused regressions cover nested and zero-trip traversal, changing bases and
far/bank-crossing records, with exact external read/write oracles and canaries.
Authored verified MIR combines repeated loop visits with direct, indirect,
recursive, arithmetic-helper and assembly calls. The assembly overwrites the
whole compiler scratch pool and changes the original pointer field; subsequent
consumers must retain the earlier pointer capture while observing fresh field
contents. Caller scalar captures and call results remain intact too.

IRQ injection covers every distinct reachable enabled instruction in the
nested-loop function in both task domains, with NMI and reentrant dispatcher
calls. Pressure and forged entry, interval and reload plans have focused
coverage. The legacy cyclic pointer fixture deliberately retains a volatile
fallback so its independent oracle still exercises an actual physical copy
cycle and complete register-state preservation, including relocation.

## Compiler cost

Three serial measured rounds follow one warm-up per compiler/profile. Each
separate optimized-dev CLI process uses opt-level 3, debug 0 and no incremental
compilation. Compiler and build order alternate. Per-child `wait4` accounting
records wall time and peak RSS; every run reproduces its pinned compiler image.
No build or native qualification runs concurrently with these measurements.

| Profile | Stage 4 wall (s) | Stage 5 wall (s) | Wall change | Stage 4 RSS (MiB) | Stage 5 RSS (MiB) | RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 55.664 | 54.823 | −1.51% | 1,211.1 | 1,276.7 | +5.41% |
| Optimized guarded | 40.109 | 40.803 | +1.73% | 1,387.0 | 1,305.2 | −5.90% |
| Raw guarded | 46.912 | 46.095 | −1.74% | 1,192.4 | 1,250.0 | +4.83% |

Every profile meets the incremental 5% wall-time and 10% peak-RSS limits.
Median corpus wall time falls from 142.686 to 141.552 seconds. Per-child RSS
samples vary; individual samples and medians remain in the scorecard, with no
follow-up needed to meet these gates. Loop trials are omitted when the cyclic
core is empty or no new residence choice is admitted.

## Reproduction

Preserve the stage-0 inputs and pinned stage-4 CLI, images and measurements.
Stage 4's exact 577 compiler/test/tool inputs were verified before this work and
archived at `target/record-placement-stage5/stage4-inputs.tar.gz`; current hashes
are retained in [compiler-inputs.json](compiler-inputs.json).

Build the ordinary CLI and `record_probe --features placement-analysis` with
opt-level 3, debug 0 and incremental compilation disabled. Pin the CLI at
`target/record-placement-stage5/actionc-65816`. Use the stage-3 reproduction
commands with the stage-5 output directory for probe, native vectors and
archived references. Run the backend library, fourteen root backend integration
targets, full native suite and each candidate/reference profile through
`tools/native65816-runtime-tests/qualify.py`.

After builds, probes and native qualification finish, measure serial paired
CLI costs and publish:

```sh
python3 -B tools/compare65816/exec_record_consumer.py host \
  --stage 5 --base target/record-placement-stage0 \
  --previous target/record-placement-stage4 --output target/record-placement-stage5 \
  --binary target/record-placement-stage5/actionc-65816 --rounds 3
python3 -B tools/compare65816/exec_record_consumer.py publish \
  --stage 5 --base target/record-placement-stage0 \
  --previous target/record-placement-stage4 --output target/record-placement-stage5
```

The stage-0 unchecked hosted-provider and standalone fixture gaps remain recorded
obligations. This native qualification adds no hosted-release or hardware claim.
The remaining whole-plan targets belong to stages 6–7.
