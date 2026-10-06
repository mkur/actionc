# Native 65816 record/value placement: stage 2

Stage 2 establishes one checked owner for existing placement decisions and
preallocation operation resources. It preserves target code and runtime costs;
mixed residence improvements are subsequent consumer work. The
[placement contract](../../MIR65816_PLACEMENT_CONTRACT.md) defines the boundary,
and the [implementation plan](../../MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md)
defines the remaining stages.

The frozen [stage-0 workload](../65816-record-placement-stage0/README.md) uses
compiler baseline `d7d536c9`, Exec816 `57df0d7`, 256 source files and 1,156
emitted routines. The candidate is identified by complete working-tree source
hashes in [compiler-inputs.json](compiler-inputs.json).

## Placement and resource gates

All three profiles have byte-identical compiler images and complete inventories,
including temporary homes, frames, stack peaks, source spans, labels, fixups and
branch encodings. Existing profitable leaf output and bank-zero reservations
remain unchanged.

| Profile | Compiler code bytes | Planned bodies | Values | Resource windows | Record/memory windows |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 444,948 | 1,075 | 29,684 | 46,007 | 20,130 |
| Optimized guarded | 594,828 | 1,075 | 29,684 | 46,007 | 20,130 |
| Raw guarded | 661,861 | 1,075 | 37,586 | 56,065 | 26,169 |

Each profile has 72 terminal-forwarding bodies and nine arithmetic helpers
under their existing independently checked contracts. They are explicitly
opaque in the placement report. External implementations are not compiler
selected bodies. All eleven frozen representatives have checked common plans
in [placement-representatives.csv](placement-representatives.csv); the full
census is in [placement.csv](placement.csv). The separate logical census remains
in [routines.csv](routines.csv) and [representatives.csv](representatives.csv).

The plan distinguishes materialized homes, borrowed input homes, register
intervals, deferred components and redirected destinations. Every logical read
has one admitted location. Complete resource windows include address setup and
the final field access. Indexed, aggregate, volatile, other arithmetic and call
forms remain explicit barriers. Whole-operation register bounds stay
conservative and grant no narrower lifetimes or new memory alias proofs.

Ten independently authored placement tests reject missing or forged homes,
partial widths, uncovered uses, stale planner objects, illegal register
intervals, scratch conflicts, forged resource extents, inconsistent CFG
requirements, malformed dense resource tables and reuse across
compilation/allocation instances. Sealed contracts are checked against
authoritative typed effects after selection, replay and final
rewrites. Existing parallel-transfer, pointer-handoff, ABI and machine-state
proofs remain in force.

All 147 independent list/record vectors pass in each profile with both I=0 and
I=4: 882 executions. Per-vector outputs, cycles, private memory traffic, stack
peaks and observed control flow match stage 0 exactly. The full qualified native
suite passes: 364 tests across 91 targets, with six existing opt-in tests
ignored. Backend library tests pass (393, one existing ignored), as do all
14 backend integration targets (88 tests, four existing opt-in inventory tests
ignored). The ten focused placement tests and immutable-borrow compile-fail
doctest pass.

Actual module-loader compilation from LF and CRLF record fixtures produces
identical images in both raw and optimized modes. Five baseline/accounting and
vector/tool-input tests also pass. Shared frontend/NIR contracts are unchanged;
other backends' suites were not executed.

The four compressed CPU qualification manifests retain full compiler, fixture
and VM source hashes. The VM is the pinned `56ddc5c5` plus the committed
status-timing correction used in stage 0.

## Compiler overhead

Paired CLI runs use optimized dev settings matching stage 0: opt-level 3,
debug information disabled and incremental compilation disabled. Each
compiler/profile has one warm-up and five measured rounds with alternating
compiler and profile order. Runs are serial after all builds, observer and VM
checks; every output must match the frozen image hash. Per-child `wait4` supplies
peak RSS.

| Profile | Baseline median seconds | Candidate median seconds | Wall ratio | Peak-RSS ratio |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 54.501 | 55.194 | 1.013 | 1.009 |
| Optimized guarded | 39.415 | 40.147 | 1.019 | 0.995 |
| Raw guarded | 44.128 | 45.540 | 1.032 | 1.001 |

Every profile meets the foundation limits of 1.05 wall time and 1.10 peak RSS.
These observations on a shared macOS host do not establish a compiler speedup.
Observer work is excluded from CLI timings.

[results.json](results.json) records exact samples, qualification summaries and
binary/source/evidence hashes. [evidence-sha256.json](evidence-sha256.json) covers
the compact publication. Bulky artifacts and logs remain under
`target/record-placement-stage2/`.

## Reproduction

Freeze and reproduce stage 0 using its documented tools. Keep the snapshot
unchanged while checking a candidate. Build the observer from the repository
root:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=3 \
CARGO_TARGET_DIR=target/record-placement-stage0/rust-target \
cargo build --locked --manifest-path tools/compare65816/record_probe/Cargo.toml \
  --features placement-analysis
python3 -B tools/compare65816/exec_record_foundation.py probe \
  --base target/record-placement-stage0 --output target/record-placement-stage2 \
  --binary target/record-placement-stage0/rust-target/debug/actionc-exec-record-probe
```

The opt-in feature reports sealed placement contracts separately and keeps the
ordinary observer compatible with the frozen compiler. Build `actionc-65816`
with the same optimized-dev settings and run `exec_record_foundation.py host`
with its pinned CLI as `--binary` and `--rounds 5`.

Run the scoped backend library, placement and integration tests, immutable-borrow
doctest and full native suite through `tools/native65816-runtime-tests/qualify.py`.
For each profile, qualify `--test code_quality -- --ignored` with
`A816_COMPARISON_MANIFEST` pointing at its frozen stage-0 manifest and
`A816_COMPARISON_RESULTS` at the stage-2 measurement destination. This recompiles
with the candidate and checks image equality before executing both interrupt
masks. Retain successful logs and manifests, then publish:

```sh
python3 -B tools/compare65816/exec_record_foundation.py publish \
  --base target/record-placement-stage0 --output target/record-placement-stage2 \
  --stage 2
```

Stage 0's unchecked hosted-provider rejection, standalone list/DOS fixture gaps
and raw port restoration failure remain separate recorded obligations. Stage 2
claims no additional unchecked hosted-release qualification.
