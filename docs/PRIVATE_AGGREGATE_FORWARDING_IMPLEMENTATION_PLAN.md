# Private aggregate storage forwarding

Created: 2026-09-09.
Status: slice 1 proof foundation, bounded slice 2 fresh initialization and
slice 3 same-block snapshot read forwarding implemented. Cross-block CASE
reuse and aggregate ABI forwarding remain slices 4–5.

First increment: verified read-only byte-range/address-use/unchanged-interval
queries and the shared 12-case NIR/VM baseline are implemented. See
[baseline and remaining work](PRIVATE_AGGREGATE_FORWARDING_BASELINE.md).

Slice 2 now distinguishes fresh LET initialization from replacement, shares
destination-aware constructor preparation, and removes eligible nested staging.
Whole native call results can use the binding; routine-static calls and unsafe
constructors retain staging. See [implementation, limits and measurements](PRIVATE_AGGREGATE_FRESH_INITIALIZATION.md).
Slice 3 adds the bounded single-initialization/use-lifetime proof needed for
same-block rewrites, not general CFG initialization analysis. See
[proof boundary and measurements](PRIVATE_AGGREGATE_BOUNDED_FORWARDING.md).

## Objective and baseline

Remove unnecessary private aggregate temporaries and copies using shared
storage/value-lifetime proofs for records, unions and variants. Preserve genuine
snapshots. This follows `191aa2a` (avoid redundant variant constructor clearing),
not a new representation or language feature.

Cover both **fresh initialization**, where shared semantic lowering can avoid
creating staging storage in the first place, and **subsequent copy forwarding**,
where NIR must prove that already-created storage/copies are redundant. Fresh
initialization is implemented in slice 2 without depending on complete general
aggregate lifetime analysis or teaching NIR to follow mutable pointer relays.

The earlier variant-storage-contract work is committed separately as `09477b2`.
Do not mix its checked-assignment/overlap changes into this series or
infer general no-alias guarantees from the variant-only contract. Measurements
must identify whether they include that working-tree baseline.

Existing machinery to reuse:

- `src/semantic/ir/aggregate.rs`: ordered preparation, complete captures,
  construction, CASE bindings, arguments and returns.
- NIR `CopyBytes`: complete extent, evaluated addresses, overlap-safe transfer,
  and source/destination volatility.
- NIR storage identity, backing, activation, CFG, dominance, use/def, byte-region,
  effects and home-elision analyses.
- `src/backend/aggregate_abi.rs`: shared physical aggregate ABI expansion.
- Existing MIR6502 copy selection for transfers that remain necessary.

Scalar promotion is not an aggregate promotion engine: aggregate values are
addressable byte images and union members overlap. Add bounded aggregate
forwarding alongside it, reusing analyses rather than weakening scalar blockers.

## Distinguish initialization from replacement

SemIR must explicitly distinguish the following destination situations; neither
MIR nor the emitter should rediscover them from generated instruction patterns.

| Destination situation | Required behavior |
| --- | --- |
| Fresh private runtime value | Initialize its final home directly when no incomplete value or storage identity is observable. No old value needs preservation. |
| Existing value being replaced | Preserve destination evaluation order, old-value reads during RHS evaluation, overlap rules and failure-before-publication guarantees. Retain staging unless separately proven unnecessary. |
| Load-time/static initializer | Keep the existing data-image initialization rules. Runtime LET initialization is not a static initializer. |

A newly introduced LET binding and a newly allocated compiler-owned capture are
initial fresh-destination candidates. An inline payload field can inherit this
status from a fresh private enclosing capture, using canonical field extents.
Freshness is a fact about a particular initialization event and its visibility,
not a synonym for local, immutable, uninitialized, or first store in a block.
Do not infer it for globals, parameters, alias/absolute/volatile storage or
arbitrary pointer/index destinations merely because no earlier write was seen.

For `LET item=MaybeByte.SOME(42)`, the new `item` is not in scope in its own RHS,
has no exposed address and has no previous value to preserve. An outer binding
with the same spelling remains a different SymbolId. The two constructor bytes
can therefore be written directly to `item`, payload first and tag last.

Fresh destination does not mean copy-free source: `LET saved=original` still
needs a full snapshot when the source may subsequently change. Direct
initialization removes redundant staging, not the value-semantics boundary.

## Shared correctness contract

- Records retain their complete image, including inline arrays and padding.
- Unions retain all bytes, including bytes beyond smaller member views. Member
  FieldIds do not prove disjoint ranges or independent scalar values.
- Variants retain required validation, payload/tag consistency, gap zeroing,
  tag-last construction and failure-before-publication behavior.
- Preserve evaluation order and evaluate each address/argument once.
- A captured value must not become a reference to subsequently modified storage.
- Pointer fields copy pointer values; their pointees remain shared and mutable.
- Preserve volatile access counts and ordering; unknown effects remain barriers.
- Record/union transfers remain overlap-safe. Variant identical/disjoint typed
  transfers do not establish global no-alias facts.
- Distinguish routine-static Atari storage from invocation-local native storage.
  LET immutability and AggregateCapture purpose alone are not lifetime/escape
  proofs across callbacks, recursion or reentry.
- NIR transformations run only on verified input and produce verified output.
  Refresh analyses after storage/CFG rewrites. No source names or MIR-to-SemIR
  lookbacks may determine eligibility.

## Slice 1: baselines and reusable storage proofs

Extend the aggregate/LET audit coverage with corresponding record, union and
variant cases: fresh initialization, capture chains, mutation after capture,
CASE/guards, aggregate payload binders, direct/indirect calls, returns, nesting
and inline arrays.

Add a constructor-only fresh-LET probe as well as the existing `fresh_call`
corpus. Measure the LET instruction region independently of the subsequent CASE
capture/validation so slice 2 improvements are not confused with slices 3-4.
Pair positive probes with replacement assignments and required-snapshot controls.

Record logical CopyBytes sites and copied-byte sums, capture/home extents,
ABI-added transfers, physical storage, code/image sizes and VM cycles separately.
Static copy counts are not dynamic traffic; logical local bytes are not physical
zero-page use or peak native frame size.

Build bounded analysis for exact storage identity/range, nominal type/layout,
read/write sites and initialization candidates. Distinguish internal address
formation from escape. Reuse storage and memory-region facts, but never interpret
different unresolved regions/IDs as a universal disjointness proof. Unknown
pointer/index/effect cases must have explicit rejection reasons. Full CFG
initialization/value-lifetime proofs are added only as later slices require them.

Acceptance: positive and negative proof tests and reproducible baselines; no
generated-code or optimizer behavior change. Save current measurements, retaining
older ADT/union baseline documents as historical measurements.

Suggested commit: `nir: analyze bounded private aggregate storage regions`.

## Slice 2: initialize fresh private destinations directly

### Shared lowering interface

1. Split fresh aggregate initialization from replacement assignment in
   `src/semantic/ir.rs::lower_binding_statements` and the shared helpers in
   `src/semantic/ir/aggregate.rs`. Source assignment keeps the replacement path.
   Use an explicit internal destination policy or distinct checked entry points;
   do not decide from identifier spelling or an immutable flag alone.
2. Refactor aggregate preparation to accept an eligible fresh destination.
   Allocate a private capture only when the consumer has no suitable destination
   or the visibility/effect/ABI proof requires staging. This is shared lowering
   for all aggregate kinds, not a variant-only emitter optimization.
3. Preserve ordinary typed SemIR stores/copies/calls and verified NIR operations.
   No new executable LET/InitializeAggregate operation or weakened complete-value
   ABI verifier is required for the initial whole-home cases.

### Producer coverage

- **Variant constructor:** write payloads, required gaps and the final tag into
  the fresh final home. Reuse the existing constructor extent/zero-range logic.
  Do not allocate a destination-pointer relay or a second constructor home for
  a direct fresh binding.
- **Aggregate-returning call (record, union or variant):** use the fresh whole
  binding/capture as `aggregate_result` when its nominal extent and capture
  ownership satisfy the existing verifier. Keep ordered callee/argument
  preparation and required result validation. Hidden-result-slot forwarding
  inside the callee remains slice 5 work.
- **Existing aggregate value:** initialize the fresh home with the necessary
  complete value copy and required validation; do not replace it with a mutable
  source alias or add a second staging copy. Records/unions retain all raw and
  padding bytes, including the unused tail of smaller union views.
- **Nested constructor payload:** recursively use an eligible field of the fresh
  enclosing home. Initially retain nested call-result captures where the ABI
  verifier requires a complete standalone buffer rather than a subobject.

### Safety and fallback

The literal constructor example is the first bounded implementation target.
Then admit effectful producers only when the existing semantic/activation facts
or explicit proofs establish that no incomplete destination can be observed.
Preserve left-to-right, exactly-once evaluation, nested checks and fault timing;
do not infer purity from LET. A failure can leave an inaccessible fresh home
partially filled, but must not publish it or modify an existing observable value.

Initialization remains at the source execution point: it is skipped on untaken
paths and repeated on loop entries. Native automatic storage and Atari fixed
routine storage retain their existing lifetime/reentry semantics. Calls,
callbacks and reentry that defeat the fresh-private proof keep the staged path.
This does not authorize treating all Atari LET homes as invocation-private.

Keep replacement-assignment staging when RHS evaluation may read the old
destination, aliases/overlap are uncertain, or validation failure must preserve
the old value. Do not alter static declaration images, implicit variant storage
initialization, source syntax or required CASE validation in this slice.

### Tests and acceptance

- Add a source-level probe for `LET item=MaybeByte.SOME(42)` with a subsequent
  use so the binding is live. Assert direct payload/tag writes, no constructor
  capture, no destination-pointer temporary and no initialization CopyBytes.
- At the current absolute-addressed Atari audit layout, its MIR6502 LET region
  is 48 bytes / 70 cycles. The acceptance target is four instructions, 10 bytes /
  12 cycles, writing `[2,42]` with the tag last. Remove the extra two-byte
  constructor home and two-byte destination-pointer home. These numbers exclude
  CASE, its own capture, validation and printing; those remain until later slices.
- Exercise nullary/payload constructors, nested aggregates, inline arrays,
  native alignment gaps and union full-image tails. Poison eligible destination
  bytes to verify complete initialization rather than relying on zeroed storage.
- Cover fresh call results and genuine snapshots for all three aggregate kinds;
  preserve mutable-source snapshots, evaluation order and validation failures.
- Add shadowing, untaken-branch and repeated-loop initialization tests, plus
  conservative call/reentry and alias/volatile rejection tests.
- Check both classic and NIR/MIR6502 output. Shared semantic lowering must
  deliver the direct-initialization shape to both; native checks retain existing
  layout/ABI and missing-Error-adapter capability expectations.

Fresh initialization is independently useful before NIR copy coalescing.
Broader storage-dependent proofs remain in NIR and are extended only as
subsequent rewrites need them.

Suggested commit: `language: initialize fresh private aggregates in place`.

Implemented boundary: call/fault-free constructors over literals, integer
expressions and known ordinary sources; direct ordinary snapshots (validation
before publication); whole fresh call-result buffers under native automatic
activation. Constructor fields inherit freshness only when the entire producer
passes the bounded proof. Nested call results remain whole captures. No purity
is inferred from the default effects of user calls, and Atari calls are not
redirected without a separate nonreentry proof. Alias/absolute/volatile and
pointer/index sources retain their previous lowering. See the linked slice 2
note for tests, exact costs and the terminal-fault promotion correction surfaced
by the large-payload test.

## Slice 3: bounded whole-aggregate forwarding

Add a narrow NIR aggregate-forwarding component using slice 1 facts. Initially
handle straight-line exact-extent transfers, including
`temporary <- source; destination <- temporary`.

Distinguish producer redirection (write an eligible final private home) from
read forwarding (reuse backing that stays unchanged through every replaced
read). An immediate relay and a long-lived snapshot need different proofs.
Reject uncertain overlap, escapes, volatility, unknown calls and incompatible
lifetimes. Do not move observable writes earlier.

Extend existing home elision to remove unused aggregate homes and dead address
temporaries after successful rewrites, including all executable/effect/relocation
references. Preserve useful debug identity without keeping dead physical homes.

Acceptance: eligible chains lose copies and allocated homes; mutation, partial
overlap, effects and nominal-mismatch counterexamples retain correct snapshots.

Suggested commit: `nir: forward bounded private aggregate copies`.

Implemented boundary: complete same-nominal-type snapshots with all consumers
after their one initializing copy in the same block. Direct reads, exact fields,
constant indexed internal addresses and disjoint copy consumers are supported.
Read forwarding never redirects producers or changes the time of observable
writes. Calls/faults/unknown writes before the last read reject reuse; calls
after it do not. Whole ABI captures and cross-block CASE snapshots remain.
Producer redirection not already covered by slice 2 needs a distinct publication
proof; it is not implicitly authorized by this read-forwarding pass.

## Slice 4: control-flow and subobject forwarding

Use CFG/dominance/dataflow to identify an unchanged backing/value on every path.
Only retain matching facts at merges; model loop reinitialization explicitly.

Cover CASE reuse of a stable private value, repeated snapshots, aggregate pattern
binders and exact nested record/union ranges. A guard that mutates the original
selector must still see an independent captured value. Start subobject forwarding
with ordinary reads/copies; do not replace a complete ABI capture with a field
without extending the corresponding typed verifier contract.

Acceptance: the minimal MaybeByte example changes from
`construct temporary -> copy item -> copy CASE capture -> validate/read capture`
to `construct item -> validate/read item`: two fewer whole copies and two fewer
aggregate homes. Complete CASE constant folding is not required.

Suggested commit: `nir: reuse stable aggregate snapshots across control flow`.

## Slice 5: aggregate call boundaries

Apply the same proofs to reuse stable private argument captures, route results
to eligible fresh private bindings, and remove capture-to-capture return staging.
Keep physical ABI expansion accounted for: it introduces additional transfers
after logical NIR optimization.

Preserve indirect callee evaluation before arguments, left-to-right argument
snapshots, mutable callee parameter independence, separate input/output images
when needed and external calling conventions. Current logical NIR requires
complete compiler-owned argument/result/return captures; do not weaken that
requirement without a structured replacement and negative verifier tests.

True callee hidden-result-slot forwarding is a bounded follow-up only when the
same infrastructure proves publication and alias safety. Do not assume all
return captures or callee entry copies are removable.

Acceptance: physical copy counts improve, not just logical NIR formatting.

Suggested commit: `nir: eliminate redundant aggregate call captures`.

## Slice 6: cost acceptance and documentation

Run compiler tests, NIR snapshots and sweep, MIR6502 sweep and pinned VM tests.
Add dedicated coverage for source mutation, self/partial overlap in both
directions, invalid outer/nested tags, unchanged destinations on failure, guard
effects, loop entries, argument ordering, full union images, native padding,
native activation versus Atari fixed homes, page crossings and larger extents.
Check optimizer idempotence and malformed replacement rejection.

Measure classic and MIR6502 with cartridge/standalone runtimes; native checks
cover layout, ABI and lowering, not unimplemented native runtime execution.
Record wins/rejections and costs per stage. Commit each major slice separately
when requested; keep unrelated storage-contract changes separate.

Suggested commit: `tests: accept shared aggregate forwarding costs`.

## Non-goals and delivery order

No general field-by-field scalar replacement, aggregate memory constant
propagation, complete CASE folding, new copy-loop selection, whole-program alias
analysis or ABI redesign. The first useful optimization milestone is slices
1-3, followed by CASE/subobject and call-boundary extensions. With the initial
slice 1 proof/baseline foundation in place, prioritize slice 2 fresh initialization
as the first code-generation improvement; do not delay its structurally proven
cases until general NIR pointer-relay or cross-block lifetime analysis is complete.

## Required commands

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test adt_codegen_audit -- --nocapture
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test unions_codegen_audit -- --nocapture
```

New focused proof/baseline tests must also be run. Full VM regression validation
is required when executable lowering/optimization behavior changes.
