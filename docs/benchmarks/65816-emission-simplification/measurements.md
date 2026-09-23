# Native 65816 emission simplification measurements

Final compiler source: `29bfda5f`. Before: frozen qualified foundation compiler
`e4fd88b5` (retained binary hash in [baseline.json](baseline.json)). Ordinary
release CLIs use the same Rust/toolchain/features; counters are compiled out.
Timing began after all build and native qualification jobs finished. Each child
must reproduce its frozen complete image hash. Samples alternate compiler and
build order after a warm-up; RSS is per-child `wait4` accounting.

## Host measurements

| Input set | Rounds | Before total seconds | After total seconds | After/before | Median peak RSS MiB before/after | Maximum peak RSS MiB before/after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 28 corpus builds | 7 | 0.395 | 0.275 | 0.695 | 7.52 / 7.13 | 14.56 / 12.75 |
| 16 size-ladder builds | 3 | 47.166 | 19.678 | 0.417 | 36.21 / 34.82 | 102.80 / 110.22 |

Totals are medians of complete rounds; per-build values below are medians of
individual child samples. These are observations, not CI performance thresholds.
Do not substitute the older foundation 0.147→0.361-second measurement for this
paired before state. Full samples/provenance: [corpus](host-corpus.json),
[size ladder](host-ladder.json).

Memory does not improve uniformly. The largest raw chain has median peak RSS
95.77→100.56 MiB; the highest observed sample rises 102.80→110.22 MiB. Other
cases mostly decrease, and the ladder median falls 36.21→34.82 MiB. These are
resident high-water measurements, not live-heap profiles. Lazy solves and the
single reconstruction change allocation timing/lifetimes while pre/post
stored-definition facts and scratch replay remain required. The samples do not
isolate the cause of the largest-case increase; no general memory reduction is
claimed and this limitation remains visible rather than normalized away.

## Actual work across 28 corpus builds

| Operation | Before | After |
| --- | ---: | ---: |
| `cfg` | 649 | 290 |
| `full_actions` | 18580 | 18580 |
| `full_replay` | 133 | 133 |
| `home_definitions` | 176 | 150 |
| `home_liveness` | 176 | 0 |
| `homes` | 176 | 150 |
| `layout` | 133 | 133 |
| `machine_liveness` | 176 | 0 |
| `original_expansion` | 75 | 24 |
| `prefix_actions` | 20497 | 9002 |
| `prefix_replay` | 175 | 75 |

Home and machine liveness are still available to queries; the production rule
does not request them. The 150 remaining definition solves are the original and
post-replay checks for 75 accepted removals. The 26 eliminated solves belonged
to redundant adapter contexts. All 133 full replays/layouts and their 18,580
visited actions remain. The 24 expansions each reconstruct an entire routine;
the old path needed one expansion per accepted load.

All 44 source/image pairs and every non-work observation are exactly equal.
The ladder has 160 guarded boundary-input executions, including both incoming
interrupt-mask states, and 16 exact LF/CRLF image comparisons. All 102 corpus
forwarding requests retain their decisions: 75 accepted and 27 blocked.
Raw data: [before](before-work.json), [after](after-work.json),
[equality gate](work-equality.json).

## Size ladder

| Family / size | Mode | MIR ops | Code bytes | Candidates / accepted | Before ms | After ms | After/before | Peak RSS MiB before/after |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| chain_16 | raw | 52 | 388 | 34 / 33 | 173.9 | 78.2 | 0.449 | 21.83 / 17.08 |
| chain_16 | optimized | 36 | 388 | 34 / 17 | 83.1 | 37.8 | 0.455 | 17.20 / 13.48 |
| chain_32 | raw | 100 | 548 | 66 / 65 | 627.0 | 272.0 | 0.434 | 31.59 / 30.89 |
| chain_32 | optimized | 68 | 548 | 66 / 33 | 275.1 | 117.6 | 0.427 | 24.22 / 22.17 |
| chain_64 | raw | 196 | 868 | 130 / 129 | 2464.2 | 1054.7 | 0.428 | 50.58 / 49.84 |
| chain_64 | optimized | 132 | 868 | 130 / 65 | 1031.1 | 429.7 | 0.417 | 44.72 / 42.50 |
| chain_128 | raw | 388 | 1508 | 258 / 257 | 10066.5 | 4238.9 | 0.421 | 89.75 / 83.73 |
| chain_128 | optimized | 260 | 1508 | 258 / 129 | 4170.9 | 1720.0 | 0.412 | 75.44 / 75.52 |
| chain_160 | raw | 484 | 1828 | 322 / 321 | 15980.7 | 6712.5 | 0.420 | 95.77 / 100.56 |
| chain_160 | optimized | 324 | 1828 | 322 / 161 | 6663.5 | 2619.8 | 0.393 | 89.92 / 84.36 |
| branches_4 | raw | 48 | 512 | 26 / 19 | 177.1 | 77.9 | 0.440 | 17.52 / 17.20 |
| branches_4 | optimized | 38 | 434 | 24 / 21 | 163.2 | 74.1 | 0.454 | 16.59 / 16.14 |
| branches_8 | raw | 84 | 736 | 46 / 35 | 562.6 | 238.5 | 0.424 | 28.27 / 28.20 |
| branches_8 | optimized | 70 | 606 | 44 / 41 | 556.3 | 242.7 | 0.436 | 25.47 / 24.09 |
| branches_16 | raw | 156 | 1184 | 86 / 67 | 2054.9 | 862.9 | 0.420 | 48.55 / 39.23 |
| branches_16 | optimized | 134 | 950 | 84 / 81 | 2123.2 | 906.3 | 0.427 | 44.39 / 41.09 |

The larger chains still show superlinear host cost: each accepted edit requires
fresh prefix replay, whole-routine replay/layout and definition postconditions.
Removing duplicate orchestration reduces measured work without eliminating
those costs. Batching edits, transferring prefix state or caching across
generations is outside this change; none is implicitly licensed by these timings.

The public ABI, stack guards, call/helper/alias rules, interrupt reserves, image
v3 and o65 profile v1 are unchanged. The full historical corpus and 658 native
artifacts remain equal; the only external failure remains optimized vbcc
`unlink`, vector 0. See the [qualification record](../../abi/action65816-emission-simplification-qualification.json).
