# Pointer setup proofs and candidate attribution

This slice reuses ordinary-storage bounds for pointer-pair selection and for
separating pointer source reads from fixed zero-page scratch. It also separates
local candidate observations from rejected alternatives and shared SSA facts.
The [contract](../../MIR6502_LATE_CLEANUP_DESIGN.md#pointer-setup-and-candidate-attribution)
defines both the proof and the diagnostic coverage.

The important TN finding is an attribution correction: the earlier round-level
census combined applied direct-pointer rematerialization with queries from its
rejected pointer-dereference competitor. Direct rematerialization itself has no
local layout query. Eliminating the competitor's queries is not evidence that
an accepted TN decision has become independent of numeric layout.

## Measurement

All **54 frozen CLI artifacts** and **24 full API outputs** are unchanged.
The original, attributed and final compilers also preserve work and trace-off/on
output equivalence. TN remains **9,970 bytes cart / 10,077 standalone**, with
**zero additional image saving**. It still uses three materializations,
43 emissions, 258 inner emission probes and 24,926 rewrite-analysis builds.

The exact local attribution in TN is:

| Candidate family and outcome | Count before/after | Numeric queries before → after |
| --- | ---: | ---: |
| Direct pointer rematerialization, applied | 77 / 77 | 0 → 0 |
| Pointer dereference, overlap rejected | 44 / 44 | 44 → 0 |
| Pointer dereference, shape without selected plan | 21 / 21 | 21 → 0 |
| Redundant indexed address, shape without selected plan | 24 / 24 | 0 → 0 |

The 65 eliminated candidate queries and their fixed-pair checks become 65
successful bounds evaluations. TN has no locally applied pointer-dereference
or redundant-address candidate in this capture. The independent pointer source
fixture similarly has one applied direct rematerialization and one rejected
dereference competitor. Accepted dereference and plain-address proof paths are
covered by the direct-MIR controls, not by a demonstrated source-level size win.

Shared/unscoped queries within TN discovery rounds remain 13,288 in cart mode
and 13,282 in standalone mode. Outside-driver
numeric queries fall from 3,772 to 3,725; 74 new structural evaluations comprise
47 successes and 27 fallbacks. Those unscoped records do not identify accepted
decisions and are not included in an accepted-dependency reduction claim.
Counts include speculative materializations and measure evaluations rather than
final instructions or bytes. The local candidate and outside-driver counts
repeat in both runtimes/origins.

## Scope

A complete ordinary two-byte source range selects the existing scratch pair
without computing its planning address. Plain-address preparation can also prove
each ordinary source byte separate from a scratch pair wholly inside zero page.
The existing declaration, extent, origin and storage-budget bounds are reused.
There is no new optimization pass or allocation policy.

Fixed addresses, aliases, descriptors, unknown storage and unproven bounds retain
the existing numeric path. `$FE` can begin a fixed ZP pair; `$FF` cannot. A scratch
pair beginning at `$FF` also remains outside the source-separation shortcut,
because the existing numeric contract compares its high byte with `$0100`.
Liveness, observable reads, memory invalidation and callee effects are unchanged.

Candidate keys join family/block/start to an exact exclusive end within one
attempt/routine/generation. Local gates, shared SSA-prefix observations and queries
outside rewrite drivers are reported separately. Missing scopes and empty local
observations do not establish complete independence. Applied plans in speculative
materializations are still provisional until their enclosing attempts select them.

## Qualification

The original baseline is `2b0530ac`. Instrumentation was committed as `31908c9f`
and compared before adding either shortcut. The proof implementation is
`4f2c7980`. The [receipt](qualification.json) compares all three versions and
records exact candidate outcomes, predicate counts, full-output hashes and work.

The source corpus covers TN, word-load sharing, argument staging, an invariant
pointer loop, word-result placement and an independent repeated-pointer-read
fixture, each in both runtimes at `$3000` and `$41C7`. The word-result fixture
disables leaf inlining in every version; other cases use the optimized config.
Direct-MIR tests separately force an applied pointer-dereference rewrite and
accepted redundant plain-address preparation, so positive proof coverage does
not depend on the source driver choosing those strategies.

## Remaining spill-pruning blockers

The exact core-allocation equality guard still rejects deleting TN's four
eligible interior spills in `FormatDirectoryRow`: surviving addresses would move.
The other two candidates in `SetWin` remain excluded by frame exposure.
The pruning flag remains disabled in both default and optimized configurations.
The explicit pruning-on recheck reports six candidates, four eligible spills,
zero removals and `CoreAllocation` in all four runtime/origin combinations.
Full outputs are unchanged; pruning adds one pipeline attempt without extra
emissions, materializations or rewrite-analysis builds.

Replacing a few local numeric checks does not discharge that equality guard.
Movement also needs preservation of shared analysis, earlier chosen strategies,
callee contracts and observable accesses. The later exact physical-allocation
guard would independently require a replacement movement contract; it is not
reached by TN's current rejected trial. Historical late proofs can still refer
to storage whose executable accesses disappeared.

The next step should target the actual proposed `FormatDirectoryRow` movement:
identify affected surviving objects and the selected decisions that rely on their
placement, including inherited facts. Query totals from rejected alternatives
should not drive more proof conversions. Keep both equality guards until that
movement has an explicit, replayable preservation contract and negative controls.

## Reproduction

Compile the [API probe](../../../tools/compare6502/word_result_proof.rs) against
the original and attributed libraries before rebuilding, giving each a new
output directory and passing `--pointer-setup`. Each capture includes complete
`CodegenOutput`, image, trace and work, with trace-off/on equality assertions.

After building the final compiler, run the
[qualification harness](../../../tools/compare6502/pointer_setup_proof.py):

```sh
python3 tools/compare6502/pointer_setup_proof.py \
  --original-api build/mir6502-pointer-setup-proof/baseline \
  --attributed-api build/mir6502-pointer-setup-proof/attributed \
  --compiler build/checked-arena/shared-target/debug/actionc-emit
```

It compares 54 frozen CLI image/listing/map artifacts, 24 full API outputs across
all three versions, candidate outcomes and compiler work. It also runs the
[pruning probe](../../../tools/compare6502/late_spill_pruning.rs) with `--tn-only`
to recheck the four runtime/origin combinations. Raw bundles remain under ignored
`build/mir6502-pointer-setup-proof/`; the receipt records their measurements.

## Validation

Checks passed: **1,329 MIR6502 unit tests**, **135 integration tests across
31 targets**, and **33 VM tests across 15 targets**. Fourteen unit, eleven
integration and one VM optional checks remain ignored; their names and reasons
are retained in the receipt. Focused cases ran first; the remaining integration
targets ran separately without repeating passing suites.

New controls cover exact candidate/range/generation attribution, bounded and
duplicate-key diagnostics, ordinary globals/locals/parameters, fixed-pair
boundaries, aliases, unknown roots, source writes, opaque effects and live A.
The source fixture goes through LF and CRLF parsing in both runtimes/origins.
Its VM test executes 80 combinations and checks both result bytes and exactly two
observable source reads. Existing pointer, argument, late-cleanup, allocation,
TN arena and TN dispatch tests also pass.

CLI hashes use the previously frozen phase-tracing corpus, already qualified
unchanged by the word-result slice. Emission code did not change and every frozen
listing is identical, so optional MADS roundtrips were not repeated. No shared
frontend/NIR contracts changed; this backend qualification does not replace CI.
