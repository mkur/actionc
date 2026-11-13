# MIR6502 late private-store cleanup implementation plan

Status: in progress, 2026-11-13.

Implements the [design contract](MIR6502_LATE_CLEANUP_DESIGN.md). Each slice is
committed separately. Compiler changes stay within MIR6502; existing user work
and unrelated backend changes remain outside these commits.

## Dependency inventory and implementation choice

Core materialization uses `MaterializeLayout` for storage identities, fixed
backing, scratch exclusions and ordinary-storage bounds. Those inputs depend on
origin and declarations, not instruction count. Captured comparisons, scalar
captures and constructor storage also validate allocation-dependent aliases.
The cleanup invariant preserves their declarations, mappings, origin and MIR
producer operations. Core cost/inlining trials need no replay because cleanup
does not change their semantic decisions and measures final profitability again.

The late physical-map consumers are record-base reuse, register copies, pointer
retention/promotion, argument staging, pointer-load sharing, A-to-X forwarding,
word-load sharing and checked-range retention. Byte-plus-one fusion uses callee
facts. These require explicit preservation validation.

Retain compilation-local witnesses for actually selected late decisions: a
closed family tag, immutable input and selected result, and family options.
Recheck each family using its existing proof/application logic with the new
physical map, requiring the same selected result. Restrict replay to selected
routines for families with partial rollback. Replays perform no emission,
materialization or profitability selection. Exact source snapshots preserve
operation positions; checked-range evidence is consumed only against its
original snapshot. Unknown family/dependency information declines cleanup.

This deliberately starts with coarse witnesses instead of introducing permanent
operation identities or a general pass manager. Measurements must expose their
memory and analysis costs. A future refinement may replace snapshots with smaller
typed predicates without weakening validation.

Physical zero-page deletion additionally requires a definition-to-overwrite
read proof, including indirect observers inside callees. Default scalar `SArgs`
prologues use their bounded ABI footprint only when binding provenance, the
typed signature and inline descriptor agree. Override bindings and unknown
readers decline deletion. This proof belongs to terminal cleanup and does not
relax the generic memory-effect summaries.

## Slice 1 — contract and plan

- Record this inventory and link the design and plan from the documentation index.
- Preserve the design's store-only scope, complete rollback and two-emission
  budget. The historical materialized-MIR API remains unchanged.
- Validate documentation links and whitespace; commit the documentation.

## Slice 2 — selected-decision preservation

- Add an explicit sidecar at the executable preparation boundary, independent
  of optional diagnostics. Capture only changed, selected late results.
- Expose family-owned replay entry points that consume supplied layouts and
  immutable witnesses, including partial selection and precise entry flags.
- Include references removed by earlier passes in final physical-map queries.
- Test changed placement, rejection, partial selection, and absence of hidden
  emission/materialization. Keep cleanup disabled until its acceptance gates
  and tests are complete; commit this foundation.

## Slice 3 — store-only transaction

- Discover direct byte stores to private spill/virtual-ZP homes with the existing
  analyzed definition-deadness proof. Scan all routines initially; filter cheaply
  before analysis. Preserve frames, producers, CFG, ABI and data.
- Reject observable physical aliases. Rebuild analyses after changes and compare
  whole-program callee contracts, including entry and exit facts.
- Emit baseline and candidate at most once each for physical maps, finalized
  bytes, per-routine instruction counts and packaged image cost.
- Recheck deletion and selected-decision proofs under candidate placement;
  require a smaller image without routine instruction growth. Restore the exact
  baseline on any candidate failure.
- Add focused safety, no-op, growth/rollback, frame-preservation and work-budget
  tests; commit the transaction.

## Slice 4 — executable integration and motivating cases

- Add a configuration gate, enabled by the optimized configuration, and call
  cleanup after T10 under the peephole master gate.
- Add transaction snapshots, typed outcomes, exact savings and work counters;
  keep trace-on/off selection identical and historical dump semantics intact.
- Exercise TN's six stores, independent argument staging and checked ranges in
  cart/standalone modes. Test shifted origins, deferred-data movement, zero-byte
  load controls and selected rollback. Record actual gains and any blockers.
- Run affected integration and VM checks, then commit integration and regressions.

## Slice 5 — backend qualification and documentation

- Run the full affected MIR6502 unit/integration qualification and its runtime/ABI
  consumers, plus listing roundtrips and actual LF/CRLF paths. Use full-fixture
  APIs where entry reachability removes the interesting routines.
- Compare enabled/disabled cleanup for execution, exact image/code/home deltas,
  materializations, emission invocations, inner probes and analysis builds.
  Reuse the frozen tracing corpus; preserve its historical receipts.
- Update the design's implemented contract, phase documentation and a separate
  qualification report with reproducible commands, limitations and results.
- Check documentation and commit qualification. Do not claim completion of the
  motivating optimization while TN/staging remain blocked.

## Completion gates

All slices must preserve verifier-clean MIR, contain no sample-specific matcher,
keep allocation unchanged and avoid extra full materializations. TN/staging
must demonstrate proved store removal and measured final savings. Layout and
callee-contract counterexamples must demonstrably restore the baseline.
Passing local backend checks does not replace the repository's full CI matrix.
