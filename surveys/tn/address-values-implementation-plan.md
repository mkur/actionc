# MIR6502 address values through locals and returns

Status: Stage 0 complete, 2026-10-12; Stages 1–5 pending. The
[qualification notes](address-values/README.md) retain measurements and checks.

This plan implements priority 1 from the
[current TN/TNDBG output audit](output-audit-2026-10-12/README.md). It continues
the completed [record-consumer work](size-reduction-implementation-plan.md)
without reopening the application layout or provider design.

## Objective

Generate indexed record addresses as efficiently when they are returned or
stored in a pointer local as when they feed a direct field load/store. Preserve
known storage addresses through proven local-pointer flows so their fields can
use absolute addressing. Retain every required source assignment, memory
snapshot and public ABI write.

TN/TNDBG are the application measurements; generic fixtures define the compiler
behavior. Matching must never use TN routine names, field names, record sizes
from source text, or the application's cache capacity.

## Evidence and limits

The audit pins source revision `02d79b01` and compiler source `1d67c984`.
Cartridge load-file sizes are 18,095/21,586 bytes for classic TN/TNDBG and
16,807/20,544 bytes for MIR6502. Preserve these measurements; establish a new
baseline if compiler inputs change before implementation.

| Generic MIR cartridge form | Routine bytes | Emitted spill/local bytes | Multiply helper |
| --- | ---: | ---: | --- |
| Returned indexed address, `Address` | 76 | 4 / 0 | yes |
| Local pointer then field read, `ReadViaLocal` | 104 | 2 / 2 | yes |
| Direct indexed field read, `ReadDirect` | 85 | 0 / 0 | no |
| Address of element zero, `AddressZero` | 63 | 4 / 0 | yes |
| Address of the embedded field, `AddressField` | 18 | 0 / 0 | no |

The first three forms use a 20-byte stride and 38-byte embedded displacement
in [escaping-record-address.act](reproducers/escaping-record-address.act).
The direct/local difference is evidence of a strategy gap, not permission to
delete the local assignment or a guaranteed 23-byte image saving.

During the 64-entry reversed MIR sort, `BatchItem` executes 964 times and
accounts for 13,496 of 15,112 measured spill operands. Its body plus the general
multiply helper consumes 399,840 of 805,176 cycles. This establishes priority,
not an attainable percentage improvement.

A separate current-compiler inspection confirms that a fixed record's direct
field access uses absolute instructions. Assigning that same address to a
source pointer local and reading its field still uses pointer preparation and
indirect-Y access. A fixed pointer-cell location or constant initializer does
not establish an immutable pointee address.

## Scope and ownership

The work belongs in MIR6502. Verified NIR already supplies typed arithmetic,
storage identity, byte offsets, indexes, loads/stores and effects. SemIR and
NIR remain unchanged unless a specific missing contract is demonstrated. Any
such shared extension must be a separate slice with the required shared checks.

Build on these existing components:

- [address-use analysis](../../src/mir6502/analysis/address_uses.rs) and
  [rewrite contexts](../../src/mir6502/rewrite/context.rs): captured values,
  mutable pointer cells, complete word flow, availability and dead flags;
- [address rewrites](../../src/mir6502/rewrite/addresses.rs): transactional
  displacement and exclusive record-consumer selection;
- [indexed materialization](../../src/mir6502/materialize/indexes.rs) and
  [storage-address materialization](../../src/mir6502/materialize/lea.rs):
  target construction and late layout resolution;
- [pseudo-machine contract](../../docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md) and
  [rewrite workflow](../../docs/MIR6502_REWRITE_WORKFLOW_PLAN.md): phase
  guarantees, analysis invalidation and physical interference.

First support same-block flows. Cross-block local propagation, joins, loops,
interprocedural specialization, inlining, a new allocator, general multiplication
expansion, aggregate copying and the audit's arithmetic/call-predicate priorities
are follow-up work. Classic is a comparison backend, not an implementation
target. TN production sources and directory contracts stay unchanged.

## Intended representation and selection

Introduce one typed, pure, word-valued MIR operation for a computed address
(working name `ComputeAddress`). Its logical shape is:

```text
word destination = captured base + constant displacement
                   + captured index * constant stride   (modulo 65536)
```

The index is optional; displacement and stride retain their full `u16` values.
Known storage bases retain typed storage identity and relocatable offsets until
final layout. Mutable pointer cells require an explicit load at the original
evaluation site; they cannot masquerade as captured bases.

The operation defines a complete word `MirDef`. It does not write source
storage, choose a fixed zero-page pair, or encode a return ABI. Those remain
separate stores and materialization decisions. Reuse existing value/storage
forms and the address-use analysis rather than introducing another expression
language or independent data-flow framework.

Selection has three cases:

1. A known storage address plus constant displacement can feed a direct field
   access, or materialize its low/high address bytes for a value consumer.
2. A dynamic indexed address can use existing full-address construction and
   feed a complete word store/result without first spilling multiplication and
   addition intermediates.
3. A pointer local written from that value can supply a subsequent field
   consumer when reaching-store and availability proofs establish the exact
   captured value. Keep the local store at its original observable point.

`PagedIndirectIndexedY` and `ScaledIndirectIndexedY` can split an effective
address between a pointer pair and Y. Their pointer bytes alone are not a
returned address. Initial value construction uses a complete 16-bit pair;
any later split representation must explicitly reconstruct the full value.

## Invariants for every slice

- Evaluate each base and index once, in original order. Retain originating
  loads and calls, including volatile reads and their multiplicity.
- Preserve 16-bit wrap, the full CARD index, low-to-high carry, and complete
  byte/word field accesses. Declared array capacity is not an index bound.
- Preserve source pointer writes, canonical parameter captures, and public
  CARD results in `$A0/$A1`. A/X is not a replacement return ABI.
- Treat unknown writes, calls, machine blocks and uncertain aliases as
  barriers. A source local is memory, not an immutable SSA value; one-lane
  overwrites invalidate a word fact.
- Prove dominance, reaching definitions and whole-routine deadness before
  removing private producers. A later high-byte or successor use blocks
  removal. Multiple uses may retain the computed value and its home.
- Keep captured values available across retained stores. Final physical
  ranges must exclude scratch/source/result collisions; different symbolic
  storage IDs alone do not prove nonaliasing.
- Preserve live machine flags and register inputs. Effects, use/def, liveness,
  printers, temp remapping, visitors and phase verification must understand
  the new operation before a selector emits it.
- Retain an executable general fallback for unsupported or unproved cases.
  No unsupported stride may be silently narrowed to the materializer's `u8`.

## Delivery sequence

Deliver one reviewed commit per stage; split a stage further if its proof and
selector changes cannot be reviewed together. Stage status and measurements
are updated only after its acceptance checks pass.

### Stage 0: extend and freeze the generic acceptance baseline

Reuse the audit's image hashes, static census, CPU vectors and generic VM
target; do not repeat or replace the completed audit. Retain a maintained
known-address fixture comparing direct fixed/compiler-placed records with
`p=@record; read/write p.field`, plus a returned constant field address.

Extend address execution coverage only where missing: complete source-local
pointer contents, returned `$A0/$A1` bytes, effectful indexes, aliasing pointer
cells, byte/word field stores, and rejected-flow cases. Keep known failures out
of normal CI until fixed. Record pre-home and materialized shapes, code/homes,
helper calls and scoped cycles for each motivating form.

Acceptance: existing generic cases pass, new semantic checks pass on the current
compiler, and remaining code-quality gaps are recorded as observations. The
four full-root baselines remain unchanged. No compiler change is made.

### Stage 1: typed address-value facts and conservative lowering

Extend the shared address analysis to describe a value-producing address chain
separately from a memory consumer. Recognize typed storage addresses, captured
bases, constant additions, word copies/extensions and constant-stride scaling.
Fold a zero index only after proving its complete value; preserve the base
evaluation even when scaling disappears.

Add the word operation and all analysis, printing, remapping and phase-contract
support. Provide conservative lowering through existing word arithmetic for
every represented form. Add context queries for complete address provenance
and exact same-block pointer-store/load forwarding, with explicit rejection
reasons. Physical assumptions remain deferred to post-home confirmation.

Do not enable profitable selection yet. Register new producer-removing families
in the maintained rewrite inventory when introduced; use transactional plans
and invalidate the appropriate analyses after application.

Acceptance: positive/negative fact tests and phase verification pass, fallback
semantics execute correctly, and production images are byte-identical to Stage
0. The pseudo-machine contract documents ownership and rejected legacy forms.

### Stage 2: constant addresses and known-storage pointer locals

Enable same-block forwarding from an explicit complete local-pointer store to
its field consumer when the reaching store, input availability and physical
ranges are proven. Keep the source assignment and every required memory read;
only avoid redundant address construction or a nonvolatile pointer-cell reload.

Select direct absolute/zero-page field access for a typed known storage base.
Resolve compiler-placed and deferred storage at the correct layout boundary;
descriptor storage continues to load its pointer rather than using the
descriptor cell's address. Simplify a proven zero-index address to its base
plus displacement, including a returned address through a dynamic record base.

Acceptance: known-address local reads/writes retain the assignment and use
direct field operands where admitted. `AddressZero` has no multiply helper and
is no larger than the equivalent `AddressField` form after required captures.
Mutation, volatile/fixed pointer cells, descriptor bases, physical collisions
and conflicting predecessor facts retain the general path. Baseline direct
record access does not regress.

### Stage 3: indexed address values for stores and returns

Select captured constant-stride multiply/add chains into the typed word-address
operation. First support the existing materializer's complete-address strategies
for strides 1–255, including BYTE and unrestricted CARD indexes. Wider strides
retain existing full-width arithmetic unless independently proven and costed.

Materialize an exclusive address value at its word consumer, including the
canonical return store or a source pointer store. Combine construction and
that store without changing its semantic position or losing either lane.
Remove only proven dead private arithmetic/copy producers. If a value has other
uses, retain its complete word definition/home rather than duplicating work or
claiming exclusive flow.

Reuse existing scratch-pair and scale machinery with explicit carrier lifetime
and clobber facts. Validate base/index source overlap, a high base byte that must
survive low-byte computation, and scratch/result overlap before admission.

Acceptance: admitted generic returned-address and pointer-store cases avoid the
general multiply helper; returned pointers are correct at boundary indexes and
wrap. `BatchItem` uses a measured profitable strategy, retains its ABI, and
shows fewer executed spill operands/cycles without requiring application edits.
Report routine code, homes and full-image changes separately. Cost rejection
remains a valid outcome for individual strides/forms.

### Stage 4: dynamic addresses across retained local stores

Use the exact local-store provenance from Stage 1 to connect a subsequent field
load/store to its captured address value. First admit one same-block source
pointer store followed by a compatible field consumer, with no intervening
barrier or pointer mutation.

Keep the pointer assignment. Let address-value materialization and the existing
post-home base-availability analysis reuse a still-valid prepared pair, or the
complete retained value, at the field consumer. Do not recompute the base/index
from mutable input cells after the local store. Producer deletion must account
for both the retained store and every additional consumer.

Acceptance: `ReadViaLocal` retains correct local bytes and field behavior, uses
no general multiply helper for the admitted stride-20 case, and improves its
104-byte/code-home baseline under the size policy below. Include source field
writes, intervening aliasing writes/calls, pointer mutation between consumers,
multiple users, live flags, high-byte-only uses and successor uses. Rejected
cases remain correct through the original path.

### Stage 5: qualify, cost and document the integrated result

Measure all accepted forms after final materialization, including parameter
capture, scratch preparation, word-result copies, shared homes and any retained
helper/runtime bytes. A smaller virtual operation count is not a size result.

Keep new selection initially confined to optimized MIR6502. The default size-oriented
policy admits neutral or smaller final code-plus-home cost; uncertainty retains
the general path. Faster variants that grow code remain experimental until a
separate size budget is explicitly documented. Compare whole-root images so
branch relaxation and shared storage are included; do not add independent wins.

Require a net improvement in the targeted TN sort address path and no TN/TNDBG
load-size regression for the enabled configuration. If the byte-scale expansion
cannot meet that requirement, retain the fallback and record the unresolved
cost decision instead of silently introducing source workarounds, stride-specific
runtime helpers or a broad multiplication policy.

Acceptance: full affected-backend qualification passes; the four-root census
and CPU profiles are reproducible; classic root images remain unchanged;
provider acceptance, source layouts and public ABI stay intact. Keep the
`BatchItem` helper shared. Compare direct versus local/returned forms on at
least two non-TN record layouts. Document both accepted and rejected strategies,
including any remaining cases that still need a helper/home.

## Validation scope

| Behavior | Required cases |
| --- | --- |
| Record construction | Strides 1, 2, 20, 255 and fallback 256; two independent layouts |
| Field layout | Offsets 0, 38, 254, 255, 256; byte/word reads and writes |
| Indexes | BYTE 0/1/127/255; CARD 0/1/255/256/257/65,535; effectful index once |
| Address boundaries | Aligned/unaligned bases, page crossing, `$FFFF` wrap; raw-memory tests where code/ROM prevents a VM mapping |
| Value flow | Direct, returned, retained local store, extra uses, lane uses, successor uses and redefinitions |
| Memory identity | Captured base versus cell, pointer-cell mutation/aliasing, pointer referring to its own cell, known direct/deferred/descriptor storage |
| Machine state | Live flags/registers, source/scratch/result collisions, nested calls and machine blocks |
| Runtime/platform | Conservative and optimized MIR materialization, cartridge/standalone ABI, actual LF/CRLF compilation |

During stages, use filtered MIR6502 library tests and affected integrations:
`mir6502_materialization_gap`, `mir6502_rewrite_inventory`,
`mir6502_fixtures` and `mir6502_call_abi`, plus the generic address VM target and
`record_consumer_boundaries`/`embedded_record_arrays` when their paths change.
Run `tn_size_profile` with per-routine output to retain independent spill/cycle
reconciliation. Exact cycle numbers are observations, not correctness assertions.

Final qualification runs the full MIR6502 backend suite and its runtime/ABI
matrix, plus `tn_directory_model`, `tn_mydos_directory`, `tn_dispatch` and
full-root LF/CRLF compilation. The existing classic comparison cases run where
already present in those affected targets; unrelated 65816/68k suites are not
required for MIR6502-only changes. If shared compiler contracts change, run the
repository's required NIR snapshots, sweep and full `cargo test` as well.
Explain changed MIR snapshots as intentional target-IR contract changes;
the new operation does not by itself change the NIR contract.

For documentation-only preparation of this plan, check content and local links;
no compiler tests are required. Retain small maintained measurements and generic
fixtures. Put binaries/listings under an isolated ignored target directory,
avoid multiple full build caches, and preserve the frozen Stage 0/7 baselines
during cleanup.
