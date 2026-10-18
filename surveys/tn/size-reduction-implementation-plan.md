# TN size reduction and record code generation implementation plan

Branch scope: the directory batch migration and its qualification described
here are preserved on `tn-ng` (checkpoint `4baaac89`). On `main`, maintained TN
is restored to `8ccdc683`, before that migration. Paths and completion statements
below refer to the migration branch unless explicitly described as pre-migration.

Status: Stages 0–7 complete, 2026-10-12. The maintained
[qualification report](size-reduction/README.md) pins the baseline and preserved
provider acceptance.

## Objective and scope

Recover TN/TNDBG code space and copy-buffer RAM while preserving the directory
model needed by **FujiNet**, the planned remote provider. Start with a measured
source simplification, fix the correctness issue exposed by the experiments,
then improve general MIR6502 record/pointer code generation. Review repeated
validation after the compiler changes make its remaining cost visible.

Production remains MyDOS during this work. Preserve generic paging, independent
panel state, selection across cache eviction, unknown-length listings and exact
operation-name resolution. Implementing the live FujiNet adapter is a separate
milestone. Its requirements govern the shared model; they do not justify
assuming a particular firmware cursor or name contract.

The [directory model contract](directory-model-contract.md) remains authoritative.
The [completed migration plan](directory-handling-implementation-plan.md)
includes the FujiNet follow-up requirements. Keep archived TN sources unchanged.

## Evidence and starting budget

Measurements use maintained source `74e1c6385a396c27296334c09693e0dd2adb7606`,
the cartridge runtime and compiler sources unchanged since `81dd7dc5`. The
pre-migration comparison uses source `0532f7d7` with that same compiler.

| Program/backend | Before migration: load bytes | Current: load bytes | Current copy RAM below `$A000` |
| --- | ---: | ---: | ---: |
| TN classic optimized | 10,549 | 18,352 | 8,558 |
| TN MIR6502 | 9,943 | 17,265 | 9,589 |
| TNDBG classic optimized | 13,794 | 21,841 | 5,069 |
| TNDBG MIR6502 | 13,048 | 21,025 | 5,829 |

For TN MIR6502, 6,753 of the 7,322 additional load-file bytes are instruction
bytes. The per-panel model grew by 86 bytes. Unchanged LIB routines have
unchanged sizes with the same compiler. This points to the new capabilities
and their expensive generated code, rather than establishing a broad recent
compiler regression.

The isolated experiments produced these results for TN MIR6502:

| Experiment | Load bytes saved | Copy RAM gained | Decision |
| --- | ---: | ---: | --- |
| Share entry-address calculation | 250 | 250 | Implement first; layout and behavior unchanged |
| Store pointers in the order table | 270 | 142 | Defer; two panels require 128 extra bytes |
| Remove entry `position` | 49 | 305 | Retain the field pending FujiNet contract review |
| Remove generic paging for MyDOS | 811 | 811 | Excluded; paging is required for FujiNet |

The first three experiments passed all three existing behavioral targets in
classic and MIR6502, including model/reader LF and CRLF paths. The last passed
MyDOS checks only and deliberately excluded paging. These are independent
experiments: their savings must not be added together without rebuilding the
combined implementation. No cycle or disk-level measurements were made.

An additional mask-table experiment exposed a classic miscompile: after an
indexed lookup, generated `TXA` uses the table index as the mask. The guarded
bitmap received 7 where 129 was expected. Its apparent size saving is rejected.

The analysis artifacts currently reside under ignored `target/tn-size-analysis/`.
Stage 0 must retain reproducible evidence in the maintained survey before those
artifacts can be cleaned up.

## Contracts every stage must preserve

- A cache holds at most 64 entries; MyDOS reads and sorts its complete listing.
  Remote windows retain provider ordering, absolute ordinals and end discovery.
- Cache replacement within the same listing preserves tags. Refresh, identity
  loss or an unrepeatable listing invalidates the selection generation. A cursor
  alone is not proof of stable identity.
- Invalidation precedes provider writes and potentially nonlocal I/O failures.
  Failed or malformed reads cannot publish a partially valid batch.
- Browsing beyond selection capacity remains possible. Unknown-length tag-all
  must finish discovery and capacity checks before file-operation side effects;
  it must never silently operate on a partial selection.
- Preview bytes are never operation names. Full-name resolution checks capacity
  before writing and preserves both panels' independent location/listing state.
- Checked public routines retain their error and no-write guarantees. Any
  internal unchecked path has explicit preconditions and a bounded lifetime.
- Action! ABI homes and observable results remain intact. In particular, CARD
  results use public `$A0/$A1` result cells; A/X is not a universal return ABI.

## Delivery sequence

Each behavior-changing slice carries its regression coverage, measurements and
one focused commit. Update this plan's status as stages complete. Do not mix a
classic correctness fix, an application refactor and a MIR optimization in one
commit. Stage 0 establishes the baseline; Stage 1 is the first size win.

### Stage 0: retain evidence and freeze acceptance

1. Add a compact maintained measurement report with source/compiler revisions,
   runtime, origins, load bytes, emitted/deferred storage, final copy-buffer
   address and shared homes. Use `measure-directory.py`; put generated binaries
   and listings under `target`, rather than checking them in.
2. Retain a minimal reproducer for the mask-table failure and its expected
   bitmap behavior. Record the failing compiler/mode without disabling the
   existing TN tests or committing a failing normal-CI test.
3. Identify the existing independent checks for provider publication, eviction,
   identity loss, long exact names and two-panel cursor restoration. Add only
   missing behavior cases needed by subsequent changes.
4. Record that `position` is provider metadata and remains present. Verify the
   chosen FujiNet API/firmware contract before proposing a different field or
   representation; do not equate a storage slot, ordinal and provider cursor.
5. Establish deterministic CPU-only VM measurements for sorting, drawing,
   tagging and publication on representative 0/1/64-entry and paged listings.
   Keep substituted I/O/screen work separate and report its scope. Size remains
   the primary budget, but extra calls and repeated checks need cycle evidence.

Acceptance: the current four size measurements are reproducible, the classic
failure is independently understood, and preserved provider behavior has an
explicit test owner. No production code change is required in this stage.

### Stage 1: share entry addressing

Add one `BatchItem(batch, slot)` helper in `DIR.ACT` for the existing
`@batch.items(slot)` computation. Its internal precondition is a storage slot
below `BATCH_CAPACITY`; it does not accept an absolute ordinal or validate a
published listing.

Use it at the five measured sites: `CachedEntry`, the two entry lookups in the
sort comparison, the post-sort ordinal pass and the MyDOS reader. Keep
`CachedEntry`'s public checks and the byte order table unchanged. Preserve both
maintained roots through their shared includes.

Acceptance: all three TN behavioral targets and storage checks pass, with no
layout changes. Reproduce the measured 250-byte saving in each MIR root and
311/315 bytes in classic TN/TNDBG, or explain any new-baseline difference.
Report the helper's cycle cost, especially inside sorting; do not infer speed
from its size saving. If compiler work later makes the helper unprofitable,
remeasure the shared and expanded forms before choosing between them.

### Stage 2: repair classic indexed-value tracking

Reduce the rejected mask-table experiment to a generic indexed load followed
by local storage and indirect compound assignment. Fix the register-value fact
that incorrectly permits a transfer from X after X contains the index. Use the
classic state/tracked-emission machinery, not a TN-specific source workaround.

Focused execution coverage must distinguish table values from indexes for all
eight bit positions, set and clear operations, intervening clobbers and relevant
branches. Keep the indexed source evaluation count and memory-write guards.
Cover cartridge and standalone output; use MIR6502 as a behavioral comparison.
Existing unrelated classic workarounds remain until their own reproducers pass.

Acceptance: the reproducer fails before and passes after the fix; the full TN
checks stay green. A mask lookup or masked `BatchStart` rewrite is optional
follow-up work only if separate size/cycle measurements justify it. It is not
part of the correctness commit.

### Stage 3: extend typed address-use proofs

Build on existing MIR6502 analyses and rewrite contexts. They already provide
use/def, dominance, liveness, effects, indexed consumers and fixed pointer-pair
facts; do not introduce a parallel analysis framework or allocator.

Add only the missing facts needed to describe an address use: captured pointer
value, constant byte displacement, dynamic index/stride, access width, consumer
and whether the address itself escapes. Separate reuse of an already captured
pointer from reloading mutable pointer storage. Start with same-block uses.

Prove single evaluation, producer dominance, remaining uses, machine clobbers
and alias barriers through the shared contexts. Distinguish unknown indirect
memory from proven private homes. A private compiler home is not by itself a
proof that an arbitrary pointer cannot alias its physical address.

Range-dependent decisions must use verified value/guard facts. An embedded
array's declared capacity does not automatically prove its index is in range.
Record rejection reasons for missing ranges, escapes, aliases, multiple uses
and machine-state conflicts.

The implementation begins in `mir6502/analysis`, `rewrite/context.rs` and the
existing `materialize/indexes.rs`, `pointers.rs` and `lea.rs` consumers. Extend
the [pseudo-machine contract](../../docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md)
when introducing a MIR representation or phase invariant. NIR continues to
own typed places, field offsets, normalized values and conservative effects;
MIR must not consult SemIR or printed source to recover them.

Acceptance: generic positive/negative proof tests pass; this foundation slice
leaves output unchanged. If a necessary fact is absent from verified NIR,
implement and document that shared-contract extension as a separate vertical
slice with the required full shared checks.

### Stage 4: consume record addresses without avoidable pointer homes

Implement three separately measured slices using Stage 3's proofs:

1. Constant byte/word field accesses: keep a usable record base and select a
   displacement at the load/store consumer instead of constructing a temporary
   pointer for every field.
2. Embedded byte-array accesses: combine field displacement and index only
   when the complete access fits the selected 6502 form. Otherwise adjust the
   base correctly and retain the full-width address path.
3. Record-element addresses: construct base + field displacement + scaled
   index directly for a proven consumer. Compare constant-stride expansion with
   existing helper cost; retain explicit addresses when they escape or remain
   live for other uses.

Test offsets 0, 254, 255 and 256, accesses crossing pages, both record
alignments, maximum indexes and 16-bit address wrap. A word access beginning
at Y=255 must not wrap its second byte back to Y=0 of the same base. Include
aliased source/destination, effectful index evaluation and volatile accesses.
Eliminate producer temps only after routine-wide deadness is proven.

Acceptance: general fixtures execute identically in cartridge and standalone
output. Report fewer constructed pointers/homes and final bytes in the current
TN routines, including `CachedEntry`, `MyDosParse`, `MyDosRender` and
`MyDosRoot`. Compare at least two non-TN record/pointer workloads; routine names
and field spellings must never participate in matching.

### Stage 5: reuse proven bases and shorten private value flow

Split this stage into two commits so their effects are independently visible.

**5A — base availability.** Reuse a prepared zero-page pointer pair within one
block when its exact captured value remains available. Preserve only facts
proved by the existing machine/effect analyses. Pointer-cell changes,
potentially aliasing writes, calls, machine blocks and unknown effects block
reuse unless a structured proof says otherwise. Extend across a simple CFG
edge only as a subsequent measured slice, with must-agree predecessor facts.

**5B — field values and call consumers.** Forward captured field values into
stores, arguments and immediate result consumers without intermediate private
home copies when use/def and liveness allow it. Preserve argument evaluation
order, nested-call results, canonical parameter placement and public result
stores. Reuse the existing call plans, home planning and copy cleanup; do not
change the public ABI or suppress source-visible storage.

Acceptance: aliasing stores, mutable pointer cells, interleaved calls, live
flags, multiple users and nested argument evaluation have negative coverage.
Report final homes and executed private accesses alongside code/cycles. Do not
claim that every load/store byte in the earlier census was redundant. The
[rewrite workflow](../../docs/MIR6502_REWRITE_WORKFLOW_PLAN.md) owns proof
queries, transactional application and analysis invalidation.

### Stage 6: make validation ownership explicit

Reprofile after compiler savings. Review one application path per slice:

- `ToggleTag`: compute/check its bit once and share the mutation logic, while
  keeping checked `SetTagged` and invalid-ordinal behavior intact.
- `TagsRecount` / `NextTagged`: exploit an already clamped iteration limit
  through a narrowly internal bit-access path. Verify partial bitmap bytes,
  extent trimming and tag-all exceptions.
- `Draw`: consider a checked viewport followed by bounded cache access only
  if no provider call, panel switch or invalidation can occur during that use.
  Do not carry a cache pointer across eviction or I/O.

Keep `BatchPublish`'s provider-facing checks and pre-I/O navigation validation.
`MyDosCanEnter` before CHDIR and the checked standalone `MyDosPush` contract
serve different callers; a trusted post-I/O commit requires a proof that its
validated input/state cannot change. Defer it unless that proof and measured
benefit are clear.

Acceptance: each trusted path has documented preconditions, all public error
and no-write cases remain tested, and the combined TN/TNDBG result improves
size or measured cycles with its RAM/size tradeoff reported. Additional helper
bodies can increase size; deleting repeated checks is not sufficient evidence
of a win. Do not redesign selection counting and validation simultaneously.

### Stage 7: qualify, set budgets and decide metadata follow-up

Regenerate all four builds and compare load bytes, emitted/deferred storage,
shared homes, copy RAM and the Stage 0 CPU workloads. Publish cumulative and
per-slice results in the maintained survey and sample notes. Keep deterministic
size regression ceilings separate from timing observations and brittle exact
instruction counts.

Retain the byte order table by default: the pointer-table experiment sacrifices
128 bytes of cache RAM for only 20 bytes of additional MIR code saving over the
shared helper. Retain `position` unless the pinned FujiNet contract establishes
that exact-name resolution and cursor restoration need no per-entry field.
Any justified layout change is a separate slice updating both the production
contract and independent host layout/provider oracles, with fresh combined
measurements. Completing this plan does not depend on removing the field.

Run final affected-backend qualification and a MyDOS smoke check with disposable
disks when an emulator/hardware interface is available. If unavailable, report
that gap explicitly; substituted CIO checks do not establish real disk behavior.
Live FujiNet protocol/device acceptance belongs to the later provider milestone.

Acceptance: existing functionality and FujiNet-facing contracts remain intact,
the source saving is delivered, the classic bug is fixed, general compiler
improvements have measured consumers, and the new memory budgets are documented.
Set further savings targets from those measurements rather than promising an
unsupported percentage of the original 7 KB growth.

## Validation and execution policy

For source/layout changes, run the existing independent behavioral matrix:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test tn_dispatch --test tn_directory_model --test tn_mydos_directory
cargo test --locked --test nir_storage_analysis \
  tn_exposes_high_value_scalar_promotion_candidates
cargo test --locked --lib tn_deferred_storage_starts_after_final_mir_bytes
python3 surveys/tn/measure-directory.py \
  --compiler target/debug/actionc --out-dir target/tn-size-reduction/current
```

Build the compiler from the intended revision before measuring. The three VM
targets cover both cartridge backends; model/reader paths exercise LF and CRLF.
Preserve paging and failure checks throughout. Add only the generic regressions
needed for each new compiler rule; use the embedded-record runtime target when
embedded-array/address behavior changes, including its standalone mode.

For MIR6502 changes, run focused analysis/materialization tests and affected
integration targets such as `mir6502_materialization_gap`, `mir6502_store_consumer`,
`mir6502_call_abi` and `mir6502_rewrite_inventory`. Substantial emission changes
and Stage 7 require the full affected 6502 backend suite and its runtime/ABI
matrix. Classic fixes likewise require affected classic state/codegen and
runtime checks. Do not execute unrelated native backend suites for backend-only
work.

Any change to shared semantics, NIR, verification, printing or their contracts
also requires these repository checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Classify fixture changes as intentional IR contracts, target materialization,
printer-only changes or bug fixes. If newline-sensitive instrumentation changes,
exercise actual LF/CRLF parsing paths; full-root text changes also need isolated
CRLF builds. Normalize host text only, preserving binary and ATASCII bytes.

Reuse existing target caches and keep only one baseline/final artifact set plus
small reports and reproducers. Protect existing frozen compiler qualification
artifacts. A passing stage is not rerun after documentation-only edits unless
new evidence requires it; final qualification checks the integrated result.
