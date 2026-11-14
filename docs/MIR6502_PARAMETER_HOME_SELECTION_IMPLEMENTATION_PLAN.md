# MIR6502 parameter home selection implementation plan

Prepared 2026-11-14. Status: slices 0–1 complete; slices 2–6 pending.

Implements the [parameter home selection design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md).
Work in the dependency order below and commit each slice after its checks pass.
Keep unrelated working-tree changes outside those commits.

## Objective and fixed scope

Give ABI capture, parameter accesses, pointer preparation, allocation, and
emission one authoritative parameter binding. Demonstrate smaller emitted code
and data for eligible leaf routines, including TN's `Fnamecmp`, without changing
Action semantics, verified NIR, or the public ABI.

Initial eligibility is ordinary internal Action routines with at most four
incoming argument bytes, non-exposed BYTE/WORD scalar parameters, and no calls,
helper-producing operations, machine bodies, or unsupported memory effects.
Mutable pointer parameters are included; indirect writes are excluded initially.
Address exposure, descriptors, observable entry conventions, and SArgs captures
retain their current implementation.

One location serves each live parameter lane; no live-range splitting, deferred
spills, cross-call residence, or new general register allocator. Shared Y
induction, pointee-read elimination, flag reuse, and return-tail merging remain
outside this plan. The handwritten 47/51-byte Fnamecmp variants are not acceptance
sizes for this work.

## Code boundaries

| Area | Existing code and intended responsibility |
| --- | --- |
| Binding and verification | [ir.rs](../src/mir6502/ir.rs), [verify.rs](../src/mir6502/verify.rs), and [printer.rs](../src/mir6502/printer.rs): typed identities, binding state, phase checks, readable locations. |
| Selection boundary | [mod.rs](../src/mir6502/mod.rs) and [materialize.rs](../src/mir6502/materialize.rs): retain the selected pre-capture input and resume core materialization without repeating upstream searches. |
| Entry and parameter consumers | [materialize/abi.rs](../src/mir6502/materialize/abi.rs), [materialize/calls.rs](../src/mir6502/materialize/calls.rs), and [param_availability.rs](../src/mir6502/analysis/param_availability.rs): entry transfers and current value availability. |
| Resource constraints | [materialize/regs.rs](../src/mir6502/materialize/regs.rs), [materialize/temps.rs](../src/mir6502/materialize/temps.rs), [materialize/zp.rs](../src/mir6502/materialize/zp.rs), and [storage_ownership.rs](../src/mir6502/analysis/storage_ownership.rs): interference, scratch reservations, and ownership. |
| Physical consumers | [materialize/layout.rs](../src/mir6502/materialize/layout.rs), [analysis/record_bases.rs](../src/mir6502/analysis/record_bases.rs), and [emit.rs](../src/mir6502/emit.rs): one physical interpretation, emitted storage, effects, and maps. |
| Final acceptance | Existing executable preparation, [terminal cleanup](../src/mir6502/late_cleanup/mod.rs), and [preservation](../src/mir6502/late_cleanup/preservation.rs): candidate-specific downstream proofs and final output costs. |

Add a focused parameter-home selection module rather than putting candidate
policy into generic IR or emission. Reuse existing CFG, machine/home liveness,
effects, ABI placement, and physical-overlap queries. No SemIR lookback or
source-name matching is permitted.

## Slice 0 — reproducible baseline and controls

Commit the design/plan and add a small measurement harness using the existing
`tools/compare6502` conventions. Store normalized source hashes, compiler
revision/configuration, commands, and measurements under a new
`docs/benchmarks/6502-parameter-homes/` receipt directory. Generated executables
and large listings stay under ignored `build/`.

Retain the real TN routine through the complete executable pipeline. Record
Fnamecmp code bytes, its inline parameter bytes, and total TN image size for
cart/standalone at `$3000` and a shifted origin. Recheck the design's historical
185 code / 4 parameter byte measurement; record any baseline difference rather
than updating expectations silently.

Create compact source controls for register use, scalar ZP residence, two mutable
pointer parameters, and required fallback. Include retaining callers so inlining
or reachability cannot accidentally remove the subject. Disable inlining only
in isolated placement probes; also retain normal optimized end-to-end runs.

**Checks and exit:** current integration/VM controls execute correctly; new
newline-sensitive loading handles LF and CRLF through the actual path. The
harness can reproduce baseline code/home/image measurements without relying on
the prior ignored handwritten artifacts. Commit baseline and controls separately
from compiler changes.

## Slice 1 — authoritative binding representation, unchanged fallback

Add a typed binding table to the routine/frame contract. Use routine-qualified
`ParamId` and byte offsets; distinguish canonical incoming locations, working
homes, and outgoing result locations. Derive ABI positions through the existing
ABI helper, not a second hard-coded argument map.

Represent inline storage, register lanes, reserved ZP byte/pair, and absent
storage. A routine is explicitly either legacy or bound. For bound routines,
every parameter has exactly one binding and conflicting legacy allocation
metadata is rejected. Do not silently consult both representations.

Teach planning layout, emitted layout, physical-memory/effect queries, and MIR
printing to use the same resolution contract. Only inline bindings reserve
ordinary bytes. ZP bindings report their actual fixed addresses; register/absent
bindings cannot acquire address symbols. Display register locations in MIR and
diagnostics without inventing memory addresses in an address-only map.

Initially exercise bound inline storage through focused tests; production
selection remains disabled and the legacy path retains its current behavior.
Existing late parameter remappers must reject bound routines, preventing two
owners before their candidate logic is migrated in later slices.

**Checks and exit:** reject missing/duplicate bindings, bad widths/offsets,
register-as-address uses, incoherent pairs, and contradictory metadata. Forced
inline bindings agree across planning/final layouts and preserve executable
bytes. Feature-off artifacts stay identical to the frozen baseline. Run affected
MIR6502 IR/verifier/materialization/emission tests and ABI integration checks.
Commit the binding foundation.

## Slice 2 — selected checkpoint and bounded acceptance path

Split core preparation from the continuation immediately before pointer-scratch
reservation, ZP allocation, ABI capture, and initial layout. Retain the selected
input, effective configuration, helper requirements, and necessary selection
metadata as one owned compilation artifact, independent of tracing.

Thread that artifact through the nested pre-home selectors and inliner together
with their selected materialized result. A rejected inner trial must not replace
the checkpoint belonging to an accepted outer result. Do not recover it from
the last callback invocation, printed MIR, a global variable, or trace output.

Audit decisions outside the core continuation that affect resumption: inliner
helper placement, scalar-capture follow-up rewrites, layout-dependent alias
proofs, and caller rewrites consuming callee contracts. Carry required typed
obligations with the selected result and revalidate them through existing proof
logic against candidate placement. An untracked dependency declines the trial;
do not introduce historical parameter addresses to make it pass.

Add an opt-in `enable_parameter_home_selection` configuration gate, initially
false in both default and optimized configurations and subordinate to the
peephole master gate. An unchanged or empty proposal performs no extra core
materialization. A nonempty proposal gets one batch trial from the checkpoint,
with upstream choices frozen and selection disabled inside the continuation.

Factor the existing downstream preparation into a reusable continuation. Both
fallback and trial must include helper resolution, late transformations, checked
ranges, terminal cleanup, final verification, and finalized executable output.
Each owns its preservation state; neither uses the other's physical witnesses.
Do not put parameter remapping inside the terminal cleanup transaction.

Keep the historical materialized-MIR diagnostic endpoint; final home selection
runs in executable preparation. Its accepted bindings are visible in the final
pre-emission trace. Document this distinction so an earlier dump is not mistaken
for the cost-accepted executable.

Reuse final emission and packaging for acceptance, returning the accepted
artifact instead of emitting it again merely for reporting. The trial must have
no affected-routine code-plus-inline-home growth and a strictly smaller packaged
image. Any failed proof, unknown cost, invalid expansion, or growth retains the
complete fallback, including maps, effects, relocations, and reservations.

**Checks and exit:** checkpoint selection survives nested accept/reject cases;
baseline resume reproduces baseline output; no-op, failure, equality, and growth
paths restore exact fallback artifacts. Trace-on/off agree. Counters prove at
most one extra core continuation and no additional upstream search for this
feature. Record downstream emission/proof work separately; do not claim the
whole compiler emits only twice. Run phase-trace, inlining, layout, late-cleanup,
and executable-output integration checks. Commit orchestration while selection
still admits no production candidates.

## Slice 3 — direct register residence and absent homes

Implement the common eligibility query and entry-to-last-use analysis using
existing CFG/liveness/effect machinery. Include hidden A/X/Y/flag effects of
supported pseudo-op expansion and interference with temporary register plans.
Unknown expansion, possible helpers, unsupported reads/writes, or exposure
select fallback. Confirm constraints again on the expanded body.

For admitted native consumers, keep a BYTE or WORD's incoming register lane(s)
through their complete live range when every path permits it. Start with the
read-only register shapes that the existing consumer forwarding supports;
other mutations retain memory until their complete definition/use chain is
supported. Register reservation must be honored by temp materialization, not
only by the parameter selector.

Seed parameter availability from ABI entry and apply the real capture transfers.
Do not depend on stores to a removed inline cell. Preserve load-produced flags
when consumers require them, and invalidate old-value facts after mutations or
clobbers. Schedule necessary entry transfers without destroying uncaptured A,
X, Y, or fixed-slot inputs.

Move the existing write-only-home proof into the common absent-home candidate.
Remove only proved unobservable stores; retain the formal ABI position. Migrate
parameter reload/consumer forwarding to the binding query. Bound routines no
longer run the separate late write-only frame remapper.

**Checks and exit:** cover A/X/Y, word lane ordering, live N/Z, mutation barriers,
joins/backedges, entry transfer hazards, address exposure, SArgs and helper
fallback. Exercise callers and return conventions in both runtimes. At least
one independent source control must save finalized bytes with the feature on;
the existing feature-off path stays exact. Commit the first opt-in optimization.

## Slice 4 — ZP byte/word working homes and result handoff

Add allocation of complete parameter bytes/pairs to the existing reservation
machinery. Reserve both lanes atomically for a word; do not assume two unrelated
virtual-byte allocations will be adjacent. Extend consumers to resolve reads
and mutations through the binding, keeping original word/carry semantics.

Use a documented deterministic ordering over compatible locations: reusable
incoming locations, admissible ABI-window pairs (including `$A0/$A1` and
`$A2/$A3`), then available compiler ZP. Minimize entry transfers within that
ordering; never trial each address through materialization. Pointer-scratch
pairs remain unavailable until the next slice makes their consumers cooperate.
Honor all source exclusions, existing allocations, implicit requirements, and
simultaneously live parameter lanes. Failure to find a home selects inline
storage before building the one batch candidate.

Migrate leaf word-parameter/result coalescing into this selection. A result home
can hold a parameter only until its last use; all return paths must satisfy the
handoff. The old coalescer remains for legacy routines and cannot remap a bound
routine. Its already-profitable controls must not regress.

**Checks and exit:** cover read-only and mutable scalar words, carry boundaries,
live incoming `$A3`, multiple parameters, partial/early result overwrites, source
ZP reservations, occupied or fragmented pairs, and exhaustion. Planning layout,
final maps, callee effects, and VM execution must agree. Demonstrate no shadow
inline allocation and an independent emitted saving. Commit ZP scalar placement.

## Slice 5 — mutable pointer parameters and direct pair consumers

Admit eligible pointer parameters across blocks and loops. Extend pointer
preparation to consume their coherent resident pairs directly. Teach every
supported implicit scratch user to honor the binding; only then admit a
compatible existing pointer-scratch pair as a working-home candidate.

Keep parameter mutation explicit. `s==+1` changes the parameter's selected pair;
using `s` with an index does not permanently rebase it. Preserve original pointee
read count/order and all word overflow behavior. No shared Y induction or
load/flag reuse is added to obtain the saving.

Check each expanded operation's actual register/pair clobbers and rebuild callee
summaries. Validate previously consumed caller contracts; reject the batch if
they no longer hold. A late pass requiring a different binding must decline its
own rewrite, not silently recolor a live parameter. Reuse the explicit binding
proof for cross-block ownership rather than widening the block-local scratch
heuristic to all parameters.

**Checks and exit:** run independent two-pointer comparison/scan controls and the
actual TN Fnamecmp through optimized callers. Cover different base low bytes,
page carries, offsets around 255/256 and 511/512, wraparound with mapped ordinary
test memory, colon cases, and all byte comparisons with a terminating mismatch.
Also cover explicit source aliases, indirect-write rejection, competing scratch,
and a source leaf that lowers to a helper. Measure placement-only Fnamecmp and
whole-TN deltas. Commit pointer residence and its focused regressions.

## Slice 6 — backend qualification and default policy

Run the complete affected MIR6502 unit/integration suite and its runtime/ABI
consumers. Include direct register capture and SArgs/observable-entry fallback
paths, cart/standalone, shifted origins, listing reassembly, relocations, maps, and
actual LF/CRLF fixture paths. Scope execution to this backend unless shared
contracts are changed or failures indicate wider impact.

Compare feature-off, feature-on, and trace-enabled final artifacts across TN,
the new independent controls, and the existing tracing/cleanup controls. Freeze
all unrelated optimizer switches. Report routine code, ordinary home bytes,
ZP occupancy, packaged image bytes, and materialization/emission/proof counts;
record timing and memory use without inferring performance from one noisy run.
An upstream-choice census must show the new trial did not rerun their searches.

Require measurable TN/Fnamecmp savings and independent positive controls,
with all fallback and preservation counterexamples passing, before enabling the
optimized default. Keep the conservative default off. If qualification fails,
leave the feature opt-in and document the concrete unmet gate; do not describe
the optimization as completed or broaden its scope to chase the handwritten
target. After a default-policy change, run its affected CLI/configuration checks.

Update the design and pseudo-machine contract to distinguish implemented
guarantees from follow-ups. Publish a reproducible qualification receipt and
the final pipeline boundary; retire migrated parameter remappers from the bound
path. Commit qualification and the justified default policy.

## Test targets and evidence

Add dedicated `mir6502_parameter_homes` integration tests and `parameter_homes`
VM tests, with focused module tests alongside the implementation. Existing
consumers include `mir6502_call_abi`, `mir6502_pointer_setup`,
`mir6502_callee_entry_flags`, `mir6502_phase_trace`, `mir6502_late_cleanup`, and
VM `word_result_placement`, `callee_entry_flags`, `pointer_setup_bounds`, and
`tn_codegen_reproducers`. Extend relevant existing tests rather than duplicating
their assertions in a new harness.

Once the new targets exist, focused checks include:

```sh
cargo test --test mir6502_parameter_homes --test mir6502_call_abi
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test parameter_homes --test word_result_placement
```

Final unit qualification uses `cargo test --lib mir6502::`; enumerate the full
affected integration and VM target set in the receipt. Include the repository's
required MADS/runtime and cross-platform checks. These commands are scoped
examples, not a substitute for selecting the actual consumers of each slice.
No repository-wide NIR sweep or other-backend execution is required for the
planned backend-only work. If a shared NIR/semantic/printer contract changes,
the AGENTS.md shared-contract checks apply in full.

## Completion criteria

- One authoritative binding controls entry, accesses, reservation, and layout
  for migrated routines; no shadow inline homes or late parameter remapping.
- Register/ZP residence is verified across real expansion and every relevant
  CFG edge, with source storage and public ABI observations preserved.
- One batch trial, exact fallback, complete-image acceptance, and candidate-local
  preservation hold through the final executable path.
- TN and independent source controls show recorded savings; the handwritten
  combined optimizations are not counted as implemented gains.
- The backend checks pass, existing fallback behavior remains covered, and
  each completed slice has its own commit and validation record.
