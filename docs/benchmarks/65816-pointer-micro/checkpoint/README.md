# Checkpoint after pointer slices 1–8

Compiler `843b3b0b`, compared with the retained native-wide-return baseline
`87feebf1`. This is a compiler measurement checkpoint. Exec was rebuilt from
frozen `c3500c8`; its hosted runtime was not qualified or its compiler pin changed.
Full Exec qualification is reserved for the final implementation commit.

| Artifact | Raw before → after | Optimized before → after |
|---|---:|---:|
| Complete list module | 3,635 → 3,288 | 3,557 → 3,106 |
| Dijkstra executable bytes | 5,122 → 5,069 | 4,535 → 4,449 |
| Frozen Exec executable bytes | 418,476 → 406,353 | 385,834 → 373,603 |
| Frozen Exec XEX bytes | 433,774 → 421,417 | 400,504 → 388,039 |

Exec saves **12,123 raw / 12,231 optimized executable bytes**. Guard totals are
unchanged at 62,856 / 62,640 bytes. Every compared routine retains its fixed
frame, spill bytes, local stack peak and number of DP home bytes. Reserved
bank-zero change is **zero**, including alignment, guards and unused capacity:
the public task pools remain 2,080 bytes for the first task and 1,568 for each of
seven others; idle remains 1,056; fixed runtime remains 10,560. Runtime including
the OS remains 61,536 bytes and loading including the OS remains 57,232.

The fourteen-kernel corpus changes only `byte_sum` (−21 raw / −20 optimized),
`unlink` (−6 / −6) and `forward_copy` (−40 / −40). Other code sizes and all guard,
frame, peak and DP-home counts are unchanged. The current corpus passes all 132
paired-mask records per host (264 executions), with identical debug/release
results. This archive reports current cycles, not an inferred historical corpus
cycle delta. Replaying the old artifacts with the current harness was rejected
by its compiler-artifact equality check; those partial results are excluded.

Dijkstra passes all 33 vectors in both modes and both incoming I states:
66 paired-mask records / 132 executions on the release host. This includes the
full original benchmark, independent graph results and queue boundary probes.
For `original-0-50`, the authenticated historical baseline gives:

| Metric | Raw before → after | Optimized before → after |
|---|---:|---:|
| Cycles | 107,757,271 → 107,908,118 | 99,562,389 → 98,270,190 |
| Stack reads | 7,964,596 → 8,169,585 | 6,360,514 → 6,661,192 |
| Stack writes | 6,519,777 → 6,519,777 | 4,149,509 → 4,144,067 |
| Peak below entry S | 76 → 76 | 70 → 70 |

The raw size reduction has a **0.14% cycle regression**; optimized cycles improve
by **1.30%**. DP traffic and metadata reads are unchanged. These are measured
tradeoffs from the complete first group, not an attribution to one slice.
The historical cycle comparison is limited to this graph; current measurements
for every graph are retained separately.

[Whole sizes](sizes.csv), [per-routine sizes and storage](routine-sizes.csv),
[corpus counters](corpus-results.csv), [Dijkstra counters](dijkstra-results.csv),
[historical Dijkstra deltas](dijkstra-original-0-50-delta.csv) and
[provenance](provenance.json) retain the results. Lists remain in the preceding
[slice archives](../README.md). Foreign compiler output was rebuilt for artifact
comparison but not re-executed here; recorded vbcc failures remain in the
[original analysis](../../65816-execlists/README.md).

The complete native VM suite passed **206 tests in each host profile**, with
four opt-in measurements ignored. The unit/integration checks and focused
slice-8 VM coverage are recorded in the parent report. This checkpoint adds no
new compiler behavior and requires no repeat of those passing suites.

## Reproduction and controls

Use `tools/compare65816/build.py` and `dijkstra.py` with the slice-8 compiler,
separate output directories and `--verify-crlf`. Their actual LF/CRLF artifacts
match. Filter manifests to Action artifacts for execution; run `code_quality`
through `tools/native65816-runtime-tests/qualify.py` in debug and release, and
run the complete opt-in `dijkstra` target in release. Set the respective
`A816_COMPARISON_MANIFEST/RESULTS` and `A816_DIJKSTRA_MANIFEST/RESULTS` variables.
Provenance retains commands, VM revision/patch, qualification manifest hashes,
input hashes and compiler binary hashes. Some qualification manifests name the
preceding commit because slice 8 was still uncommitted when they started;
the qualified compiler and fixture content hashes match slice 8.

Exec builds use `tools/native_program.py` from the frozen checkout, with
`--allow-compiler-override --tasks --task-capacity 8 --console`, the explicit
baseline DOS mounts, and `--no-opt` for raw. Retain the existing checked-layout
compatibility adapter: it removes only `stack_checks: true` from the generated
layout supplied to actionc, whose guards are enabled by default; it rejects
unchecked layouts. The baseline mount list is retained in provenance. The
initial build accidentally used the now-empty default mount configuration;
those numbers were discarded and both modes rebuilt with identical mounts.
All generated Action source, source/platform/ABI inputs, mount configuration and
bank-zero budgets now match the baseline. The reporter verifies these controls
and every entry/call guard rather than assuming guard bytes from code size.

Archive with:

```sh
python3 -B tools/compare65816/report_pointer_checkpoint.py \
  --input target/pointer-micro/checkpoint \
  --baseline target/wide-returns \
  --exec-build-root /path/to/frozen/exec/build \
  --output docs/benchmarks/65816-pointer-micro/checkpoint
```

The reporter requires the retained artifacts and completed qualification logs.
It checks their hashes, complete vector sets, successful results, mask coverage,
source equality, guard counts and unchanged build inputs before writing tables.
