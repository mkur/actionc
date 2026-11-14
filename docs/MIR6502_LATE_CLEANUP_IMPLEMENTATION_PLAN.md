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

The original implementation added a separate physical-ZP reader walk and a
scalar `SArgs` exemption. The ownership correction below supersedes that policy:
the shared home-definition analysis supplies lifetime proofs, while explicit
source storage and physical ABI/assembly effects remain protected. There is no
second rule making a private temporary observable solely because it occupies ZP.

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

## Concrete movement audit and ownership correction

The diagnostic slice `a898ac04` reports actual old/proposed planning entries on
`CoreAllocation` rejection and attributes the three selectors implicated by the
mixed rounds. The [audit report](benchmarks/6502-spill-movement/README.md) records
the evidence and reproduction commands. Removing TN's four candidate bytes
would move 50 surviving entries in 12 routines in cart mode, or 51 in 13 routines
standalone. Full output and compiler work remain unchanged in all 12 cases.

The audit changes the proposed next step. A source pointer allowed to observe
compiler state could target an ordinary temporary as well as fixed ZP scratch;
staging does not remove that issue. The existing 6502 rules mix a private-home
non-escape contract with stronger physical-ZP observability checks. Resolve this
ownership inconsistency before expanding selected-predicate replay. Explicit
low-level/ABI effects remain relevant, and address-taken program objects need
correct relocation rather than automatic numerical-address preservation.

The newly scoped selected gates contain no resolved inputs from the proposed
moving storage; unresolved spill references and inherited facts remain explicit
coverage gaps. No semantic ownership rule, guard relaxation or compaction is
implemented by this slice. Validation passed 452 scoped unit tests, 78 integration
tests, three VM tests, two audit tests and 12 full-output/census comparisons.

## Storage ownership implementation

The implementation uses two slices:

1. Classify compiler homes by typed role and exposure. Publish all declared
   source ranges from final emission, including unused declarations, aliases and
   descriptor backing. Use the shared definition-deadness proof for terminal
   deletion of spills, allocated ZP and locally initialized pointer scratch.
   Retain explicit numeric/assembly/ABI readers, fixed frames, callee contracts,
   preservation replay, final-cost acceptance and rollback. Remove the redundant
   physical-ZP reader walk and its special `SArgs` handling; commit implementation,
   contract documentation and regressions.
2. Qualify both runtimes and shifted origins against the frozen pre-change
   compiler, execute source-ZP/ABI alias controls, and check LF/CRLF parsing and
   MADS reconstruction/relocation. Record output, work and actual savings in a
   separate receipt; commit qualification.

The first fixed-scratch consumer is deliberately narrow: a materializer
reservation and typed pointer use establish the role for `$AA..$AF`, and each
block must initialize the relevant byte before a read. Calls, machine blocks
and barriers end that initialization evidence. Incoming and cross-block values
remain public/ABI. The shared home analysis still conservatively handles fixed
homes around opaque ABI boundaries. This changes terminal cleanup eligibility;
it introduces no global liveness relaxation or frame compaction.

Source declarations take precedence over compiler placement. TN's `$E0/$E1`
and `$E6..$E9` stay program-owned. The callee-pointer and caller-after-return VM
fixtures now explicitly declare the `$E0/$E1` cells they observe, and continue
checking those cells alongside ABI bytes, parameter homes and ordinary numeric
addresses. Generic source-memory
`Unknown`/`All` effects do not expose non-escaping implementation homes; explicit
physical regions and typed machine references do.

The frozen comparison uses the compiler at `203560ed` (compiler code from
`a898ac04`) and the existing
[`word_result_proof.rs`](../tools/compare6502/word_result_proof.rs) harness without
extra flags. The same sources are compiled with the optimized configuration,
pruning disabled, ActionCart/Standalone runtimes, and origins `$3000`/`$41C7`.
Results are identical in size at both origins:

| Program | Cart before → after | Standalone before → after |
| --- | ---: | ---: |
| TN | 9,970 → 9,962 | 10,077 → 10,069 |
| Word-load sharing | 301 → 301 | 301 → 301 |
| Argument staging | 330 → 330 | 412 → 412 |
| Pointer-invariant loop | 77 → 77 | 77 → 77 |
| Word-result control | 50 → 50 | 50 → 50 |

Implementation is committed as `d2acf831`. These are packaged XEX bytes. TN saves
**eight additional bytes** from one dead ZP store each in `MoveMenuBar`, `Draw`,
`Tag` and `TagAll`; producer deletion is unchanged. Its complete terminal
transaction now removes ten stores and four producers, saving 38 bytes against
the pre-cleanup candidate. All 16 independent-control outputs, including full
`CodegenOutput` debug representations, remain identical. Each of the 20 cases
also checks complete output/work equality with the census disabled/enabled.
TN retains 3 materializations, 43 emission invocations, 258 inner probes and
6 preservation replays; rewrite-analysis builds increase from 24,926 to 24,940.
Independent-control work is unchanged.

Reproduce the current corpus with the repository's build profile, then link the
existing harness against that library:

```sh
export CARGO_TARGET_DIR=build/checked-arena/shared-target
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
mkdir -p build/mir6502-storage-ownership
cargo build --lib
rustc --edition=2024 tools/compare6502/word_result_proof.rs \
  --extern actionc="$CARGO_TARGET_DIR/debug/libactionc.rlib" \
  -L "dependency=$CARGO_TARGET_DIR/debug/deps" \
  -o build/mir6502-storage-ownership/probe
build/mir6502-storage-ownership/probe build/mir6502-storage-ownership/reproduced
```

The output directory must be new. Run the same harness against the baseline
revision to compare; the frozen baseline executable for this run was
`build/mir6502-spill-movement/final-probe`. The committed
[qualification receipt](benchmarks/6502-storage-ownership/qualification.json)
records image hashes, full-output equality, cleanup outcomes and compiler work.
Large traces and binaries remain local under
`build/mir6502-storage-ownership/`.

Validation passed 1,336 MIR6502 unit tests, 133 integration tests across all
30 `mir6502_*` targets, and 35 VM tests across 16 affected targets. Fourteen
optional unit checks and eleven optional integration checks were initially
ignored; the relevant MADS unit and integration checks were then run explicitly
and passed. One frozen unchecked-allocator measurement remains ignored. The new
ownership fixture also executes 6,144 original/selected programs across every
byte input, three home kinds, both runtimes and both origins. Source-ZP and
ABI observation tests retain concrete value and machine-state assertions.
Caller fixture instrumentation and parsing both exercise LF and CRLF input.
Documentation links and whitespace checks pass. Shared NIR/frontend contracts
and other backends were unchanged; local validation is scoped accordingly.

## Pruning ownership follow-up — slice 1

Apply the same home-ownership rule to allocation pruning while retaining the
core and final surviving-address equality guards. Reuse `StorageOwnership` for
current and historical address/machine references and source backing. Remove
frame-wide exposure caused by source-object addresses or initializer relocations.
Use emission's complete source-storage inventory instead of a separate partial
list of globals and absolute locals. Generic source-memory effects do not expose
private allocations; explicit physical accesses and ABI homes remain protected.
Unresolved assembly names or uninspectable raw-machine footprints still decline.

Regression coverage must distinguish source-address exposure from actual spill
exposure, retain historical typed assembly and source backing, reject source
aliases and missing inventories, and preserve explicit physical/ABI accesses.
Extend the existing exhaustive byte oracle to indirect source reads and writes;
run its MADS reconstruction/relocation check in both text encodings. Keep the
existing core-movement, deferred-storage movement and preservation rollback tests.
Qualify pruning off/on against TN and independent controls, measuring actual
output and work. Pruning remains opt-in; this slice grants no permission to move
surviving storage.

Completed: TN now has six eligible unused spill bytes instead of four; the two
in `SetWin` are no longer excluded by neighboring source-address exposure. Both
movement guards remain intact, and `CoreAllocation` still rejects the proposal.
TN stays at 9,962 bytes cart / 10,069 standalone. This slice saves no additional
bytes in the measured corpus.

The existing [`late_spill_pruning.rs`](../tools/compare6502/late_spill_pruning.rs)
harness compared the compiler at `514b5ca5` with this implementation over eight
source/NIR-mode cases, both runtimes and origins `$3000`/`$41C7`. All 64 complete
outputs and images (pruning off/on) match the baseline. Compiler work is unchanged
in all 32 cases, and all 32 pruning trace-off/on comparisons pass. The
[qualification receipt](benchmarks/6502-pruning-ownership/qualification.json)
records hashes, work, outcomes and validation commands. Large artifacts remain
under ignored `build/mir6502-pruning-ownership/`.

Validation passed 46 affected unit tests, eight integration tests and five VM
tests, plus the explicit MADS check with eight LF/CRLF reconstruction/relocation
comparisons. The private-tail oracle executes 2,048 original/selected programs
across every byte input, both runtimes and both origins using indirect reads and
writes. The integration and VM controls parse actual LF and CRLF source. Local
checks are scoped to pruning, terminal cleanup and their runtime consumers;
shared frontend/NIR contracts and other backends did not change.

## Move one live private byte — implementation and qualification

Implement a narrow interior-compaction slice under the existing opt-in gate:

1. Admit exactly one downward-moving ordinary private byte whose uses remain
   symbolic native byte loads/stores. Keep every source/ABI object, other live
   home and storage attribute fixed in both planning and final layouts. Reuse
   ownership, reference classification and the complete source inventory.
2. Check old/new ordinary-memory bounds and explicit physical observations in
   current and retained historical programs. Retain the exact declaration-only
   delta, earlier-decision replay, callee contracts, final-cost acceptance and
   complete rollback. Add no full materialization or diagnostic dependency.
3. Qualify the independent producer control (two dead declarations before one
   live spill), negative movement cases, byte execution, source writes and return
   state. Compare TN and independent controls against the previous compiler, both
   runtimes and origins, with pruning off/on and trace off/on. Exercise actual
   LF/CRLF parsing and MADS reconstruction/relocation.

The [storage-substitution contract](MIR6502_LATE_CLEANUP_DESIGN.md#core-allocation-preservation)
records why ordinary symbolic byte references and unchanged separation/bounds
permit this movement without a general core replay framework. Multiple moving
live bytes and source-object movement remain outside the slice.


The source control also exposed a replay dependency on its two retired pointer
capture slots. The movement proof alone was insufficient: pointer-retention
replay correctly rolled back when their physical placements disappeared. Extend
only symbolic capture-value replay for certified retired private identities;
keep physical-address queries unresolved. Qualify origin invalidation, unknown
captures, CFG joins, certificate scope and address-query rejection alongside the
source control. This introduces no ghost allocation or exception to exact
selected-rewrite replay.
