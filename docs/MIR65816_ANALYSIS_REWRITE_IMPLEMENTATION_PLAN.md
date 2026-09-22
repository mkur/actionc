# Native 65816 analysis and checked-rewrite implementation plan

Status: slices 0 through 9 complete; see the
[final qualification](MIR65816_ANALYSIS_REWRITE_QUALIFICATION.md). Based on main
`6711670`, this implements the
[foundation plan](MIR65816_ANALYSIS_REWRITE_FOUNDATION_PLAN.md). Commit each
completed slice separately, retaining existing local changes and all current
selection policies.

## Deliverable and acceptance contract

Build a typed selected-code view, byte-home liveness, stored-definition facts,
register/flag liveness, and an atomic checked-rewrite driver. Finish by moving
one existing adjacent A16 load omission onto that driver. This is an
unchanged-output migration: stores, homes, allocation/interference, copy
schedules, guards, modes, ABI v1, image v3, o65 profile v1 and Exec816's pin stay
as they are. No new store elimination, register residency, DP allocation,
cross-block forwarding or flag-dead exception is enabled.

The [frozen baseline](benchmarks/65816-analysis-rewrite/baseline.json) references
the qualified INX compiler `1ce9624` and results at `a73dab7`. Planning checked
all 440 source/fixture hashes, 224 artifact files across 56 comparison builds,
and 649 native artifacts in each host profile. These checks authenticate saved
evidence; they are not a new compiler test run. Preserve all 28 Action images
and all 264 comparison records, including control-flow records and the known
optimized vbcc unlink vector-0 failure.

Representative optimized results must remain rotation 126 bytes / 735 cycles /
eight stack bytes and sum-loop(13) 120 bytes / 1,092 cycles / six stack bytes.
The full-record comparison, including forwarding and copy counters, is the
gate; these examples are not substitutes for it.

## Code ownership and proposed interfaces

Keep implementation private beneath `src/mir65816/emit/`. Reuse the existing
[dataflow solver](../src/analysis/dataflow.rs) and
[graph interface](../src/analysis/graph.rs) without changing their semantics.
Leave NIR, semantic lowering, MIR6502 and the native allocator unchanged.

| Proposed module | Responsibility |
| --- | --- |
| `selected.rs` | Typed instructions, ordered compiler events, selected routine and source attribution. |
| `effects.rs` | Exhaustive instruction effects and conservative call/alias summaries. |
| `analysis/sites.rs`, `analysis/cfg.rs` | Snapshot ownership/generation, stable sites and selected-code CFG. |
| `analysis/homes.rs` | Canonical physical byte identities, ownership and overlap classification. |
| `analysis/home_liveness.rs` | Backward may-liveness of home bytes. |
| `analysis/home_definitions.rs` | Forward reaching definitions, use attribution and possibly undefined private reads. |
| `analysis/machine_liveness.rs` | Backward register-lane and independent flag liveness. |
| `analysis/mod.rs` | Immutable analysis snapshot and checked query API. |
| `replay.rs` | Re-execute recorded typed actions through the tracked boundary. |
| `rewrite/context.rs`, `rewrite/plan.rs`, `rewrite/driver.rs` | Proof queries, checked rule plans, validation, rollback and invalidation. |
| `rewrite/rules.rs` | Initially only identity/test transactions and existing adjacent-temp A16 forwarding. |

Names describe responsibilities; avoid introducing a general shared machine IR.
The existing `emit/liveness.rs` continues to own temporary interference.
`state.rs` continues to own forward values and stack/mode facts. `code.rs`
remains the byte/fixup sink; `layout.rs` still finalizes conditional layout.

Use the following contract vocabulary:

- `SelectedRoutine`: immutable owner token, routine identity, allocation
  identity, generation, typed actions and selected CFG. Keep it private; expose
  only immutable observations under `native65816-state-proof` for tests.
- `SelectedSite`: entry, instruction, compiler event or exit, identified within
  that snapshot. PCs are a derived encoding map, never a proof identity.
- `HomeByte`: invocation-entry-relative stack byte or current-domain DP byte,
  with ownership metadata kept separately. Retain logical TempId/frame-object
  attribution without treating different names for the same byte as disjoint.
- `InstructionEffects`: reads, definite writes, possible writes, ordered memory
  accesses, register-lane/flag effects, environment requirements and control flow.
- `AnalysisSnapshot`: immutable input plus CFG, normalized homes and analysis
  results. Every query checks snapshot ownership and generation.
- `RewritePlan`: sealed rule kind, window, original-content identity,
  replacement actions, removed definitions and declared effect changes.
  Matchers cannot attach an arbitrary proof or caller-authored effect summary.

## Slice 0 — Preserve the baseline and prepare the gate

Completed in `51a5499`: [checker](../tools/compare65816/check_analysis_rewrite.py),
[negative controls](../tools/compare65816/test_analysis_rewrite.py) and
[baseline self-comparison](benchmarks/65816-analysis-rewrite/slice0-equality.json).
All 82 comparison-tool tests pass. The gate authenticates the actual input
against the frozen files, compares every observer field outside build provenance,
and requires exactly the known external failure.

The baseline record is delivered with this plan. Before compiler edits, verify
it against the retained `target/loop-inx-after` directory. Missing artifacts
must be reconstructed in an isolated checkout of the qualified compiler, never
rebuilt with the changing implementation or substituted into old reports.

Add `tools/compare65816/check_analysis_rewrite.py` as a small wrapper around
the equality primitives in
[check_state_tracker.py](../tools/compare65816/check_state_tracker.py). Also
compare the complete control-flow records and explicitly require the single
known external failure. New analysis telemetry goes in separate files; do not
add fields to old executable artifacts or normalize away differences.

Add negative controls for changed bytes, fixups/maps, frame/guard ranges,
control targets/predicates, traffic/cycles and existing forwarding counters.
The checker must reject altered baseline hashes and unexpected failures.

**Exit:** the checker accepts baseline versus itself and rejects each deliberate
mutation. Commit the checker and its focused tests before compiler changes.

## Slice 1 — Centralize typed instructions and effects

Completed in `0bc371af`: [typed forms](../src/mir65816/emit/selected.rs),
[physical effects](../src/mir65816/emit/effects.rs), verified native call/return
annotations, independent effect probes and
[exact corpus equality](benchmarks/65816-analysis-rewrite/slice1-equality.json).
See [results and qualification](MIR65816_ANALYSIS_EFFECTS.md). The optional effect
observer is separate from historical snapshots; selected-action recording and
CFG construction remain slice 2. No selection or allocation policy changed.

Move the admitted instruction enums from
[tracked.rs](../src/mir65816/emit/tracked.rs) into `selected.rs`, keeping the
existing facade methods available. Introduce one exhaustive dispatch used by
encoding, tracker updates and effect recording. Share instruction semantics,
not the tracker's current precision: for example, a conservatively unknown DP
value still has a concrete DP read/write effect.

Resolve width from verified machine state. `ByteOp` currently describes the
encoded operand size; a stack/DP operation using it may access a full word.
Immediate encodings, M width and index width must remain distinct.

Describe carry reads for ADC/SBC/ROL/ROR, memory reads before RMW writes, branch
flag reads, XBA lane exchange, TSC/TCS full-word behavior, and mode-dependent
loads/transfers/INX. REP/SEP have exact flag/mode effects; X8 narrowing also
defines zero high bytes of X/Y. Do not infer read sets from which forward facts
the tracker happens to invalidate.

Annotate calls at `Builder::call` using its verified call plan: outgoing argument
reads, declared result lanes, scratch clobbers and conservative memory effects.
Model inline arithmetic/helper sequences from their actual selected operations.
Unknown external behavior stays a proof barrier. Physical call/return stack
accesses and the indirect PHK/PER/PHA/RTL protocol remain protected.

**Tests:** exhaustive form coverage; independently specified effects for every
admitted family and both legal widths; no caller-supplied effects. Extend the
existing independent encoding/VM probes for changed effect boundaries.
**Exit:** current library/emission tests and exact corpus equality pass.

## Slice 2 — Record selected actions and build their CFG

Completed: production action/request recording, immutable selected routines,
[scoped sites](../src/mir65816/emit/analysis/sites.rs),
[selected CFG](../src/mir65816/emit/analysis/cfg.rs), and reconciliation before
and after layout. See [results](MIR65816_SELECTED_ACTIONS.md) and
[exact equality](benchmarks/65816-analysis-rewrite/slice2-equality.json).
All selection decisions still come from the existing emitter; this slice
provides no liveness queries, replay driver or additional omissions.

Record instruction actions and zero-byte compiler events while the existing
emitter remains authoritative. Use the same semantic dispatcher; the sidecar
must neither change decisions nor duplicate an instruction from a compound
wrapper. Keep recording available to production analyses, independently of
optional test traces.

Include label allocation/binding, mode requests even when REP is omitted,
body-anchor establishment, home registration, barriers, capture/consume
requests, MIR-entry obligations, X reservation/refresh requests and source-span
boundaries. Record the inputs to a proof request; never treat a recorded
successful result as permission during replay.

Build the graph from the selected sequence. Include internal compare blocks,
guard/fault exits, staging paths, empty transfers and indirect-call continuations.
Represent the inverse-branch/JML dispatch as a typed compound with two exact
successors; it remains indivisible for initial rewrites. Calls are intraprocedural
summary nodes with return continuations, not recursive expansions of callees.
An RTL used for indirect transfer must not be mistaken for routine return.

Check every label and edge, instruction boundary, mode requirement and stack
equation. Join differing facts conservatively; unsupported or inconsistent
boundaries block proofs. No source-string or disassembled-text reconstruction.

Add `analysis/sites.rs` with an owner token and allocation/selection generation;
the same RoutineId in a different compilation must not validate an old site.
Implement `DataflowGraph` in `analysis/cfg.rs`. Keep source MIR spans and selected
sites distinct when a fused comparison owns several machine blocks.

**Tests:** diamonds, loops, duplicate edge obligations, internal labels,
fallthrough, fault exits, far and indirect calls; wrong owner/site/generation;
complete code/fixup/trace reconciliation before and after branch relaxation.
**Exit:** every corpus instruction has typed attribution, all snapshots remain
equal, and there are no unclassified production paths.

## Slice 3 — Canonical home bytes and backward liveness

Completed: [physical homes and read-only liveness](MIR65816_HOME_ANALYSIS.md),
verified ownership provenance and ordered alias-aware transfers through the shared
solver. [Exact output equality](benchmarks/65816-analysis-rewrite/slice3-equality.json)
passes. No new omissions or allocation changes are enabled.

Implement `homes.rs` from the verified allocation, ABI and entry-S equations.
Use signed offsets relative to invocation entry, including outgoing/transfer
storage. The same `d,S` with different S values may name different bytes; different
`d,S` values may name the same byte. Checked ranges must account for native
bank-zero addressing rules; uncertain/wrapping ranges stay protected or unknown.

Keep DP relative to the active execution domain. Account for aliases between
temp homes, staging, fixed scratch and frame objects at byte granularity.
Private ownership needs verified provenance; an arbitrary numeric address is
not proof of non-aliasing or non-observability.

Adapt MIR6502's home-liveness transfer using the shared solver:
`live_before = reads union (live_after minus definite_writes)`.
Unknown reads expand conservatively to potentially aliased homes. May-writes
cannot kill liveness. Include implicit pointer-byte reads and read-before-write
effects of compound instructions and calls. Boundary observations come from
the native ABI and protected environment, not the 6502 return-home constants.
Compose ordered sub-effects when a compound operation writes a byte before a
later internal read; do not turn that internal read into a spurious entry use.

**Tests:** read on one branch, definite overwrite on every branch, loop-carried
reads, backedge to a read before the candidate store, partial word overwrite,
reused homes, DP/stack separation, S movement, pointer aliases, volatile memory
and arguments read before a call clobber. Check exact expected live sets with
hand-authored typed graphs, independently of the production classifier.
**Exit:** read-only `home_live_before/after` queries; no new omissions.

## Slice 4 — Reaching stored definitions and read attribution

Completed: [stored definitions and read attribution](MIR65816_HOME_DEFINITIONS.md),
possibly undefined private reads, checked store/window queries and conservative
alias blockers. [Exact output equality](benchmarks/65816-analysis-rewrite/slice4-equality.json)
passes. These are read-only facts; no stores or homes are removed.

Adapt [MIR6502 home definitions](../src/mir6502/analysis/home_definitions.rs).
Identify each definition by `(HomeByte, write site)`. Merge possible reaching
definitions at joins; a definite write replaces definitions only for bytes it
writes. Preserve prior candidates under may-writes and represent uncertainty;
never let an unknown write prove a private byte initialized.

Track possibly undefined private bytes separately, retaining that possibility
when only one incoming path initializes a home. ABI-defined inputs are seeded
from their verified entry contracts. Collect uses before applying the writes of
the same operation. Attribute aliasing reads through canonical physical bytes.

Provide `uses_of_definition`, `definition_dead_outside_window` and
`undefined_private_reads`. Invalid/unreachable sites and incomplete effects
produce blockers, not vacuous success. Window-local reads must still be checked
against the replacement before a rewrite can remove their definitions.

**Tests:** two stores to one home where only one is dead; a store read between
later overwrites; conditional initialization; partial aliases; same-site loop
iterations; unknown reads/writes; introduced replacement reads. Keep existing
memory definedness policy conservative rather than changing accepted programs.
**Exit:** exact store queries distinguish cases whole-home liveness cannot.

## Slice 5 — Register-lane and independent flag liveness

Completed: [checked physical machine liveness](MIR65816_MACHINE_LIVENESS.md),
independent graph tests and VM perturbations. Environment effects and compiler
witnesses remain protected. No selection or emission policy changed.

Adapt [MIR6502 machine liveness](../src/mir6502/analysis/machine_liveness.rs)
using the selected CFG and central effects. Track A-low/A-high, X-low/X-high,
Y-low/Y-high and separate N/Z/C/V bits. Use union at joins and the same backward
read/definite-write transfer; do not assume facts from code-generation order.

A8 loads preserve the hidden A-high byte; X8 writes/narrowing have different
rules. INX uses index width. Flag dependencies inside helpers, compare dispatch
and multi-byte carry chains must be visible. Seed native result boundaries from
`abi::ResultLocation`, including zero-extension requirements of byte and
three-byte results, plus declared environment restoration.

Represent S, direct-page register, DBR, PBR-dependent transfers, E/M/X, decimal
and I requirements as protected environment effects in the first driver. Model
their reads/writes for verification, but do not use deadness to delete them.
Keep X reservations and compiler witnesses as additional proof obligations.

**Tests:** each flag independently live/dead, live through a diamond/loop,
carry-in arithmetic, CMP-to-branch, INX-to-TXA, A8-to-A16/XBA, index narrowing,
TSC/TCS, call input versus call clobber, and every native result class.
Add VM perturbation probes at representative declared-dead and declared-live
sites, observing defined outputs and selected memory events rather than
claiming equivalence from analysis output alone.
**Exit:** checked register/flag queries alongside the unchanged forward tracker.

## Slice 6 — Typed replay with exact encoding equality

Completed: [fresh typed replay](MIR65816_TYPED_REPLAY.md) is authoritative after
direct/replay shadow qualification. The full native debug/release and isolated
CRLF suites pass; all [corpus artifacts and records remain equal](benchmarks/65816-analysis-rewrite/slice6-equality.json).
Replay checks freshly recomputed request decisions and preserves symbolic sites;
it does not supply a checked rewrite transaction API yet.

Implement replay into a fresh `TrackedEmitter65816`, seeded only with verified
entry/allocation facts. Execute recorded actions through the same dispatcher;
do not copy a stored `State65816` snapshot or replay the old proof answers.
Recompute mode permission, capture generations, consumed witnesses and X
refresh checks. Record label/span endpoints symbolically and derive offsets
from replay so byte cursors cannot become stale proof identities.

Separate replay from recording to avoid recursive capture. First compare replay
with direct emission in tests; then make replayed output authoritative after
the equality gate passes. Before enabling this, recover all zero-byte compiler
events needed by existing selection. If a command cannot be replayed faithfully,
finish its explicit representation rather than adding raw encoder access.

Rebuild Code bytes, symbolic/return fixups, MIR spans/transfers, branch records,
boundaries and proof traces. Run the unchanged `layout::finalize` once afterward;
keep both flat-image and o65 paths on the same finalized result. No serialized
format change is needed for analysis records.

**Tests:** trace on/off, repeated deterministic replay, short/long dispatch,
PER continuations, zero-byte barriers/mode requests, captured home generations,
X pending/refresh intervals and frame/guard accounting. Full native execution
qualification is required when replay becomes authoritative.
**Exit:** every old artifact and measurement remains equal; replay is the only
route by which later accepted typed edits become machine bytes.

## Slice 7 — Checked plans and atomic application

Completed in `883b5f51`: [checked contexts, sealed plans and atomic transactions](MIR65816_CHECKED_REWRITES.md)
are qualified on identity/test rules. No production optimization has migrated yet.

Add `Proof<T> = Proven(T) | Blocked(reason, site)` and an immutable context.
Expose checked home/definition/register/flag queries plus a narrow local
equivalence witness from the existing tracker. Do not implement generic value
availability across joins in this foundation.

Initially accept one contiguous window inside one selected block, with no
labels, calls, guard operations, stack/mode changes, reservation changes or
protected compiler events removed. The identity/test rule and the pilot rule
are a closed set. A live-state change requires rule-specific equivalence;
merely declaring a delta or finding a dead destination cannot prove a rewrite.

Apply the following transaction:

1. Validate owner/allocation/generation, sites and exact original actions.
2. Recompute removed definitions and replacement reads/effects. Reject omitted
   declarations, observability/order changes and live unproven deltas.
3. Build a scratch selected routine, apply one replacement and increment its
   generation. Rebuild all affected facts initially; optimize invalidation later.
4. Reverify CFG, modes, stack equations, ownership and compiler-event obligations;
   replay and finalize into scratch output.
5. Publish the new routine, analyses and output only if every check succeeds.
   A failed plan leaves all original objects unchanged.

Use checked errors for malformed plans; do not use panic-catching as rollback.
Preserve errors for invalid source MIR. Record deterministic blockers and applied
rule counts separately from existing corpus measurements. Revalidate after each
successful plan; even non-overlapping windows can have interacting liveness.
Require a decreasing rule metric and a bounded iteration count. Test no-ops use
one-shot application and are not iterated to a fixed point.

**Tests:** wrong compilation/routine/allocation, stale generation, invalid range,
live store removal, replacement reading a removed definition, partial overlap,
undeclared A/C/N/Z changes, unsupported effects, stale capture/X proof, two
interacting plans, non-decreasing rule, replay failure and complete rollback.
**Exit:** no new production optimization; the driver is proven on synthetic
transactions and identity replays.

## Slice 8 — Migrate adjacent temporary A16 forwarding

Completed in `e4fd88b5`: [typed candidates and authoritative checked forwarding](MIR65816_ADJACENT_CHECKED_FORWARDING.md).
Shadow comparison was committed separately in `4130d858`; the authoritative
qualification preserves all 102 decisions (75 accepted, 27 blocked) and the
[full frozen corpus](benchmarks/65816-analysis-rewrite/slice8-equality.json).
No eligibility or ABI change is enabled. Each edit still rebuilds all analyses;
immutable sharing within one solver run avoids copying unchanged facts.

Choose only the temporary-home case in
[`Builder::load_checked_word`](../src/mir65816/emit/accumulator.rs), whose current
first choice is `consume_word`. Preserve its priority over `load_x_word`, all
operand/home preflight, retained stores, exact A/N/Z requirement and single-use
witness consumption. Frame and incoming-parameter forwarding remain unchanged.
Preserve the current failed-request behavior too: `consume_adjacent` takes the
witness before checking it, so a rejected request cannot leave it reusable.

Split testing the adjacent witness from consuming it. At the load request,
capture a typed candidate containing the actual LDA that would otherwise be
emitted, its original home, and the retained producer/capture attribution.
The candidate must exist before omission; analyzing the already-omitted stream
cannot prove that the read was removable.

First run the new rule in shadow mode on that candidate and compare its decision
with the old predicate for every request, including rejected ones. Construct
and replay the original/replacement windows from the same verified entry facts;
prove private read removability and full A/N/Z equality. Do not generalize to
dead flags or non-adjacent facts. Compiler-event dependencies must stay valid.

For authoritative integration, retain candidates in the planned typed routine
before encoding and let the checked driver select their replacements. The
planning traversal may use the proved-equivalent post-state to continue existing
selection, but it cannot publish bytes before the driver validates the final
plan. If a candidate is rejected, retain its explicit load and revalidate the
continuation; never replay a previously recorded success token. Rebuild derived
positions and traces after all accepted edits. This adaptation is a separate
commit from the shadow comparison.

Remove the legacy authoritative decision only once all existing positive and
negative cases agree. A proof failure safely retains the ordinary load; an
unexpected mismatch on the qualified corpus blocks this migration's completion.
The private test-only reference path can be removed after qualification.

**Tests:** current accumulator tests plus multiple uses, byte/word and alias
boundaries, barriers/labels/calls, partial writes, stale sites and malformed
plans. Compare raw/optimized decisions, bytes, trace behavior and unchanged
forwarding counts. IRQ/NMI, relocation and frame guards must still pass.
**Exit:** one production consumer uses the foundation; eligibility is unchanged.

## Slice 9 — Final qualification and documentation

Completed in the final qualification/report commit following `e4fd88b5`:
[results and host costs](MIR65816_ANALYSIS_REWRITE_QUALIFICATION.md),
[qualification record](abi/action65816-analysis-rewrite-qualification.json) and
[host measurements](benchmarks/65816-analysis-rewrite/host-compilation.json).
All 658 native artifacts agree across debug, release and CRLF; all 224 corpus
files and 264 records preserve the frozen baseline. The host corpus takes
2.46 times the baseline wall time, with median per-process peak RSS rising
from 5.70 to 6.92 MiB. This completes the foundation, not its performance tuning.

Save new analysis observations separately in
`docs/benchmarks/65816-analysis-rewrite/`, with compiler/tool/source hashes and
explicit counts of classified, blocked and accepted cases. Add a final
`docs/abi/action65816-analysis-rewrite-qualification.json` recording native
debug/release, corpus, CRLF and mutation-control evidence. Do not overwrite the
INX reports or claim compilation-time overhead is zero: measure host compile
time and peak memory on the same corpus before/after, recording methodology.

Update the emission contract, state-tracker design and quality plan to describe
the actual completed interfaces and sole migrated consumer. The foundation
note remains the contract; mark this plan's slices completed only with their
commit and qualification references. Subsequent optimization plans must use
the resulting proof API and declare their own measured deltas.

## Validation schedule

Run focused checks at each slice; do not repeat every native suite after changes
that only add read-only analysis. Use the following scope:

| Change boundary | Required local checks |
| --- | --- |
| Equality tooling | Focused Python tests and all deliberate negative controls. |
| Effects/recording | Native emitter library tests; emission/state-boundary/ABI integration; independent instruction probes; full raw/optimized corpus equality. |
| Home/definition/machine analyses | New analysis unit tests, hand-authored CFG expectations and independent probes; affected emitter checks; equality when production recording changes. |
| Replay/authoritative rewrite path | Affected root integration plus full native debug/release qualification, corpus equality and isolated CRLF rebuild. |
| Final documentation | Saved-evidence hashes, links and diff checks; rerun code checks only for intervening compiler/test changes. |

Core existing commands (new analysis tests join the `mir65816` library filter):

```sh
cargo test --features native65816-state-proof --lib mir65816
cargo test --test mir65816_state_boundary --test mir65816_abi \
  --test mir65816_contract --test mir65816_emission --test mir65816_o65 \
  --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```

For corpus gates, build a fresh release `actionc-65816`, use
`tools/compare65816/build.py --verify-crlf` with a new output directory, then run
the ignored `code_quality` target in both host profiles with the manifest/result
environment variables documented in [the baseline results](MIR65816_LOOP_INX.md).
Those executions retain the known vbcc failure; require exact full-record
equality and no additional failures. Run the new strict checker with the frozen
baseline. Compare all metadata and serialized flat/o65 outputs in their affected
native tests, including both incoming I states.

Rebuild newline-sensitive native fixtures and the emission-boundary fixture in
an isolated CRLF checkout when replay/pilot integration is qualified. Snapshot
changes are not expected. An unexplained difference is a failed gate, not a
reason to refresh the golden output.

This plan does not require NIR changes. If implementation crosses that boundary,
split the change and run contributor-required NIR snapshots, sweep and full
compiler tests. No public analysis ABI, generic cross-target rewrite framework,
new optimizer switch, or new performance optimization is part of completion.
