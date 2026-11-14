# MIR6502 phase tracing

`actionc-emit` can record the actual full code-generation run without changing
its optimizer settings, reporting mode, emitted output or compilation schedule:

```sh
mkdir -p build/traces
cargo run --bin actionc-emit -- --profile modern --backend mir6502 \
  --emit-listing --trace-mir6502 build/traces/tn \
  --trace-routine FormatDirectoryRow --trace-routine SetWin \
  samples/tn/modern/TN.ACT > build/traces/TN.asm
```

The destination must not exist; its parent must exist. Repeat `--trace-routine`
with exact routine names or IDs such as `r12`. Selectors refer to routines in
initially lowered MIR, before generated helpers and inlining. An unknown or
ambiguous selector is an error. Omitting selectors records every routine.
Filtering affects diagnostics only. In the manifest, `selected_routines` is the
resolved selector set; an empty set without filter errors means unfiltered. The option belongs to `actionc-emit` and
requires full MIR6502 generation; other backends and early IR dump modes are
rejected. Existing peephole-report settings remain independent.

The bundle contains:

- `manifest.json`: schema version, effective backend configuration reference,
  source/compiler identities, completion status, work counts, detail budget,
  final snapshot and successful final layout references.
- `events.jsonl`: append-only event IDs, attempt IDs and typed diagnostic facts.
- `snapshot-NNNNNN.mir`: existing MIR text for each captured snapshot.
- `home-summary.json`: final explicit accesses, physical placements and references
  to home demand, remaps and cleanup outcomes in the journal. Access-count
  changes across the returned outer pipeline identify later consumer removal.

Identities use FNV-1a for diagnostic correlation, never optimizer proofs or
cache keys. The qualification harness uses SHA-256 for artifact checks. The
root source identity covers decoded source; the backend NIR identity also
reflects included input. No timestamps or absolute output paths are recorded.

The default detailed journal budget is 32 MiB. `--trace-max-bytes N` overrides
it, including zero for a metadata-only truncated run. The budget measures encoded
records, including snapshot text, rather than allocator/RSS usage. One record
is constructed at a time. After truncation, detailed collection stops while
bounded completion metadata and work counts continue. The derived summary
has a separate 256 KiB output cap and identifies an omitted index explicitly.
Large bundles belong under ignored `build/`.

A manifest distinguishes `complete`, `truncated`, `compilation-failed`,
`filter-error`, `trace-write-failed`, and unfinished `incomplete` runs. A failed
write is a CLI error; recording its failure status is best effort if the
filesystem itself is unavailable. Frontend failures also leave a failed manifest.
A partial trace is useful for inspection but does not qualify a diagnosis.

Optimized executable generation now includes `late-private-store-cleanup` after
T10 checked-range retention. Its input/proposed/returned snapshots and
`late-cleanup-result` distinguish accepted savings from rollback;
`late-cleanup-store-retained` explains physical read blockers. The historical
stage name also covers the byte-producer extension. Accepted outcomes report
`removed_stores` and `removed_producers` separately; removed MIR operations are
not themselves an emitted-byte saving. Final cost decides acceptance. Logical
discovery, physical deletion, final-placement replay and earlier-decision validation have
separate diagnostic scopes. See the [cleanup contract](MIR6502_LATE_CLEANUP_DESIGN.md).
The opt-in `late-spill-pruning` transaction reports candidate, unreferenced and
removed spill counts separately. Its `late-spill-pruning-result` records core
allocation, exposure, physical placement and earlier-proof rejection reasons.
A rejected allocation experiment retains the accepted store/producer cleanup.
`--trace-core-layout` adds an opt-in planning-address census. `core-layout-round`
events associate observations with actual applied rewrite rounds; they are
conservative envelopes that can include rejected-candidate queries. Queries
outside the shared drivers produce `core-layout-unattributed` events. Neither
kind is proof metadata. Check `core_layout_census.status` as well as the overall
trace status before interpreting coverage. See the
[census contract](MIR6502_LATE_CLEANUP_DESIGN.md#core-allocation-dependency-census).
The historical materialized-MIR dump still ends before T10 and terminal cleanup;
the exact `pre-emission` snapshot includes all selected terminal work.

## Reading evidence

Attempts form a tree, not a linear list of optimization passes.
Starts labeled `context` group child work and inherit their completion from the
parent; starts labeled `attempt` have their own completion record. Nested successes
remain speculative until selected by every parent. Selection records carry
accepted/rejected routine IDs; returned snapshots show the actual composite
program after rollback. Equivalent source snapshots are explicitly labeled
as equivalent, rather than guessed to be the unique supplier.

An operation coordinate belongs to its snapshot, or to a driver invocation plus
its generation. A generation number alone is insufficient. Home IDs belong to a
routine and representation; coloring may put distinct values in the same home.
Follow typed remap records for exact correspondence. Changes without operation
maps are comparisons between snapshots, not persistent operation identity.

`compat-spill-stop` observes the compatibility scan's actual short-circuit path.
`home-definition-reader` identifies the first reader outside the queried deletion
window. `rewrite-blocked` preserves the proof's typed reason. None of these
records runs a new proof or makes a rewrite eligible.

`emitted-range` associates final MIR operations and terminators with offsets
from the emission origin. Empty ranges are explicit, including suppressed loads
and tail-call terminators. They establish that an operation emitted no bytes;
they do not establish general deadness. `final-home` uses the final emitter's
own layout, including declarations with no explicit accesses. Final layout is
qualified by successful relocation and output construction in the manifest.

Executable placement trials can contribute several complete emissions to the
journal. The manifest's `final_snapshot` and `final_layout`, together with
`final-artifact-selection`, identify the selected artifact, including rollback
to an earlier emission. Filter `emitted-range` and `final-home` by that snapshot
ID. The home summary performs this join for final placements and zero-byte
reads; speculative records remain available as history. A missing snapshot after
trace truncation is never replaced by an earlier trial's ID. This selection uses
the existing emitted artifact and does not request another emission.

Parameter placement has two bounded trials: leaf selection and the optional
terminal parameter-to-argument handoff. `parameter_home_trials` counts both;
`parameter_argument_handoff_trials` identifies the handoff subset. Its
`parameter-argument-handoff-result` records final acceptance or fallback relative
to the selected leaf artifact. An empty candidate set records a skipped stage
and adds no continuation. Follow final artifact identity after either trial;
an accepted leaf result is not necessarily the final selected executable.

Zero explicit reads, opaque effects, and zero emitted bytes are separate facts.
The summary retains attempt IDs and does not count speculative removals as final
savings. Use final snapshots and selection records when following its references.

The API is additive: `mir6502::generate_output_with_trace` takes a caller-owned
`Mir6502Trace`; existing entry points use a disabled context. The backend chooses
no filesystem paths. Work counters cover actual core materializations, pipeline
attempts, emission invocations and inner probes. Analysis-build counts cover the
instrumented shared local rewrite drivers, not every compiler analysis.

See the [implementation plan](MIR6502_PHASE_TRACING_IMPLEMENTATION_PLAN.md) and
[pipeline audit](MIR6502_OPTIMIZATION_PIPELINE_AUDIT.md) for boundary ownership.


The word-result destination proof also emits
`ordinary-range-outside-zero-page`. This is a declaration/extent/budget result,
not a numeric predicate: its input address is null because no address was
queried. Count `address-query` events separately when measuring numeric layout
dependence. Unsupported shapes can emit a false structural observation followed
by explicit address queries. Round envelopes still include rejected candidates.


Pointer candidate scopes add an optional `candidate` key to observations and a
`candidates` array to `core-layout-round`. Keys contain family/block/start; each
shape records its exclusive end and latest exact driver outcome (`candidate`,
`overlap-rejected`, `applied`). `shape-only` and `no-shape` are not accepted plans.
The containing attempt/routine/generation forms the rest of the identity. SSA
prefix queries have no candidate key and remain shared analysis. These scopes
cover local gates only; inherited facts and parent selection still matter.
`fits-fixed-zero-page-pair` records a resolved pointer-pair choice. Structural
bounds observations remain separate from numeric queries.

With both spill pruning and the layout census enabled, a rejected core allocation
can emit `core-allocation-movement`. It links the baseline/proposed snapshots and
reports whole-program planning-frame changes, including later routines regardless
of routine-detail filters. Its own `delta.status` and `delta.omitted` must also be
checked: detail is capped at 2,048 changes. Counts include alias views and are not
unique physical objects or byte savings. No candidate emission or proof replay is
performed by this diagnostic. Candidate attribution additionally covers local
selectors for direct fixed-pointer placement, direct word-to-indirect copying
and word-array value staging; shared structural analysis remains separate.
