# MIR6502 late private-store cleanup implementation plan

Status: completed, 2026-11-13.

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

## Completion record

The five planned slices are complete. Commits `b2c06e54`, `08379f12`,
`aabe0126` and `1596bdcd` cover the plan, preservation sidecar, guarded
transaction and default executable integration. Qualification added the separate
`cac00bf6` hardening slice: physical zero-page lifetimes extend through caller
continuations, and proof diagnostic scopes always close. The final qualification
commit contains the harness, receipt and completed documentation.

TN's cart image is 9,982 bytes (18 saved); standalone is 10,089 bytes (18 saved).
Argument staging saves four bytes in both runtimes. Frames remain identical,
full materialization counts do not increase, and the transaction stays within
two extra emissions. Exact counts, unchanged controls, runtime coverage and
resource measurements are in the [qualification report](benchmarks/6502-late-cleanup/README.md).

Validation passed 1,296 MIR6502 unit tests, 127 integration tests, 28 VM tests,
54 CLI artifact comparisons, 20 complete API comparisons and 16 MADS LF/CRLF
roundtrips. Existing optional ignored checks are listed in the report. No shared
NIR/semantic/printer contracts changed.


## Producer extension slices

1. Extend the design with the closed producer shapes, liveness/read-effect
   contract, callee preservation, physical replay and bounded shared loop.
2. Reuse the existing dead-register analysis with separate core/late purity
   predicates. Add a disabled-by-default extension switch, guarded deletion
   accounting and focused positive/negative tests; commit the implementation.
3. Qualify enabled/disabled producer cleanup against the frozen store-only TN
   compiler and independent controls. Add runtime and final-placement coverage,
   enable the optimized default, run the affected backend suites and record
   measured image/work/resource differences in a separate receipt; commit it.

Producer extension completed: `bbb1f5a7` records the contract, `97b50d4d`
implements the shared analysis and tests, and `075aa821` enables the optimized
default. Qualification records the actual final placement, output and runtime
checks in a [separate report](benchmarks/6502-late-producers/README.md).
TN saves another twelve bytes in both runtimes (cart 9,970; standalone 10,077).
The independent pointer-transform fixture saves six bytes. Frames and the
existing materialization/emission budgets are preserved. Local validation passed
1,303 MIR6502 unit tests, 129 integration tests plus the explicit MADS test,
30 VM tests, 54 CLI artifact comparisons, 28 complete API comparisons and
24 MADS LF/CRLF roundtrips.

## Allocation-pruning experiment slices

1. Document the core-allocation gap, the address-stable subset, immutable
   producer-cleaned baseline, candidate/reference rules and bounded costing.
   Commit this design and plan before implementation.
2. Reuse the existing spill-pruning candidate scan with structured references.
   Add exact core/physical preservation guards and a declaration-only terminal
   transaction with rollback, explicit opt-in configuration and trace outcomes.
   Add focused positive and rejection tests; commit the implementation.
3. Qualify TN and independent source/control programs in both runtimes and at
   shifted origins. Check execution, relocation, trace equivalence, frame deltas,
   actual image savings and work. Run the affected backend tests, save a separate
   receipt and commit results. Do not describe TN's candidate bytes as savings
   or claim general compaction when the core address guard rejects it.

The [allocation contract](MIR6502_LATE_CLEANUP_DESIGN.md#allocation-pruning-experiment)
explains why late selected-decision replay alone cannot approve moving live
storage. This slice leaves the established frame-preserving cleanup enabled;
it does not weaken its safety checks to obtain a target image size.

The bounded experiment is complete: `ed007fa7` records the design and plan;
`304025f3` implements guarded declaration pruning, exact planning/emitted-storage
certificates, rollback, trace outcomes and focused regressions. The final
qualification commit adds the reproducible harness and
[separate receipt](benchmarks/6502-late-spills/README.md).

TN remains 9,970 bytes cart / 10,077 standalone: **zero additional saving**.
The constructed direct-MIR control saves one byte; all source controls retain
their producer-enabled output. The flag remains off in both configurations.
General compaction remains unimplemented because the existing preservation
sidecar cannot replay core allocation assumptions after surviving objects move.

Validation passed 1,314 MIR6502 unit tests, 131 integration tests, 31 VM tests,
the explicit MADS test (eight LF/CRLF reconstruction/relocation comparisons),
54 frozen CLI artifact comparisons and 32 complete API comparisons. No shared
frontend/NIR contracts changed.

## Core layout census follow-up — slice 1

Completed as diagnostic-only instrumentation in `d1ca2cb9`, followed by corpus
qualification and an independent positive control. The opt-in observer records
planning-address queries, selected predicate results, typed storage references
and actual applied rewrite sites. Association is conservative per discovery
round; unknown addresses, unattributed legacy queries, speculative parent
attempts and uninstrumented layout owners remain explicit limitations.

The [census report](benchmarks/6502-core-layout-census/README.md) records TN and
independent controls, unchanged full output/work, validation and the next
candidate family: known-callee word-result placement. This completes the census
slice only. It adds no replayable core proof or compaction permission.


## Ordinary word-result proof follow-up

1. Add the declaration/extent/budget contract to the existing design, implement
   a reusable ordinary-range proof, and use it only for known-callee word-result
   destinations. Retain explicit fallback checks and reject unresolved fallback
   addresses. Audit accumulator/callee/structural-plan dependencies. Test bounds,
   excluded storage, relocation invariance, liveness and fallback behavior;
   commit the implementation.
2. Qualify TN and independent controls in both runtimes and shifted origins.
   Compare complete outputs, compiler work and census queries against the frozen
   pre-change compiler; exercise LF/CRLF and execution. Record actual savings and
   remaining dependencies, then commit the qualification. Keep frame compaction
   disabled and retain all existing terminal preservation guards.


Implementation is committed as `51915136`. The typed path eliminates all numeric
queries from TN's 42 locally applied known-callee word-result rounds (324 raw
queries and 270 equality evaluations previously), including `Alloc`. This is
family-level progress; the rest of the pipeline still needs preservation proofs.
The [qualification report](benchmarks/6502-word-result-proof/README.md) records
unchanged complete output/work and zero additional image saving.

Qualification completed: 1,323 MIR6502 unit tests, 134 integration tests, 32 VM
tests, 54 frozen CLI artifact comparisons, 20 full API baseline comparisons and
20 full API trace-off/on comparisons pass. The independent control validates
LF/CRLF and actual high-lane-first execution. No shared IR contracts changed.


## Pointer setup follow-up slices

1. Add bounded candidate attribution to the existing census, link exact driver
   outcomes, and keep SSA prefix facts separate. Capture a diagnostic-only
   baseline and check output/work equivalence; commit this instrumentation.
2. Reuse the existing ordinary-range proof for pointer-pair selection and plain
   pointer-source separation. Keep numeric fallbacks and all preservation gates.
   Test fixed-pair boundaries, unknown/alias storage, actual competing candidates
   and repeated preparation; commit the implementation.
3. Compare TN and independent controls against both the original output baseline
   and the attributed baseline, including both runtimes/origins, LF/CRLF and VM
   execution. Record accepted local gates separately from rejected/shared queries,
   actual image sizes/work, and the remaining interior-spill blockers; commit
   qualification. Compaction remains disabled.

Instrumentation is committed as `31908c9f`; bounds-proof reuse and regression
coverage as `4f2c7980`. The [qualification report](benchmarks/6502-pointer-setup-proof/README.md)
records unchanged full output/work across the original, attributed and final
compilers. TN stays at 9,970 bytes cart / 10,077 standalone. Candidate attribution
corrects the earlier pointer-family interpretation: TN's 65 local pair queries
belonged to unselected dereference candidates, not applied direct rematerialization.
The applied paths are exercised by direct-MIR controls; this slice establishes
no new accepted local numeric-dependency reduction in TN.

The explicit pruning recheck still stops at `CoreAllocation` for the four eligible
interior spills; two further candidates remain excluded by frame exposure.
Next work should identify obligations for that concrete movement before replacing
either the core or physical surviving-address guard. Validation passed 1,329 unit,
135 integration and 33 VM tests, 54 frozen CLI artifact comparisons, 24 full API
comparisons across three versions and four TN pruning-off/on comparisons.
