# Native 65816 record/value placement: stage 6

Stage 6 integrates embedded-array accesses and nonvolatile aggregate copies into
the common resource and value placement plan. Complete pointer captures can
remain in existing domain DP storage while indexes, field displacements and
copy workspaces use their checked locations. The public ABI and reserved
bank-zero bytes remain unchanged.

## Frozen Exec816 workload

The stage-0 inputs remain unchanged: Exec revision `57df0d7`, 256 source files
and 1,156 routines. Three profiles use the same runtime and compare against the
qualified stage-5 compiler and artifacts.

| Profile | Stage 5 code | Stage 6 code | Saved | Indexed windows | Resident indexed windows |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 434,227 | 430,091 | 4,136 (0.95%) | 857 | 558 |
| Optimized guarded | 583,903 | 579,765 | 4,138 (0.71%) | 857 | 558 |
| Raw guarded | 652,416 | 648,963 | 3,453 (0.53%) | 858 | 615 |

Indexed workflows improve in 157 routines in each optimized profile and 140 in
raw mode. Every routine's frame, spill extent and local stack peak is checked
against stages 0 and 5. Data identities, sizes and addresses and the ABI/task/IRQ
reservations remain unchanged.

Representative release results against stage 5 include:

| Routine | Stage 5 bytes | Stage 6 bytes |
| --- | ---: | ---: |
| BLOCKIO.DeviceName | 227 | 87 |
| FSINFO.Name | 739 | 612 |
| SDFSWRITE.Create | 2,031 | 1,909 |
| CONSOLEFOREGROUND.Reclaim | 775 | 667 |
| COOKEDLINE.Recall | 899 | 873 |

The frozen Exec MIR contains zero aggregate-copy operations. Aggregate admission
and correctness are qualified by independent native cases; this report does not
attribute aggregate code or runtime savings to Exec.

## Independent native measurements

Each profile executes 147 frozen vector cases with both incoming interrupt
masks, using independent result, ABI and external-access oracles. Ordered
external reads/writes match stages 0 and 5 exactly, and every case meets the
cycle and stack gates.

| Profile | Stage 5 cycles | Stage 6 cycles | Stage 5 private accesses | Stage 6 private accesses | Stack peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 72,457 | 71,833 | 20,847 | 20,655 | 33 |
| Optimized guarded | 76,645 | 76,021 | 20,847 | 20,655 | 33 |
| Raw guarded | 88,163 | 87,539 | 24,480 | 24,288 | 45 |

The incremental change saves 624 cycles and 192 private accesses in each
profile. Private accesses count actual stack and domain DP byte traffic. These
are totals over this fixed corpus, including the independent mixed record-flow
probe; they are not whole-Exec runtime measurements.

## Contracts and qualification

Native indexing shares its complete offset/stride/payload bounds with residence
budgets. Larger offsets and wider indexes use full modular 24-bit arithmetic in
a working copy, preserving the captured pointer. Existing deferred three-byte
A/X/component producers retain their stack input bindings until their later
physical consumption has a complete placement contract.

A typed aggregate request binds the original source point, exact byte extent and
overlap policy. Fresh replay reconstructs the entire transfer protocol, including
loop counts and direction. Scratch interference and static payload sites are
checked separately. Copy ordering, padding, self-copy behavior and external byte
accesses preserve the existing protocol. Zero extent has no memory effects;
volatile copies retain their unsupported diagnostic.

Five new native cases cover raw and optimized embedded arrays, payload widths
1–4, BYTE/CARD/24-bit index magnitudes, strides and offsets above 64 KiB, bank
crossings and modular address carry. Relocated O65 execution and actual LF/CRLF
compilation check the source path. Aggregate extents include 0, 1, 3, 8, 259 and
65,537 bytes, forward/backward overlap, self-copy and nonoverlap. Immutable
captures survive writes to aliased storage; pressure retains complete stack
homes. IRQ injection covers every reachable enabled instruction in the combined
indexed/aggregate routine, with NMI, reentrant dispatch and both task domains.
Aggregate tests cross 64 KiB banks; wrapping aggregate object aliases receive
no new guarantee from this placement change.

Compiler tests reject forged scratch, payload bounds, aggregate extent/overlap
requests and a changed loop count with unchanged static access totals. The
address-selection integration fixture now checks the actual indirect access
strategy: resident fallback is allowed to be short. This is an intentional
backend contract assertion update; frontend/NIR shapes and printing are unchanged.

The scoped backend library suite passes 415 tests (one existing ignored), and
all fourteen integration targets pass 88 tests (four existing opt-in tests
ignored). The full native suite passes 379 tests across 95 targets (six existing
opt-in tests ignored). Thirteen baseline/accounting and consumer tool tests pass.
All seven native qualification manifests bind the same 632 compiler, runtime and
fixture inputs. Shared frontend/NIR contracts are unchanged; other backends'
suites were not executed.

The pinned stage-6 optimized-dev CLI has SHA-256
`f0132aa1fb5ec371d1e583da3c7a2a0bb989dd4e015e230001c9d5cb70624ae8`.
[results.json](results.json) retains routine and per-vector gates and paired host
samples; [compiler-inputs.json](compiler-inputs.json) records the source generation.
[evidence-sha256.json](evidence-sha256.json) authenticates compact evidence and
seven compressed qualification manifests.

## Compiler cost

Three serial measured rounds follow one warm-up per compiler/profile. Separate
optimized-dev CLI processes use opt-level 3, debug 0 and no incremental
compilation; compiler and profile order alternate. Per-child `wait4` reports
wall time and peak RSS, and every build must reproduce its compiler-specific
pinned image. No build or native qualification runs alongside these measurements.

| Profile | Stage 5 wall (s) | Stage 6 wall (s) | Wall change | Stage 5 RSS (MiB) | Stage 6 RSS (MiB) | RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 69.338 | 69.106 | -0.34% | 874.4 | 872.5 | -0.22% |
| Optimized guarded | 56.057 | 46.331 | -17.35% | 900.2 | 893.8 | -0.70% |
| Raw guarded | 48.429 | 49.255 | +1.71% | 1,026.3 | 1,047.2 | +2.03% |

Every profile meets the incremental median limits of 5% wall-time growth and
10% peak-RSS growth. Median corpus wall time is 172.659 seconds before and
170.230 after. Shared-host timings vary substantially: guarded before samples
range from 44.464 to 63.179 seconds and after from 45.788 to 79.229; individual
pairs can be slower despite a passing median. These results qualify the agreed
cost bounds and do not establish a stable compiler speedup. All original samples
are retained, with no follow-up required to meet the median gates.

## Reproduction

Stage 5's exact 579 compiler/test/tool inputs were verified before work and
archived at `target/record-placement-stage6/stage5-inputs.tar.gz`. Preserve the
stage-0 sources and pinned stage-5 CLI, images and native measurements.

Build the CLI and `record_probe --features placement-analysis` with opt-level 3,
debug 0 and incremental compilation disabled. Pin the CLI at
`target/record-placement-stage6/actionc-65816`. Use the existing consumer probe,
vector generation and references commands with the stage-6 output directory.
Run the scoped backend unit and fourteen integration targets, full native suite
and each candidate/reference profile through the qualified native runner.

After builds and native qualification finish, measure serial paired costs and
publish:

```sh
python3 -B tools/compare65816/exec_record_consumer.py host \
  --stage 6 --base target/record-placement-stage0 \
  --previous target/record-placement-stage5 --output target/record-placement-stage6 \
  --binary target/record-placement-stage6/actionc-65816 --rounds 3
python3 -B tools/compare65816/exec_record_consumer.py publish \
  --stage 6 --base target/record-placement-stage0 \
  --previous target/record-placement-stage5 --output target/record-placement-stage6
```

Hosted unchecked-release and standalone fixture gaps remain stage-7 obligations.
This qualification makes no hosted whole-application or hardware performance
claim.
