# MIR6502 pointer load sharing implementation plan

Prepared 2026-11-11. Status: planned. Compiler baseline: `9488d269`.
Implement in four vertical slices and commit each after its checks pass.
This plan contains the bounded rewrite contract; no new executable MIR form
or separate design note is required for this first schedule.

Reuse the original pointer bytes already loaded while computing `q+1` to
prepare the following `q^` read. The current optimized `Strcat` loads each
byte twice. Moving its existing scratch-pair stores beside the first loads
could eliminate two absolute loads: a local opportunity of six bytes and
eight cycles. Final layout and execution must establish the actual saving.

The [preceding measurements](benchmarks/6502-pointer-byte-plus-one/README.md)
freeze `Strcat` at 120 bytes and TN cartridge at 9,868 XEX bytes. A six-byte
local saving would make those 114 and 9,862 respectively if it survives final
qualification. These are investigation targets, not acceptance assumptions.

## Scope and ownership

Select one exact, contiguous, same-block sequence after normal materialization.
The source is a complete ordinary direct parameter/local word S. An existing
fixed zero-page capture pair C receives `S+1`; an existing fixed scratch pair P
then receives the original S for one immediate-offset-zero indirect byte load
into A. Require complete, non-wrapping, physically disjoint S, C and P ranges.
Use the backend's existing ordinary-RAM scratch pairs for P; preserve their
reservations. Other source classes, constants/operators, consumer offsets,
scaled/live-Y strategies and nonadjacent forms retain their existing paths.

SemIR and NIR retain their current source meaning and computation. MIR6502
owns load sharing, carry, register/flag equality and physical alias proofs.
Emission writes the existing explicit loads, stores, arithmetic and indirect
load. Do not add an emitter cache, assembly-pattern optimizer or source-name
special case. No ABI, source storage, frame or allocation change is planned.

The call is outside the rewrite window. Discovery requires neither a scalar
callee signature nor a machine-callee preservation annotation. Calls, helpers,
machine blocks, barriers, indexed accesses and additional operations cannot
occur inside this first template. Keep parameter homing, outgoing captures,
argument order, calls and post-call pointer reconstruction intact.

## Exact schedule

In the table, each row is an existing MIR operation. `add.low` is byte Add
with constant 1, clear carry-in and produced carry-out. `add.high` is byte Add
with constant 0, carry from the previous lane and ignored carry-out. Both use
A as destination and left operand. All direct loads/stores are byte-wide.
Retain the backend's existing decimal-mode-clear arithmetic invariant.

| Original operations | Replacement operations |
| --- | --- |
| `A = load S.lo` | `A = load S.lo` |
| `A = add.low A, 1` | `store P.lo, A` |
| `store C.lo, A` | `A = add.low A, 1` |
| `A = load S.hi` | `store C.lo, A` |
| `A = add.high A, 0` | `A = load S.hi` |
| `store C.hi, A` | `store P.hi, A` |
| `A = load S.lo` | `A = add.high A, 0` |
| `store P.lo, A` | `store C.hi, A` |
| `A = load S.hi` | `A = LoadIndirect(P, offset=0)` |
| `store P.hi, A` | |
| `A = LoadIndirect(P, offset=0)` | |

This is an 11-operation window becoming nine. In current TN, S is `t`, C is
`$A2/$A3` and P is `$AC/$AD`. `$A2` is a staging capture; the actual third ABI
byte is passed in Y. Preserve both capture stores and the later register
reloads. Match typed byte identities and carry fields, never these names or
the enclosing `Strcat`/`MovePage` routines.

Include the terminal `LoadIndirect` in the proof window. Immediately before
it, A and N/Z differ: the original carries S.high and the replacement carries
the arithmetic high result. The unchanged indirect load establishes equal A
and N/Z, while its unchanged Y setup establishes equal Y. X, SP and D are
unchanged; C/V come from the same high-byte addition. Prove complete exit-state
equality through this consumer. A variant ending before it would require
different liveness proofs and is outside the initial schedule.

## Required proof

Provide one shared typed proof query, provisionally
`analysis/pointer_load_sharing.rs`, exposed through the post-home rewrite
context. Its witness records routine, generation, exact block/range, all four
source-read sites, both arithmetic operations, retained stores, terminal
consumer and physical byte ranges. Reconstruct the exact replacement from
that witness. Missing shape, ownership or placement facts reject selection.

Establish these obligations together:

- The two original loads of each source lane identify the same byte. S has
  two resolved ordinary-RAM bytes from the same parameter/local word. Logical
  naming or register residence alone cannot establish their physical identity.
  Reject absolute/hardware-backed, unresolved and unsupported source storage.
- S is physically disjoint from both capture writes and both pointer writes.
  The original late reads therefore see the same bytes as the retained early
  reads. Check separately named aliases with the final map; different IDs are
  insufficient. Unknown writes cannot certify source stability.
- C and P are complete, non-wrapping, mutually disjoint pairs in ordinary RAM.
  Moving the low P store cannot corrupt the following source-high read or a
  capture. Moving either P store cannot affect any crossed read. There are no
  observers of the partially built pair inside the exact template.
- Both arithmetic inputs and the full carry chain are unchanged. Each moved
  store preserves A and flags, including carry between the low/high additions.
  No additional source read, stack operation or register repair is introduced.
- Every original capture and pointer store survives, with equal values and
  lane order. Only their interleaving changes. All source-visible/ABI writes
  keep their values and relative order; no store or home deletion is implied.
- The terminal indirect read remains after complete C/P setup. Its effective
  address, value and read count are identical because the memory at that point
  is equal. Its target may alias S, C, P or another ABI cell; treat those entry
  states consistently rather than assuming the target is disjoint.
- Full memory and machine state agree at the window exit. Surviving users,
  successors and loops must receive the same values. Preserve all published
  callee result/preservation facts consumed by earlier final selectors.

The generic post-home dependency checker proves neither store-motion legality
nor value equality merely because write counts match. Keep that checker and
its unknown-reader rejection intact. Construct a structural plan only after
the typed query succeeds, and validate its witness against the exact original
and replacement at application. Add a narrowly typed proof field/validation
hook to the existing plan/driver if needed; a rule-name test, asserted Boolean
or aggregate effect equality cannot authorize the motion. Revalidate against
the supplied physical map, not addresses copied into an unchecked witness.

## Integration and cost limits

| Code home | Planned work |
| --- | --- |
| `analysis/pointer_load_sharing.rs`, [context.rs](../src/mir6502/rewrite/context.rs) | Exact typed proof, physical aliases and complete exit-state obligations |
| [call_predicates.rs](../src/mir6502/analysis/call_predicates.rs), [record_bases.rs](../src/mir6502/analysis/record_bases.rs) | Reuse ordinary-source classification and physical memory facts |
| [plan.rs](../src/mir6502/rewrite/plan.rs), [posthome.rs](../src/mir6502/rewrite/posthome.rs), [driver.rs](../src/mir6502/rewrite/driver.rs) | Structural transaction, generation and application-time witness validation |
| `materialize/pointer_load_sharing.rs`, [mod.rs](../src/mir6502/mod.rs) | Final discovery, snapshot rebuilding, layout replay and bounded costing |
| [register_copies.rs](../src/mir6502/materialize/register_copies.rs), [address_values.rs](../src/mir6502/materialize/address_values.rs) | Existing physical-reference emission and routine/home costs |
| [known_callees.rs](../src/mir6502/analysis/known_callees.rs), [effects.rs](../src/mir6502/analysis/effects.rs) | Reuse existing byte effects and preserve published exit contracts |
| [passes.rs](../src/mir6502/passes.rs) | Independent `enable_pointer_load_sharing` control |
| [pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md) | Document this scheduling invariant and unsupported forms |

Run the selector after the complete existing final pipeline, including pointer
byte increment fusion. Keep earlier inlining, allocation, selection and their
cost trials identical between feature settings. Initially leave the new flag
false in conservative/default and optimized configurations; enable optimized
only after slice 4 qualification. The peephole master switch still controls it.

Use a cheap exact-shape scan before acquiring a physical map or building
analyses. Discover one legal transaction per snapshot through the shared
driver. Each transaction removes two operations, so the original operation
count plus one bounds convergence. Do not add a general scheduling window or
materialize the whole program per candidate.

Measure the complete accepted baseline, proposed batch and at most one batch
with rejected routines restored. Require unchanged frames, home capacities,
reservations and occupied zero page; no affected-routine growth; no whole-image
growth; and a strict final XEX byte saving. Rebuild the physical map for the
combined proposal and replay accepted transactions from the baseline using
its identities. This proof depends on S's final address as well as the fixed
capture/pointer pairs. Unknown cost, lost alias proof, replay mismatch,
verifier failure or weakened published exit facts retains the baseline.

Bound costing to seven additional emissions per selected batch: two physical
maps, three routine/home cost measurements and two final XEX measurements.
There are no additional whole-program materializations. Short-circuit earlier
on failure, restore rejected routines before final replay, and retain the
complete baseline if the combined placement cannot be qualified. Record actual
emission/materialization counts in the results.

Report `pointer-load-sharing-candidate`, `selected`, `applied` and specific
blockers for shape, ordinary storage, placement/overlap, ordered access,
carry/machine state, stale witness, published exits and final proof/cost.
Count applications only after final acceptance. Keep counts stable when
snapshots shift neighboring operation indices and when reporting is enabled.

## Slice 1 Freeze the miss and prove the schedule

1. Freeze compiler/source/dependency hashes, raw/materialized MIR, listings,
   images, routine sizes, frames and zero page at `9488d269`. Preserve the
   preceding optimization as enabled in both settings. Keep large artifacts
   under ignored `build/pointer-load-sharing/before/` and a compact manifest
   under `docs/benchmarks/6502-pointer-load-sharing/`.
2. Add `fixtures/runtime/pointer_load_sharing.act`, an integration target
   `mir6502_pointer_load_sharing` and a VM target `pointer_load_sharing`.
   A confirmed contextual starting shape is
   `Observe(p+p^+1,q+1,q^)`, with runtime pointer parameters, an independent
   WORD/WORD/BYTE observer and a live caller. Confirm the exact window with
   optimized NIR; a simpler arithmetic first argument can use a different
   materialization path and is a control until inspected.
3. Freeze TN/TNDBG at `$2C00` and the neutral fixture at `$3000`/`$41C7`, in
   cartridge/standalone runtimes. Resolve routine/parameter identities from
   maps. Confirm which source loads survive earlier register-copy cleanup.
4. Add the inert typed proof and tests for exact shape, complete words, final
   aliases, carry fields, terminal consumer, stale generations and wrong
   replacements. Document application-time validation without weakening
   shared dependency or ordinary-memory rules.

Acceptance: the neutral case has correct baseline arguments and a confirmed
miss. Unsafe shapes reject and compiler output remains byte-identical.

Suggested commit: `Freeze pointer load sharing baselines and schedule proofs`.

## Slice 2 Select and cost the exact schedule

1. Connect the shared proof to a small final selector, replacing the exact
   11 operations with nine existing operations. Add the independent flag,
   initially disabled, without changing earlier producers or emission.
2. Apply through generation-qualified transactions; reconstruct and validate
   the witness at application. Test a deliberately changed source/capture,
   consumer or physical map cannot reuse an earlier proof.
3. Implement bounded routine rollback, physical replay, final verifier and
   strict image-cost checks. Revalidate already-published callee contracts
   across the complete accepted program. Unknown emission cost restores the
   full prior program.
4. Test idempotence, neighboring candidates, iteration bounds, partial-batch
   acceptance, combined-layout failure and identical images with reporting
   off/on. Retain earlier arithmetic, comparison and argument selections.

Acceptance: a feature-on optimized-NIR neutral case removes the two redundant
loads with equal arguments, results, exit state and storage capacity. Record
its actual routine/image saving; rejected cases retain their old strategies.

Suggested commit: `Select proven pointer load sharing with final costing`.

## Slice 3 Qualify memory and machine boundaries

Batch distinct behaviors through independent host oracles, emitted sequences
and actual callee-entry observations:

| Area | Required cases |
| --- | --- |
| Arithmetic | Pointer-low values 0–255 on safe pages; bytes 0/1/127/128/254/255; low carry, high overflow and `$FFFF+1` wrap in isolated fragments |
| Direct storage | Source aliases either capture/pointer lane, partial overlap, separately named physical aliases, missing placement, hardware/absolute source and incomplete words; each must retain fallback |
| Indirect observation | Target aliases source homes, capture cells, pointer lanes and other ABI cells; preserve its single read after complete setup and the correct alias-constrained value |
| Machine state | Incoming carry variants, full C/N/Z/V and A/X/Y/SP equality at exit, Y setup, live successor/call inputs and balanced stack; unsupported carry chains reject |
| Structure | Extra/interposed operation, barrier/call/machine block, changed consumer/offset, scaled/live Y, stale/wrong-owner proof, two adjacent candidates and repeated application |
| Existing strategies | Controls with second expression `q+0`, `q+2` or `q-1`, and an already-shorter argument path; preserve existing pointer increment fusion with its flag off/on |
| Execution | Raw/optimized NIR, both runtimes and origins, repeated runtime inputs, actual LF/CRLF parsing, guards, source-home preservation and observed ABI bytes |

Watch each source-home lane drop from two reads to one; retain the order of
the surviving low/high reads. Watch equal pointer-store values/counts and the
unchanged outgoing ABI write trace. Compare complete observable memory and
callee-entry flags/stack, rather than only final arithmetic results. Do not
seed indirect data over code, carrier or parameter bytes to manufacture
inconsistent aliases; exercise constrained aliases separately.

Run both post-call parameter-home overwrite probes on TN/TNDBG with current
owner-qualified maps. Their reconstruction remains required. Execute the
exact newly measured images, including positive-length strings, same-buffer
copies, overlap and page boundaries. Preserve the existing zero-length
`MovePage` behavior without redefining it in this optimization's oracle.

Acceptance: admitted schedules preserve the complete contract, and each unsafe
boundary demonstrably compiles through the baseline path.

Suggested commit: `Qualify pointer load sharing aliases and machine state`.

## Slice 4 Measure applicability and enable the optimized default

1. Add an opt-in corpus exporter, toggling only this feature, with optimized
   NIR and normal entry-reachable linking. Include the new neutral case,
   existing pointer-increment fixture, TN/TNDBG, direct word/indexed-byte
   arguments, scalar/predicate captures, a string loop, graphics and resident
   string calls. Distinguish absent templates from late proof/cost failures.
2. Record final routine/XEX sizes and hashes, code/reserved extents, frames,
   zero page, source-home read counts, retained writes, actual target cycles,
   compile timings and bounded costing counts. Compare both runtimes and the
   two neutral origins. Six local bytes need not imply six per whole image
   or one candidate per program.
3. Pin the new flag off in historical exporters and baseline-sensitive test
   helpers for comparisons, argument staging and pointer increment fusion.
   Keep their frozen manifests, images, cycle samples and role tables intact.
   Add current-default identity checks against the new measured images and
   a separate interaction matrix instead of comparing new defaults to older
   feature-on image hashes.
4. Enable the optimized default after focused checks and measurement pass,
   then run final scoped backend qualification. Conservative/default remains
   off. Commit compact results under
   `docs/benchmarks/6502-pointer-load-sharing/`, update this plan and the
   pseudo-machine contract/index, and keep large artifacts ignored.

Acceptance: a qualified neutral saving, honest corpus applicability, no image
growth, reproducible feature-off controls and passing backend/ABI checks.
Record any TN blocker as a result; do not widen the template to force a win.

Suggested commit: `Measure pointer load sharing and enable qualified defaults`.

## Validation commands

For this documentation-only preparation, check content, relative links and
`git diff --check`. Implementation introduces the proposed focused targets:

```sh
cargo test --locked --lib pointer_load_sharing
cargo test --locked --test mir6502_pointer_load_sharing
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test pointer_load_sharing
```

Final qualification follows [AGENTS.md](../AGENTS.md), scoped to MIR6502:

```sh
cargo test --locked --lib mir6502::
python3 - <<'PY'
from pathlib import Path
import subprocess
targets = sorted(p.stem for p in Path('tests').glob('mir6502*.rs'))
subprocess.run(['cargo', 'test', '--locked',
                *[f'--test={name}' for name in targets]], check=True)
PY
cargo run --locked --bin actionc-mir6502-sweep -- --emit-load fixtures/mir6502
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test pointer_load_sharing --test pointer_byte_plus_one \
  --test argument_staging --test scalar_captures --test call_predicates \
  --test call_predicate_qualification --test register_copy_forwarding \
  --test pointer_retention --test tn_codegen_reproducers --test tn_dispatch
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --lib direct_action_word_arithmetic_args_execute_through_the_vm_library
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --lib indirect_call_fields_execute_through_the_vm_library
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --lib indexed_byte_fixed_action_args_execute_through_the_vm_library
python3 tools/compare6502/tn/strcat_breakdown.py --check \
  --roles-output build/argument-staging/strcat-byte-roles.tsv
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test strcat_cost_breakdown -- --ignored --nocapture
```

Run the corresponding direct-word/indirect-field selection assertions with
the scoped VM library checks above; avoid their unfiltered Cargo wrappers.
The indexed-byte selection-count discrepancy is already documented in the
preceding results; keep its execution assertion as an independent control.
Add opt-in measured-image VM and listing/MADS checks for the new target,
including relocation and actual LF/CRLF assembly. Retain historical measured
image oracles with explicit feature settings. Use `CARGO_INCREMENTAL=0` when
needed to bound generated debug artifacts.

No shared NIR or semantic change is planned. If implementation changes those
contracts, also run `cargo test nir_fixtures_match_snapshots`,
`cargo run --bin actionc-nir-sweep -- fixtures/nir` and `cargo test` as required.
Explain any materialized-MIR expectation change as target scheduling; raw NIR
and its printed contract remain unchanged.
