# Native 65816 emission simplification

Implementation follows the [simplification plan](MIR65816_EMISSION_SIMPLIFICATION_PLAN.md).
The qualified foundation compiler is `e4fd88b5`; main `902a7848` adds only planning
documentation. Historical machine-code and foundation evidence remain immutable.

## Measurement boundary

The [frozen baseline](benchmarks/65816-emission-simplification/baseline.json)
authenticates all 470 foundation source/fixture inputs, 658 native artifacts in
each host profile and the complete saved comparison reports. The before release
CLI was built with Rust 1.95.0, no optional features and incremental compilation
disabled, and retained under `target/emission-simplification-before/`.

`emit/work.rs` exists only in tests and `native65816-state-proof` builds.
`proof::measure_work` counts actual operations within one synchronous calling-thread
scope. Other threads and nested/unwound scopes cannot contaminate a measurement.
The counters never appear in image or historical trace metadata. Ordinary release
CLIs contain no work collector. Counts distinguish home-access construction,
each dataflow analysis, CFG construction, original-load expansion, full/prefix
replay, actions visited by each walk and layout calls.

The ignored native `simplification` target exports 28 corpus builds and 16
generated size-ladder builds. Chains contain 16/32/64/128/160 updates; the second
family contains 4/8/16 conditional updates within a loop. Both source modes
retain increasing emitted work. Every ladder image executes five independent
boundary inputs in both incoming interrupt-mask states, checks CPU/stack/domain
guards, and exactly matches compilation through CRLF source text. The exporter
saves the actual serialized images, source and resolved layout outside the
historical artifact schemas.

From the repository root, with a fresh output directory:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
A816_SIMPLIFICATION_DIR="$PWD/target/emission-simplification-before/probes" \
python3 -B tools/native65816-runtime-tests/qualify.py --release \
  --test simplification -- --ignored
python3 -B tools/compare65816/simplification.py \
  target/emission-simplification-before/probes \
  --compiler target/emission-simplification-before/actionc-65816 \
  --output target/simplification-before-work.json
```

The summary tool verifies build cardinality, source/image hashes, LF/CRLF and
execution claims, and increasing code size in each family/mode. It also writes a
16-build host manifest. Use `measure_host.py --expected-builds 16 --rounds 3`
with that manifest for the ladder, and its default 28-build/seven-round settings
for the comparison corpus. Measure preserved before and final after CLIs together
after compilation/qualification jobs finish. Every invocation must reproduce its
baseline image hash; measured processes have no counters enabled.

The host-tool controls use real child processes to verify per-child accounting
and reject changed output hashes or incorrect build cardinality. Timing values
are observations, not CI thresholds. Saved before/after reports include all
samples, binary/tool/source hashes and the exact methodology.

## Removed migration scaffolding

The unused shadow runner is deleted. Synthetic identity transactions and their
one-shot bookkeeping exist only in unit tests. Reference-selection arguments,
the tracked reference flag and exact reference-output comparison exist only in
qualification builds; ordinary emission has one checked route. The active
independent reference/replay tests and the frame/incoming witness implementation
remain. Dead-code allowances are confined to the retained proof query surface.

The ordinary CLI reproduces all 28 Action images, including CRLF compilation.
The [full corpus gate](benchmarks/65816-emission-simplification/s1-equality.json)
preserves all 224 comparison artifacts and 264 records in both host profiles,
including only the known optimized vbcc unlink vector-0 failure. Four scoped
native checked-rewrite/replay tests retain their two historical artifacts.

## Analysis demand

Each immutable snapshot owns private `OnceCell` results for home liveness,
stored definitions and machine liveness. Fallible home resolution remains eager.
Queries validate sites before demanding a solver; repeated queries share only
that snapshot's result. New generations construct empty cells. No global cache,
analysis registry or cross-generation reuse is introduced.

The driver's stored-definition and undefined-read postconditions remain in
place. Adjacent-load transactions no longer run the unused home/machine liveness
solvers. New controls compare every exposed query with eagerly computed results
across calls, loops and pointer accesses, reject stale sites without triggering
analysis, and check exactly one solver run per demanded result.

## Original-stream reconstruction

Selection retains each actual load before computing the local projection, and
keeps its blocker diagnostic for observations. The adapter validates candidate
order, exact consume inputs and complete request records. It does not construct
another analysis context or repeat the local equivalence proof.

All projected-away loads are inserted in one traversal; one ordinal map updates
parent and request-end links, and one selected constructor checks the expanded
CFG. Empty expansions keep the existing generation. Record creation and symbolic
reindexing are shared with the driver's replacement operation. The former
per-load reconstruction is test-only and serves as an independent ordering oracle.

Fresh replay verifies the complete original continuation. Each removal still
requires the sealed rule and driver transaction. Rediscovery treats original
ordinals only as locators, checks exact request/load correspondence and mints a
site for the current generation. Planning diagnostics cannot authorize removal;
a withheld final allocation identity still keeps the actual load. Selection's
projection remains necessary because instruction cursors and X fallback choices
influence later selection.

## Replay validation boundary

Every initial, edited and replayed selected routine constructs its graph from
private actions. Public-to-the-emitter queries expose shared immutable slices;
layout only remaps encoded ranges. The constructor audit found no action/graph
mutation escape. Prefix and full replay now use that verified input directly.
They retain fresh decision, action, child and environment checks, and full replay
still constructs and verifies its own output graph. Code reconciliation remains
at transaction entry and publication because byte metadata is separately owned.

Mutation controls exercise all constructors with malformed labels, request ends,
parents, modes and stack equations. Measured controls require zero graph builds
for prefix replay and exactly one output graph build for full replay. No caller
flag, persistent validation cache or skipped output check implements this change.
