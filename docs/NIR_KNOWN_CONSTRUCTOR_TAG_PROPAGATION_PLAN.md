# Known constructor tags through verified NIR

Status: slices 0–3 implemented; final acceptance pending.
Baseline: `42d4f94`, inspected and measured on 2026-09-09.

## Objective

Fold CASE dispatch when an earlier executable store proves the captured
selector's tag. Implement this as bounded constant propagation for exact U8
subregions of private aggregate storage. Constructor names, tag field names,
source syntax and variant validity are not optimizer inputs.

For `LET item=MaybeByte.SOME(42)`, current optimized NIR still contains:

```text
store item.+1 = 42
store item.+0 = 2
%tag = load item.+0
%none = cmp %tag Eq 1
branch %none ? none_arm : next_arm
```

The intended result replaces `%tag` with the typed integer constant `2`.
Existing value folding and CFG cleanup then select the SOME arm and remove
unreachable alternatives and the unreachable invalid-tag fault. The same byte
proof may replace the payload load with `42`; no constructor-specific rule is
needed. Construction stores remain unless an existing independent pass proves
them dead. This work does not promise complete aggregate home elimination.

An unknown tag retains ordinary dispatch and validation. A known invalid byte
must select the existing fault path, not be treated as a valid constructor.

## Current foundations and measured gap

| Component | Reuse and limitation |
| --- | --- |
| `src/semantic/ir/aggregate.rs` | Owns constructors, tag-last publication, validation, selector capture and ordered guards. Its current lowering already supplies explicit typed stores and field offsets. |
| `src/nir.rs` | Runs value cleanup, aggregate forwarding, scalar storage propagation, scalar promotion, home elision and final value cleanup, in that order. |
| `src/nir/storage_optimizer.rs` | Caches whole scalar homes by `NirStorageId`; field accesses and aggregate storage do not establish the required field constants. Do not relax its scalar eligibility rules to admit aggregates. |
| `src/nir/analysis/aggregate_regions.rs` | Resolves bounded ordinary storage regions and address uses, rejects uncertain backing, and checks byte overlap. Exact regions alone do not prove initialization or privacy. |
| `src/nir/analysis/aggregate_lifetime.rs` | Demonstrates must-availability across CFG edges for one captured image. Its boolean snapshot fact is not a scalar constant lattice. |
| `src/nir/optimizer.rs` | Already folds integer comparisons, propagates constants through temps/edges, simplifies branches and removes unreachable blocks. |
| `src/nir/aggregate_forwarding.rs` | Can remove a stable CASE snapshot before this pass. Copies that protect a value across mutation must remain independent. |

Reproduced `tests/support/fresh_maybe_byte.act` at origin `$3000`:

| Backend | Cartridge XEX bytes | Standalone XEX bytes | CPU cycles to PrintBE |
| --- | ---: | ---: | ---: |
| Modern classic | 148 | 418 | 163 |
| Optimized MIR6502 | 80 | 363 | 34 |

These are current measurements, not projected post-change costs. Execution
stops at PrintBE entry with A=42; printing itself is excluded. The existing
construction-only measurement is separate: 10 bytes / 12 cycles.

Reproduction commands:

```sh
cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-optimized-nir \
  tests/support/fresh_maybe_byte.act
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked \
  --test aggregate_fresh_initialization \
  maybe_byte_example_reports_cost_to_printbe_without_running_printing -- --nocapture
```

Same-block forwarding (slice 1) measures 44 cartridge / 280 standalone XEX
bytes and 23 cycles to PrintBE. Classic remains 148 / 418 bytes and 163 cycles.
The optimized fixture intentionally loses the tag load, both comparisons and
the unreachable NONE/fault arms. Its payload load crosses a block boundary and
remains for the CFG slice. Raw lowering and construction stores are unchanged.

CFG propagation (slice 2) also removes the payload load. The measured result
is 45 cartridge / 281 standalone XEX bytes and 23 cycles. Immediate payload
materialization adds one image byte relative to slice 1; both images remain
well below the original baseline. This target lowering tradeoff does not
change the NIR byte proof or justify a MIR peephole in this series.

## Ownership and bounded scope

SemIR continues to own Action! meaning. NIR owns facts about bytes stored at
resolved places and substitution of compatible typed values. MIR6502 and the
other target consumers receive ordinary verified NIR; emission owns final
bytes, labels and runtime linking.

Add a focused NIR component, provisionally
`src/nir/subregion_constants.rs`, using the existing analyses. No executable
`Variant`, `Constructor`, `KnownTag` or `AssumeValid` form is required. Do not
parse `__variant_tag`, inspect semantic field IDs, hard-code tag offset zero,
or consult SemIR from an optimizer/backend.

The initial eligible roots are ordinary local homes with
`NirLocalPurpose::AggregateCapture`, including nested fixed fields. Require:

- An exact, nonempty, in-bounds region in ordinary non-aliased storage.
- `address_use` classified as `NotFormed` or `InternalOnly`, no data-image or
  relocation exposure, and no machine visibility.
- A nonvolatile U8 load/store with a compatible typed constant. Record, union
  and variant captures use the same rule; overlapping union views use offsets.
- A fact established by executable operations in the current routine flow.
  Neither LET immutability nor the capture marker supplies an initial value.

Routine-static Atari homes are eligible between proven writes and barriers;
they are not assumed invocation-private. Native automatic storage uses its
existing identity domain. Calls kill these new facts on every target in this
series, including calls marked pure or calls on an apparently private home.
Narrower call handling needs separate evidence and is deferred.

Exclude ordinary user globals/parameters, static-image seeding, absolute or
aliased roots, dynamic indexes, mutable pointer relays, pointer/REAL values,
and wider integer constant reconstruction. Constant indexes and dominating
SSA `AddrOf` origins may be admitted only through the existing exact-region
proof; no new pointer-flow or alias analysis is part of this plan.

The same rule can propagate an ordinary record/union U8 field. That is useful
shared behavior, not permission to promote complete aggregate images to SSA.

## Fact and transfer contract

Facts are routine-local and keyed by stable storage identity plus byte offset
and scalar type. The first supported width is exactly one byte. Preserve the
root identity domain in eligibility checks; never compare IDs from different
routines as though they denoted a common object.

Use a finite candidate census of queried U8 regions. Copy propagation may
extend it through exact source/destination mappings, but must not expand every
byte of a large aggregate merely because it has a large extent. Bound any
closure/work budget explicitly; exhaustion retains loads and is not a source
unsupported diagnostic.

At routine entry no byte is known. Distinguish unreachable/unprocessed state
from a reachable state with no known values. A known value survives a CFG join
only when every incoming reachable path supplies the same typed constant.
Initially consider all CFG edges possible; existing constant branch cleanup
may expose further facts in a subsequent bounded round.

| Executable operation | Effect on the new facts |
| --- | --- |
| Exact eligible U8 store of an integer constant | Kill overlapping facts, then establish that byte's normalized U8 value. |
| Other scalar store with a proven exact destination | Kill every overlapping tracked byte using the full store width; preserve proven disjoint cells. Do not derive byte values from wider stores. |
| Store with unknown address/extent or unresolved alias | Clear all facts. Different unresolved IDs are not a disjointness proof. |
| Eligible U8 load with a known cell | Substitute a typed constant at all uses and remove the load; do not create longer-lived nonconstant temps. |
| Ordinary proven nonvolatile read | Establish no fact from an unknown value. Preserve facts only where the read has no opaque/hardware boundary. |
| Absolute/unknown-pointer reads, volatile loads/stores, volatile copies | Clear facts; retain each read/write and its ordering. |
| Calls, machine blocks, REAL operations, unsupported effects, potentially faulting arithmetic | Clear facts and retain the operation. No special Error or pure-call exception. |
| Pure arithmetic, casts, compares and address formation | Preserve memory facts; rely on existing constant folding for their results. Address escape still disqualifies the root. |
| Nonvolatile `CopyBytes` before copy propagation is enabled | Invalidate the destination if exact and the source read has no unknown/absolute boundary; otherwise clear facts. Do not invent copied constants. |
| Proven exact, nonvolatile `CopyBytes` after slice 3 | Transfer known source cells from the input state to corresponding destination offsets, after invalidating overwritten destination cells. |

The invalidation code must account for every `NirOp` variant exhaustively.
Reuse the existing region/overlap checks and fault classification. Do not
change the aggregate snapshot proof's behavior merely to accommodate this pass.

Substitution is a statement about the value captured by a particular load.
Once replaced, that SSA value remains constant even if a later guard call
mutates its original storage. A later load from that storage requires a new
proof. Clearing memory facts must not revoke an already captured SSA value or
turn a captured selector into a fresh memory read.

## Delivery slices

### Slice 0: proof census and baseline

Add focused coverage in `tests/nir_subregion_constants.rs` and prepare
`fixtures/nir/known_constructor_tags.act` with raw and optimized snapshots.
Register new fixtures through the existing fixture inventory machinery.

Keep the first increment read-only: classify candidate cells, byte ranges,
initializing stores and blockers. Start with the fresh MaybeByte example and
record/union equivalents whose field names and offsets differ. Capture the
current NIR, emitted sizes and VM costs before enabling transformations.

Acceptance: deterministic proofs; exact overlaps and negative address-use
cases covered; baseline fixture still contains the redundant tag load and
dispatch. This slice changes no compiler behavior.

### Slice 1: same-block U8 forwarding

Implement explicit-store-to-load constant propagation for eligible cells in
one block. Consume immutable analysis for a fixed program generation, collect
substitutions, rewrite all value uses including terminators/edge arguments,
rebuild temp facts, and verify the output.

Place the new propagation/value-cleanup group after aggregate forwarding and
before scalar storage propagation. Run ordinary value/CFG cleanup after a
successful rewrite. Stabilize this small group when cleanup exposes another
eligible load; each transforming round must remove loads or simplify CFG.
Do not repeatedly run scalar promotion or physical ABI expansion as a shortcut.

Acceptance: the fresh MaybeByte program has no remaining selector load,
constructor comparisons, NONE arm or reachable InvalidVariantTag path in Main.
It still reaches PrintBE exactly once with 42. Disjoint payload stores preserve
the tag fact; overlapping stores and each barrier category invalidate it.
Private record/union U8 fields obey the same behavior without name-based rules.

### Slice 2: CFG joins, loops and captured selectors

Extend the fact map using `NirDataflowProblem`/`solve_dataflow`. Intersect equal
constants at joins; a missing/conflicting incoming fact is unknown. Solve the
immutable CFG before rewriting loads. Rebuild CFG, dominance, use/def and
region analyses after structural cleanup.

Exercise agreeing and conflicting diamonds, skipped initialization, zero-trip
loops, loop-carried mutation, continue/backedges, and repeated LET execution.
A backedge must not bootstrap a fact before the first initialization.

Acceptance: identical reaching stores fold a later dispatch; differing or
missing stores retain the necessary load/branch. Guard calls preserve the
already captured selector while invalidating future memory loads. Payload
patterns and ordered guard effects remain in source order.

### Slice 3: facts through retained snapshots and nested fields

For exact nonvolatile copies, propagate only cells contained in both transfer
extents. Read all source facts from the pre-copy state before killing the
destination range. Support proven disjoint source/destination regions and
exact self-copy; retain a conservative fallback for partial overlap, unknown
addresses, unresolved aliases and volatile access.

Declining copy propagation still invalidates every possibly overwritten cell.
Unknown/absolute source reads are barriers even when the destination is exact.

A destination snapshot owns the copied fact after the copy. Later source
mutation invalidates source facts, not the independent destination. Keep the
actual `CopyBytes`, payload image and ABI capture unless the existing aggregate
forwarding pass independently proves that transfer redundant. Do not redirect
producers or reinterpret aggregate call results in this slice.

Use fixed nested field offsets and exact-region queries to handle inline
variants inside records/arrays. An outer tag never implies that a nested tag
is valid. Each removed nested check needs its own explicit byte-value proof.

Acceptance: retained captures and copy chains can carry known tags; snapshot
mutation controls still execute the original arm. Unknown nested tags retain
early deep validation, even for ignored payloads. Invalid nested tags fault
before guard effects or destination publication.

### Slice 4: integration, costs and contract acceptance

Finalize the bounded pass scheduling with public optimizer idempotence tests:
`optimize_program(optimize_program(p)) == optimize_program(p)` for the corpus.
Specifically check cases exposed by aggregate forwarding, scalar cleanup and
CFG simplification. If ordering needs adjustment, keep it confined to the
dependent constant/CFG cleanup group and measure compile-time cost before
enabling it. Do not introduce an unbounded whole-pipeline fixed point.

Add `tools/vm-runtime-tests/tests/known_constructor_tags.rs`. Test the same
sources in modern classic, MIR6502 with raw NIR, and optimized NIR/MIR6502,
under cartridge and standalone linking. Use bounded completion, whole guarded
memory windows, effect counters and volatile access observations where needed.
Retain terminal-stop coverage when an Error test handler returns.

For the minimal example, require fewer than 34 cycles to PrintBE and smaller
than 80/363-byte cartridge/standalone images, then pin measured post-change
limits. Do not estimate or claim complete zero-cost construction in advance.
Report constructor stores, remaining loads/branches, CopyBytes counts, emitted
code versus total XEX size, and execution cycles separately. Classic is the
semantic reference and should retain its measured behavior/costs because this
work is NIR-only.

Update `NIR_TARGET_SHAPE.md` and the CASE validation note with the implemented
byte-fact guarantees and remaining barriers. Label changed optimized snapshots
as an intentional optimization; raw lowering snapshots should remain unchanged
apart from newly added cases. Keep historical aggregate cost CSVs intact.

## Required regression matrix

| Area | Required cases |
| --- | --- |
| Explicit byte facts | Fresh nullary/payload constructors, U8 values 0/1/255, unrelated field names, nonzero nested offsets, repeated identical stores. |
| Byte overlap | Disjoint payload updates, tag overwrite, wider overlapping store, union views with identical/partially overlapping extents, adjacent cells. |
| Initialization and CFG | Uninitialized entry, skipped definition, agreeing/conflicting joins, unreachable predecessor cleanup, zero-trip and repeated loops. |
| Captures and guards | Source mutation after snapshot, captured selector reused after a mutating guard, false guards reaching later arms, payload/binder initialization order. |
| Invalidity and fault order | Unknown and known-invalid outer tags (including handcrafted verified NIR stores), known outer/unknown nested tag, inactive nested alternatives, divide-by-zero before/after initialization, Error returning. |
| Barriers and identity | Direct/indirect/external calls, recursive/callback reentry into Atari static homes, opaque ASM, absolute/hardware reads and writes, pointer escape, data relocation exposure, volatile access count/order. |
| Copies | Disjoint exact transfer, self-copy, retained snapshot chain, unknown/partial-overlap fallback, source mutation after transfer, large extents without bytewise fact explosion. |
| Types and targets | Record/union/variant U8 cells on Atari 6502, 65816 small/native and 68000 layouts; no endian-dependent widening or pointer/REAL reinterpretation. |
| Verifier and stability | Malformed input rejected before analysis; exact replacement types, valid temp/edge uses, no executable legacy forms, deterministic output and public optimizer idempotence. |

Four-target verification is a layout/IR gate, not a claim of native execution.
The current native Error-adapter capability limit remains explicit. No target
must reconstruct source variants or field names to consume the result.

## Checks and completion criteria

After every transforming slice, run the required repository checks:

```sh
cargo test --test nir_subregion_constants
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

For executable/cost acceptance, also run:

```sh
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo check --all-targets
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked \
  --test known_constructor_tags --test aggregate_fresh_initialization \
  --test variant_case_validation --test case_guards --test nested_patterns
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked
git diff --check
```

Completion requires the fresh known-constructor case to fold through verified
NIR, safe propagation through the planned CFG/copy cases, all negative controls
to retain required effects/faults, measurable Atari gains, unchanged language
and ABI contracts, and passing existing fixtures and full suites.

Deferred work: general globals/parameter propagation, call-return constructor
summaries, tag-range/set analysis, edge-derived memory assumptions, wider scalar
packing/unpacking, dynamic pointer analysis, constructor producer coalescing,
hidden-result-slot forwarding and broader dead aggregate store/home removal.

The first implementation commit should deliver slice 0. The first transforming
commit should deliver slice 1 and its measured MaybeByte improvement before
adding CFG or copy generalization.
