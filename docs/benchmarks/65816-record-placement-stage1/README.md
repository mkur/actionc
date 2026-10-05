# Native 65816 record/value placement: stage 1

Stage 1 establishes immutable logical value, CFG and invocation-storage facts
before native placement. It adds validation and checked queries, with no target
code improvement claimed. The [contract](../../MIR65816_LOGICAL_ANALYSIS.md) and
[implementation plan](../../MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md) describe its
scope and the next resource/placement stage.

The workload, generated sources, runtime and external inputs are the frozen
[stage-0 inputs](../65816-record-placement-stage0/README.md): compiler baseline
`d7d536c9`, Exec816 `57df0d7`, 256 source files and 1,156 routines. The candidate
is a working-tree compiler change, identified by complete source hashes in
[compiler-inputs.json](compiler-inputs.json), not by a new Git revision.

## Exit gates

All three profiles have byte-identical compiler images and complete MIR/emission
inventories, including routine/data metadata, temporary homes, frame maps,
spans, fixups, labels and branch encodings. Bank-zero and stack requirements are
unchanged. The analysis checks 1,147 routine bodies in each profile; nine
external/helper implementations remain opaque under their existing ABI checks.
All eleven frozen representative routines are included in
[representatives.csv](representatives.csv); the full census is in
[routines.csv](routines.csv).

| Profile | Compiler code bytes | Logical temporary values | Reachable uses checked |
| --- | ---: | ---: | ---: |
| Optimized release | 444,948 | 29,881 | 33,738 |
| Optimized guarded | 594,828 | 29,881 | 33,738 |
| Raw guarded | 661,861 | 37,783 | 38,018 |

The value counts include computations and block parameters, not just loads.
Individual load-capture counts, identity counts, edge mappings, cyclic blocks,
maximum logical liveness and solver evaluations are recorded separately. Loop
writes have unknown dynamic versions; capturing a pointer proves no pointee
extent, alignment, disjointness or unchanged field contents.

All 147 independent list/record vectors pass in each profile with both I=0 and
I=4: 882 executions. Saved per-vector outputs, cycles, private traffic, stack
peaks and observed control flow match stage 0 exactly. The full qualified native
VM suite passes: 364 tests across 91 targets, with six existing opt-in tests
ignored. Backend library tests pass (382, one existing ignored), as do all 14
backend integration targets (88 tests, four opt-in inventory tests ignored).
Seventeen independently authored logical-graph tests and an immutable-borrow
compile-fail doctest pass. Actual module-loader compilation of the independent
record source produces identical images with LF and CRLF, in both modes.

The four compressed CPU qualification manifests retain their full compiler,
fixture and VM source hashes. The VM is the pinned `56ddc5c5` plus the committed
status-timing correction used in stage 0. No other backend's test suite was
executed; shared frontend/NIR contracts are unchanged.

## Compiler overhead

Paired warm CLI measurements use the same optimized dev settings as stage 0:
opt-level 3, debug information disabled and incremental compilation disabled.
Each compiler/profile has one warm-up followed by three measured rounds, with
compiler and profile order alternating. Builds run serially after compiler and
VM checks; every result must match the frozen image hash. Peak RSS comes from
per-child `wait4`, not cumulative process accounting.

| Profile | Baseline median seconds | Candidate median seconds | Wall ratio | Peak-RSS ratio |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 55.519 | 54.790 | 0.987 | 0.876 |
| Optimized guarded | 40.250 | 39.975 | 0.993 | 1.045 |
| Raw guarded | 45.099 | 43.929 | 0.974 | 0.981 |

Every profile meets the foundation limits of 1.05 wall time and 1.10 peak RSS.
The apparent reductions are measurement variation on a shared macOS host, not
a claimed speedup. The extra observer's direct analysis timings are separate
from these CLI runs and appear in the routine CSVs; they are warm single-pass
observations, not paired throughput measurements.

[results.json](results.json) contains exact samples, binary/source/evidence
hashes and qualification summaries. [evidence-sha256.json](evidence-sha256.json)
covers the compact publication. Bulky images, inventories, logs and timing
artifacts remain under `target/record-placement-stage1/`.

## Reproduction

Freeze/reproduce stage 0 first using its documented tools. Keep its snapshots
unchanged while checking a candidate. From the compiler repository root:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=3 \
CARGO_TARGET_DIR=target/record-placement-stage0/rust-target \
cargo build --locked --manifest-path tools/compare65816/record_probe/Cargo.toml \
  --features logical-analysis
python3 -B tools/compare65816/exec_record_foundation.py probe \
  --base target/record-placement-stage0 --output target/record-placement-stage1 \
  --binary target/record-placement-stage0/rust-target/debug/actionc-exec-record-probe
```

The optional feature keeps the ordinary stage-0 probe compatible with its frozen
compiler. It observes checked logical facts after compiling; its observer cost
is excluded from the CLI timing gate. Build `actionc-65816` with the same compact
optimized-dev settings, then run `exec_record_foundation.py host` with that CLI
as `--binary` and `--rounds 3`.

Run the scoped library and explicit backend integration checks, and qualify the
full native suite using `tools/native65816-runtime-tests/qualify.py`. For each
of the three profiles, qualify `--test code_quality -- --ignored` with
`A816_COMPARISON_MANIFEST` pointing at its frozen stage-0 manifest and
`A816_COMPARISON_RESULTS` pointing at the stage-1 measurements file. This
recompiles with the candidate and checks image equality before execution.
Retain successful qualification logs and manifests, then run
`exec_record_foundation.py publish` with the same `--base` and `--output`.

Stage 0's unchecked hosted-provider rejection, standalone list/DOS fixture gaps
and raw port restoration failure remain recorded obligations. Stage 1 neither
resolves those gaps nor claims new unchecked hosted-release qualification.
