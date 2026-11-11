# MIR6502 callee entry flag summaries plan

Prepared 2026-11-11. Status: slices 1–3 complete; the new control remains off. Compiler baseline: `82ada38f`.
Implement in four vertical slices and commit each after its checks pass.
This refines the existing MIR6502 flag proof contract; a separate design note
or new executable IR form is unnecessary.

Record which incoming C, N, Z and V values a scalar callee may observe, then
let argument A to X forwarding request only N/Z independence. `TAX` and the
removed `LDX` change N/Z while preserving C/V. A callee that first defines
N/Z and then passes live C/V to a nested call can therefore qualify without
proving that C/V are unused.

The [qualified forwarding baseline](benchmarks/6502-argument-x-forwarding/README.md)
has TN/TNDBG cartridge images of 9,858/12,954 bytes. `Strcat` is 113 bytes
and its sampled call takes 355 cycles. These are controls for the new work;
count only additional savings from the more precise flag proof.

## Current gap and candidate evidence

[known_callees.rs](../src/mir6502/analysis/known_callees.rs) currently stores a
set of scalar routines whose entry ignores all four tracked flags. It uses
CFG liveness, treating nested calls/helpers/barriers as observations of every
flag. [posthome.rs](../src/mir6502/analysis/posthome.rs) and the rewrite context
copy that Boolean fact. Argument forwarding asks for it when an unchanged
tail does not restore N/Z before the call.

The current regression
`argument_x_forwarding_rejects_scalar_wrappers_with_transitive_flag_inputs`
distinguishes a wrapper that immediately calls another routine from one that
first loads an immediate into A. The load defines N/Z, but the all-flags
predicate rejects both wrappers because C/V can still reach the nested call.
Retain the first rejection. Under the refined query, the second wrapper must
prove N/Z unused while queries including C or V still reject.

The existing typed census records these call-flag blockers, separately in
each runtime:

| Program | Caller routines | Flag-blocked sites |
| --- | --- | ---: |
| TN | `PopUp`, `Inv`, `Copy` | 3 |
| TNDBG | `PopUp`, `Path`, `Inv`, `DrawWinFrame`, `Dbg`, two in `Copy` | 7 |

These counts combine several causes: eligibility, unavailable facts, explicit
contracts and live inputs. Slice 1 must classify each by typed target and
requested mask. Some targets contain machine blocks or lack scalar facts and
will remain unsupported. The counts do not establish recoverable bytes.
The unsupported setup operations in `Sort`/`Tag` remain separate work.

## Entry summary contract

Add a typed incoming-flag fact alongside existing callee exit summaries,
provisionally `MirKnownCalleeEntryFlags`:

```text
Unknown
Known { may_observe: MirFlagSet }

entry_flags_unobserved(target: RoutineId, changed: MirFlagSet)
```

For a known summary, a request succeeds only when `changed` and `may_observe`
are disjoint. Unknown facts cannot prove a nonempty request. Keep missing
facts distinct from a known empty mask; default construction must fail closed.
Use stable routine IDs for lookup and existing `MirFlagSet` bits for C/N/Z/V.
Names belong only in diagnostics and measurement output.

`may_observe` includes each incoming flag that can be read before a definite
definition on any reachable CFG path. Union requirements at joins and solve
loops using the shared dataflow machinery. A definition must protect every
path reaching an observation; a definition in one arm is insufficient.

Retain the initial eligibility boundary: scalar signature, non-opaque routine
effects, no declared flag input/preservation contract and no machine blocks
in the routine. Preserve the forwarding call site's explicit checks for
opaque effects, flag reads and ABI/effect flag preservation. The new fact
does not override those declarations. Invalid CFGs, missing entries and
unsupported facts return unknown.

Every nested call, runtime helper and barrier remains a conservative input
observation of all flags, regardless of an incomplete callee declaration.
A definite N/Z definition before that boundary can discharge N/Z inputs;
definitions of C/V are separate. Do not propagate new entry summaries through
calls or add a call-graph fixed point in this task. Recursive calls receive
the same conservative boundary treatment.

## Definite definitions and shared analyses

Reuse [machine_liveness.rs](../src/mir6502/analysis/machine_liveness.rs),
[effects.rs](../src/mir6502/analysis/effects.rs) and the shared CFG solver.
Provide an explicit entry-proof query or mode within shared liveness so the
summary does not grow a second CFG solver or an opcode-name scanner.

Audit the entry transfer before publishing individual bits. `flag_writes`
and certified definite definitions can kill incoming values; a may-clobber
alone cannot. The current general transfer combines `flag_writes` and
`flag_clobbers`, so the entry-proof mode must distinguish them. In particular,
calls, helpers and compound operations must not appear to define every flag
merely because their compatibility effects conservatively clobber flags.
Read-before-write operations such as ADC/ROR must keep their incoming carry
requirement even when they also define carry.

Keep the existing transfer behavior for unrelated liveness consumers. For an
unsupported entry transfer, retain incoming dependencies or return unknown.
Add precise effect facts only for a demonstrated required operation, with
focused emitted-code coverage. Do not broaden machine-block admission or
claim that a compiler-generated prologue defines flags without structured,
complete facts. N/Z bits may be independently known even when C/V remain
potential inputs.

Compute a summary once per target in an immutable program analysis snapshot.
The refined selector may restrict this work to targets of its typed candidate
calls. Avoid adding repeated full-program entry analyses to every existing
callee-summary consumer. Rebuild facts when a contributing callee body changes;
do not cache them by routine name or reuse them across program snapshots.

## Forwarding integration and controls

Keep the earlier all-flags compatibility predicate and its meaning intact.
Existing selectors, including restricted argument staging, retain their
current admission policy. The new mask query must not replace a strong
all-flags answer with an N/Z-only Boolean under the same name.

Introduce `enable_precise_callee_entry_flags`, initially false in both config
constructors. Its first consumer is the late generalized A to X selector,
under the existing peephole and argument-X switches. When disabled, reproduce
`82ada38f` selection and images exactly. When enabled, retain the preceding
qualified cases and admit additional cases only through the new N/Z query.
Share the existing discovery, transaction and costing batch; do not create
another whole-program materialization or per-site emission trial.

Expose the mask query through the post-home snapshot and rewrite context.
The forwarding witness must identify the target, requested N/Z mask and
proof policy used. Driver revalidation must reconstruct that fact from its
supplied analysis snapshot as well as checking the routine generation, byte
identity, physical map and exact replacement. A caller's generation alone
does not establish that a callee summary is current.

Retain both existing flag obligations: inserted TAX must not destroy live
N/Z after the store, and deleting LDX must not remove an observed definition.
Keep the tail-restoration path intact. The mask query discharges only the
callee-entry part of the second obligation; it does not replace caller
liveness or prove a published exit contract.

Final replay must validate consumed entry facts against the proposed callees,
either by rebuilding supplied summaries or proving that the consumed unused
bits remain unused. Preserve existing published exit facts too. If an accepted
callee changes a consumed input fact, reject or restore the dependent batch.
Test a caller and callee changing together; baseline callee facts alone are
insufficient for final acceptance.

Keep source evaluation, all stores/homes, X lifetime, physical aliases,
64-operation/tail limits, call shapes and final cost gates unchanged. SemIR,
NIR, executable MIR forms, ABI placement and emission remain unchanged.

| Code home | Planned change |
| --- | --- |
| `analysis/known_callees.rs`, `analysis/machine_liveness.rs`, `analysis/effects.rs` | Entry masks, conservative boundary reads and certified flag definitions |
| `analysis/posthome.rs`, `rewrite/context.rs` | Snapshot-scoped mask queries alongside the coarse compatibility path |
| `analysis/argument_x_forwarding.rs`, `rewrite/plan.rs`, `rewrite/driver.rs` | Requested-mask provenance and reconstruction of the typed witness |
| `materialize/argument_staging/x_forwarding.rs`, `passes.rs` | Independent control, existing costed batch and final input/exit validation |
| Existing argument-X tests and new `callee_entry_flags` targets | Mask, wrapper, snapshot, emitted-state and runtime regressions |

Code paths in this table are relative to `src/mir6502/` except test target names.

## Slice 1 Freeze controls and add inert per flag facts

1. Freeze compiler/dependency/source hashes, images, listings, MIR, frames,
   storage and zero page at `82ada38f`. Include TN/TNDBG in both runtimes and
   neutral controls at `$3000`/`$41C7`. Reuse matching archived artifacts only
   after validating their hashes and facts against this baseline.
2. Classify the ten recorded TN/TNDBG flag blockers by typed target, requested
   N/Z, eligibility and incoming masks. Record separate reasons for unknown
   facts, live N, live Z and explicit contracts. Do not optimize printed IR.
3. Add the inert per-flag representation and entry-proof liveness mode, with
   shared definite-definition tests. Retain the current coarse consumer path.
4. Add a focused neutral fixture, provisionally
   `fixtures/runtime/callee_entry_flags.act`, plus integration and VM targets
   `mir6502_callee_entry_flags` and `callee_entry_flags`. Confirm a genuine
   optimized-NIR miss with C/V observable after an N/Z definition. Inspect
   normal inlining/prologue output; supplement with typed MIR fixtures where
   source syntax cannot express an individual flag input.

Acceptance: inert facts distinguish N/Z from C/V, unknown remains explicit,
all baseline images remain equal and at least one neutral miss survives the
normal preceding optimizations.

Suggested commit: `Model per flag callee entry requirements`.

## Slice 2 Consume precise N and Z facts in argument forwarding

1. Add the default-off control and context query; keep the earlier coarse
   predicate available for baseline behavior and historical consumers.
2. Request exactly N/Z for the additional forwarding admissions. Carry proof
   provenance through the existing witness and driver; reject mismatched
   masks, wrong targets, unknown facts and stale callee snapshots.
3. Reuse the existing costed batch, routine rollback and final physical replay.
   Validate consumed input facts against the combined proposal alongside
   existing output contracts, frames and exact final XEX costs.
4. Extend stable reporting to distinguish coarse admissions, additional N/Z
   admissions, unknown inputs and each live requested flag. Count an application
   only after final acceptance. Report the actual queried masks/target IDs in
   opt-in diagnostics, without changing executable printed MIR contracts.

Acceptance: a neutral case removes its reload with retained stores and equal
C/V-dependent behavior; an immediate flag-forwarding wrapper still rejects.
Off reproduces the frozen compiler, reporting leaves bytes equal and both
configuration defaults remain off for the new control.

Suggested commit: `Use precise callee entry flags for argument X forwarding`.

## Slice 3 Qualify flag lifetimes and analysis invalidation

| Area | Required cases |
| --- | --- |
| Masks | All 16 requested masks and all C/N/Z/V input combinations; N-only, Z-only, N/Z and C/V observations; missing versus known-empty facts |
| Definitions | Loads defining N/Z; compares and arithmetic; carry-dependent ADC/SBC/ROR; declared may-clobbers that are not definite kills |
| CFG | Both join arms defining N/Z, one arm missing a definition, entry reads, backedges and unreachable reads; invalid/missing entry fallback |
| Boundaries | Immediate nested call rejects; N/Z definition before that call permits only N/Z requests; opaque calls, helpers, barriers, recursive calls and machine-block eligibility remain conservative |
| Contracts | Explicit effect/ABI flag reads or preservation still reject; restored-tail path continues to supply identical flags |
| Generations | Wrong target or requested mask, stale caller/callee body, altered replacement and simultaneous caller/callee selection fail final validation |
| Existing invariants | Equal arguments, A/X/Y/C/V/SP, stores, parameter homes, read order, memory guards, physical aliases and routine exit guarantees |

Extend the existing wrapper regression rather than deleting its negative
case. Assert that its N/Z-defining sibling has C/V in `may_observe`, N/Z
outside it, and still fails an all-flags request. Exercise the requested-mask
truth table with an independent host oracle, not the production mask helper.

Use emitted execution for the new positive/negative schedules. Sweep all 256
forwarded bytes and the 16 input flag combinations, selecting cases where
old/new N/Z actually differ at scalar entry. Observe carry/overflow-dependent
results at the nested boundary and compare exact flags only after the
retained definition restores them. Verify the untouched immediate-wrapper
fallback against an observer that consumes incoming N/Z.

Run raw/optimized NIR, cartridge/standalone, both origins, repeated inputs and
actual LF/CRLF parsing. Batch boundaries rather than multiplying every alias
case by every flag input. Reuse existing forwarding alias/read/write oracles
and cover all eight preceding argument/pointer feature combinations on the
new neutral schedule.

Acceptance: permitted N/Z differences have no observable effect, C/V inputs
remain meaningful, unsafe wrappers retain the reload and final validation
cannot use stale entry facts.

Suggested commit: `Qualify per flag entry proofs across calls and control flow`.

## Slice 4 Measure applicability and qualify the optimized default

Add an opt-in exporter `callee_entry_flags_measure_corpus` using the existing
normal linked compilation path. Write artifacts to ignored
`build/callee-entry-flags/` and compact manifests/results to
`docs/benchmarks/6502-callee-entry-flags/`. Preserve earlier measurement outputs
and pin the new control off in historical exporters and baseline-sensitive
helpers whose outputs it can affect, especially the argument-X corpus.

Compare only the new flag with generalized X forwarding and all preceding
optimized strategies fixed on. Include the previous 28 runtime/origin pairs
and the new neutral fixture in both runtimes/origins. Record image/code/home
sizes, exact hashes, routine changes, frames, reservations, zero page,
candidate masks, live/unknown blockers, final applications, direct X-load
counts, retained stores, additional emissions/materializations, summary
evaluations and compile timings.

An accepted batch retains the existing bound of seven top-level emissions
for maps, routine/home costs and XEX costs; the refinement adds no full
materializations. Record actual counts for absent or wholly rejected
candidates as well. Avoid repeated entry analysis per candidate or turning
generic liveness/callee consumers into new optimization clients.

Execute exact measured TN/TNDBG images, exercise panel/dispatch behavior and
bind fresh default compilation to measured feature-on bytes. Extend the
measured-image checks with `ACTIONC_CALLEE_ENTRY_FLAGS_MEASURED_IMAGES`, pointing
to the distinct entry-flags directory, and a matching VM target
`callee_entry_flags_measurements`. Keep old artifacts and their oracles
reproducible. Preserve counted-string, overlap and post-call parameter-home
alias controls. `Strcat` remains a
113-byte/355-cycle control unless a separately explained new site exists.

Enable the new control in the optimized configuration only after qualification;
the conservative default remains off. Require a demonstrated neutral saving,
strict final XEX improvement for admitted batches, no corpus growth, unchanged
storage capacities and preserved consumed entry/exit contracts. Report TN
and TNDBG changes separately, including unchanged results and their blockers.
The ten historical blockers are a census to reclassify, not an acceptance
target of ten successful rewrites.

Update the pseudo-machine contract, this plan's completion record and the
documentation index. Keep mutable-routine incidental return flags unpublished;
an incoming mask does not establish an outgoing flag value.

Suggested commit: `Measure precise callee flags and enable qualified defaults`.

## Validation commands and scope

During development, run filtered `known_callees`, `machine_liveness`,
`argument_x_forwarding`, `argument_staging` and rewrite-driver library tests,
the new integration/VM targets and their directly affected controls.
Final qualification runs the full affected backend:

```sh
CARGO_INCREMENTAL=0 cargo test --locked --lib mir6502::
CARGO_INCREMENTAL=0 python3 - <<'PY'
from pathlib import Path
import subprocess
targets = sorted(p.stem for p in Path('tests').glob('mir6502*.rs'))
subprocess.run(['cargo', 'test', '--locked',
                *[f'--test={name}' for name in targets]], check=True)
PY
CARGO_INCREMENTAL=0 cargo run --locked --bin actionc-mir6502-sweep -- --emit-load fixtures/mir6502
CARGO_INCREMENTAL=0 cargo test --release --locked --lib callee_entry_flags_measure_corpus -- --ignored --nocapture
CARGO_INCREMENTAL=0 ACTIONC_CALLEE_ENTRY_FLAGS_MEASURED_IMAGES="$PWD/build/callee-entry-flags/measured" \
  cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test callee_entry_flags --test argument_x_forwarding --test argument_staging \
  --test pointer_byte_plus_one --test pointer_load_sharing \
  --test register_copy_forwarding --test scalar_captures \
  --test call_predicates --test call_predicate_qualification \
  --test pointer_retention --test tn_codegen_reproducers --test tn_dispatch
CARGO_INCREMENTAL=0 cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test callee_entry_flags_measurements -- --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test --locked --test mir6502_callee_entry_flags -- --ignored --nocapture
```

Run LF/CRLF MADS reassembly and relocation for the new positive fixture,
using the established integration-test design. Retain scoped direct-word,
indexed-byte and indirect-field runtime assertions and applicable selection
assertions, plus frozen Strcat static/VM audits. Avoid unfiltered Cargo fixture
wrappers; retain the documented indexed-byte selection-count discrepancy as
separate from runtime correctness. Use fresh measured-image paths for VM
equality checks, and execute the matching new measured TN oracle explicitly.

This is MIR6502 analysis/rewrite metadata work. No other backend or shared
NIR contract change is planned. If implementation changes NIR, semantic
lowering, verifier or printer contracts, run the required shared-contract
checks from `AGENTS.md` as well. Planning changes alone need content/link
checks, with no compiler test run.

## Completion record

| Slice | Status | Evidence |
| --- | --- | --- |
| 1 Baselines and incoming masks | Complete, `6ff44406` | Frozen images/facts, typed blocker census, inert flag analysis and neutral execution controls |
| 2 Forwarding integration | Complete, `0ab5a1b6` | Opt-in N/Z proofs, callee body witnesses and final input-contract replay |
| 3 Flag and snapshot qualification | Complete | Flag/CFG truth tables, simultaneous replay, exhaustive VM observations and measured TN execution |
| 4 Corpus and default | Planned | |

Slice 1 freezes eight image controls in
[baseline.json](benchmarks/6502-callee-entry-flags/baseline.json) and records
all ten flag blockers in the [typed census](benchmarks/6502-callee-entry-flags/census.json).
The neutral fixture is 69 bytes in either runtime/origin. Its positive callee
masks a parameter before observing flags, defining N/Z while preserving C/V;
the immediate observer sibling retains the negative case. Three focused
analysis tests, nine MIR6502 machine-liveness tests, integration and 3,072 VM
calls pass, including raw/optimized NIR and actual LF/CRLF source parsing.
Both runtimes/origins match the frozen neutral images. The entry-proof mode
uses certified definitions while the existing liveness policy and selector
stay unchanged.

Slice 2 adds opt-in precise facts only for candidate targets that need them.
The earlier coarse path and default settings retain their frozen behavior.
The neutral `Forward` shrinks by two bytes (69→67-byte images) while
`UnsafeForward` retains its reload. Six focused tests pass, including wrong
masks/targets, stale callee bodies with unchanged masks and weakened final
input contracts. Integration and 6,144 VM calls pass across feature settings,
raw/optimized NIR, runtimes/origins and LF/CRLF input. Existing argument-X,
argument-staging and rewrite-driver controls also pass. Proofs carry immutable
callee bodies; final replay rebuilds their facts from the combined proposal.

Slice 3 passes ten focused tests and two VM tests. The execution matrix now
covers 22,144 calls, including all 256 forwarded bytes and all 16 C/N/Z/V
inputs, plus 64 calls across predecessor-feature combinations. Normalized
parameter/output write traces and returned A/X/Y/status/SP agree between
settings. CFG tests cover each observed/requested mask, joins, loops and
unreachable reads; ADC/SBC/ROR keep incoming carry dependencies. Explicit
contracts and individual N/Z inputs retain fallback. Combined caller/callee
selection passes final replay and saves three bytes in its typed fixture.

The corpus exporter supplies exact images for execution. Eight TN/TNDBG
images pass 80 string and 16 post-call parameter-alias cases; `Strcat` remains
113 bytes and 355 cycles. The measured neutral images are 69→67 bytes and
TNDBG is 12,954→12,953 bytes in cartridge mode; TN remains 9,858. The new
control remains disabled until slice 4 completes backend/default qualification.
