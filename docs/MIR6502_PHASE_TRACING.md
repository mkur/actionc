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
Filtering affects diagnostics only. The option belongs to `actionc-emit` and
requires full MIR6502 generation; other backends and early IR dump modes are
rejected. Existing peephole-report settings remain independent.

The bundle contains:

- `manifest.json`: schema version, effective backend configuration reference,
  source/compiler identities, completion status, work counts, detail budget,
  final snapshot and successful final layout references.
- `events.jsonl`: append-only event IDs, attempt IDs and typed diagnostic facts.
- `snapshot-NNNNNN.mir`: existing MIR text for each captured snapshot.
- `home-summary.json`: final explicit accesses, physical placements and references
  to home demand, remaps and cleanup outcomes in the journal.

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

## Reading evidence

Attempts form a tree, not a linear list of optimization passes. Nested successes
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
