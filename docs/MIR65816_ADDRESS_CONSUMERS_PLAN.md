# MIR65816 address values and indirect consumers

Status: slice 1 implemented; slices 2–4 pending. Compiler baseline: `979205e4`.
This change belongs in actionc; Exec routines remain ordinary source code.

## Problem and measured baseline

`EXECLISTS.NewList` currently emits 133 bytes and reserves a 12-byte frame.
The three stores initialize a header with `chain + 3`, null and `chain`.
The emitted code nevertheless captures the computed addresses, reloads them
for stores and stages the same incoming pointer into DP three times.

The existing generated Exec workload has 960 routines and 485,991 routine-code
bytes. Other useful baselines are AddHead (170 bytes, frame 16), AddTail
(182/16), Insert (369/16) and Remove (101/0). These are compile-only measurements
from the local compiler override recorded in the
[forwarding-wrapper evidence](benchmarks/65816-forwarding-wrappers/README.md),
not measurements of the pinned play image.

There are four separate causes:

1. [Pointer forwarding](../src/mir65816/emit/pointer_forwarding.rs) can omit a
   capture, but runs after allocation and retains its reserved home. Its read
   resolver still requires that home to exist. Representation-preserving casts
   are not included in these borrowed bindings.
2. [Pointer address selection](../src/mir65816/emit/pointer_values.rs) requires
   a complete destination home. Even a zero-offset address is copied there;
   a nonzero offset is computed there before its consumer is considered.
   Pointer cast coalescing can eliminate a copy without shrinking the frame.
3. [Storage demand](../src/mir65816/emit/home_demand.rs) already selects scalar
   expressions with their consumers, but indirect stores and pointer-valued
   address expressions are outside its current admission rules.
4. `prepare_address` in [select.rs](../src/mir65816/emit/select.rs) stages an
   ordinary stack-backed base for every access. The existing DP pointer-leaf
   allocator avoids this for a closed load/store subset, but excludes
   AddressOf, casts and constant store operands such as NULL.

The general solution is to plan value storage and address use together before
allocation, then retain checked address facts during instruction selection.
No instruction peephole, source-name recognition or list-specific rule is needed.

## Shared design

Extend the existing home-demand plan with explicit decisions for:

- **Owned storage:** a normal stack/DP home remains necessary.
- **Borrowed value:** reads use an authoritative parameter/local home, with a
  checked lifetime and source identity. This is never a writable temp home.
- **Value alias:** a representation-preserving cast or zero-offset address
  refers to an existing value. Its logical identity and uses remain tracked.
- **Consumer expression:** a pure address computation is selected with its
  complete consumer, including a register/scratch schedule and clobber proof.

Use typed TempId/ParamId/frame-object identities, definitions and operand roles.
Separate a pointer's value from the contents of its pointee. A pointer loaded
from mutable memory is a snapshot; its address does not authorize reloading
that memory later or reusing its contents after a store.

Keep one authoritative demand decision for allocation, selection and independent
verification. Reuse the existing pointer-forwarding classifiers; migrate their
ownership instead of introducing a second competing capture-elision pass.
Resolve physical stack offsets only after final frame layout. Preflight complete
consumers and fall back atomically before suppressing any required capture.
Do not discover a missing home halfway through emission.

Start within a basic block. Count all routine-wide operand occurrences,
including address bases, stored values, indices, calls, returns and edges.
An alias may have several uses if every use is supported; a computed expression
initially requires one terminal consumer. Follow def-use relationships through
pure address/cast nodes, rather than matching an exact sequence of source lines.
Do not move observable reads or writes, or extend bindings through calls/joins.

## Slice 1 — Plan aliases and borrowed homes before allocation

Suggested commit: `65816: account for borrowed pointer values before allocation`.

Move the existing immutable incoming-pointer and non-escaping local-pointer
proofs into the demand planning stage. Preserve their current barrier rules and
terminal-consumer preflight. Admit same-representation pointer casts and
AddressOf with an indirect base, zero displacement and no index as value aliases.
Do not treat the address of pointer storage as the pointer stored there.

Omit homes only when the entire borrowed/alias use chain is admitted. Retain a
real capture for unsupported uses, source modification, partial/volatile loads,
escaping locals or incompatible casts. A source used in both address and payload
roles needs both roles validated, not two independent optimistic decisions.

Update allocation, value resolution, cast coalescing, staging and frame-map
verification together. Resolve aliases before looking up an owned home;
recompute frame extent and incoming displacements after omitted homes disappear.
Keep existing DP pointer/scalar allocation profiles authoritative until their
interaction is explicitly supported.

Acceptance: ordinary pointer assignments and record-field stores through
identity casts lose unnecessary captures and their reservations in raw and
optimized NIR. Malformed sparse maps and stale source bindings are rejected.

## Slice 2 — Select address expressions into their consumers

Suggested commit: `65816: select address expressions into indirect stores`.

Add a checked indirect-store consumer to the existing expression machinery.
Start with nonvolatile scalar stores at a captured 24-bit base plus a constant
displacement. Support existing BYTE/CARD register expressions, pointer identity
values and pointer AddressOf/constant-offset computations. Keep dynamic scaled
indices and arbitrary wide expression trees on the existing path initially.

Plan both sides of the store: destination preparation, value computation,
register lifetimes, carry and the exact payload accesses. Prepare the destination
without destroying a live result. Reuse the existing A16/X high-lane strategy
for complete three-byte results, with Y available for destination indexing.
Do not add stack pushes just to avoid an allocated temporary. Conflicts with
reserved X or DP residents require a different proven schedule or fallback.

Evaluate all required source bytes before the first observable store whenever
the destination might overlap the source. Do not replace a captured pointer
copy with interleaved reads/writes that change partial-overlap behavior.
Only pure computations or reads of proved stable private values may be
rescheduled inside the selected region. Other memory reads retain their sites.

Keep 24-bit carry/wrap behavior, exact three-byte extents and existing symbolic
relocation checks. A zero offset becomes an alias; a nonzero offset is arithmetic,
not a pointer-type special case. Emit external pointer stores as a low word and
one bank byte; overlapping word stores remain confined to private homes.

Acceptance: pointer field initializers and address-valued assignments store
their results without a stack capture. NewList should need no temporary frame
after slices 1–2; verify this through its emitted code and frame map. Compare
the full selection cost, including mode changes, rather than counting removed
STA/LDA pairs alone.

## Slice 3 — Reuse prepared indirect bases with explicit invalidation

Suggested commit: `65816: reuse checked indirect address bases`.

Track a prepared address as a full value identity plus source-definition
generation, the existing DP scratch triplet and its write generation. Different
TempIds borrowed from the same stable source may identify the same base.
Equal stack offsets or a shared source name are insufficient evidence.

Route address preparation through an ensure-base operation: stage on a miss,
omit staging only when the complete current fact proves equality. Keep field
displacements in Y so accessing one field does not modify the base. Reuse this
for loads and stores across ordinary records, descriptors and pointer traversal,
independently of payload type or field names.

Use typed instruction effects to invalidate on overlapping scratch writes,
source changes, calls/helpers, assembly/unknown effects, domain/S transitions
and labels/joins. Preserve source operation ordering and existing memory facts.
Do not preserve a witness merely by restoring it after a generic barrier.

In particular, persistence across a store needs a preservation proof for the
cached DP bytes and any borrowed source. Audit the private-scratch ownership
rule already used by the pointer-leaf allocator and express that rule explicitly
in this plan's checked access contract. The generic home analysis currently
treats unresolved indirect writes as potentially aliasing every tracked byte;
do not silently weaken it. Absolute/volatile/escaped or otherwise unresolved
aliasing retains invalidation and reloads. If NewList cannot satisfy the existing
ownership contract, retain its reloads and report that limit rather than invent
a new language-level nonaliasing assumption to reach a byte target.

Acceptance: several accesses to a proved stable base stage it once; a scratch
clobber or relevant alias forces restaging. Repeated reads of mutable pointee
fields still happen. DP reuse is a bounded selection fact, not a general memory
load cache or whole-function register allocator.

## Slice 4 — Reconcile zero frames, validate and measure

Suggested commit: `65816: verify address-consumer frames and record size savings`.

Recheck frame extent, spills, edge staging, incoming offsets and local stack
peaks from the final demand plan. Preserve real local objects and caller/callee
stack obligations. Keep omitted operations' source spans and truthful empty
home entries; update typed replay and proof observations alongside selection.

An important boundary: the current ordinary prologue validates stack bounds
even for a zero-byte frame. Smaller frames do not automatically authorize
guard removal. Keep that diagnostic policy in this series and report its cost
separately. A general guardless-leaf rule would be a separate policy change,
requiring proof of no calls, pushes, frame objects or staging, valid native-entry
obligations, and an explicit decision about invalid-entry diagnostics.

Run focused checks while each executable slice is developed; batch the affected
MIR65816 unit/emission/o65 checks and runtime cases at the integration checkpoint.
Do not repeat passing suites after documentation-only edits. Cover:

- Raw and optimized NIR; unrelated record layouts/names, mixed payload widths,
  multiple pointer bases, zero/nonzero offsets and repeated alias uses.
- Bank carries, 24-bit wrap, pointer-byte canaries, dirty hidden B, exact source
  access order, same-object and partial-overlap cases.
- Source reassignment, volatile/absolute accesses, calls, cross-block uses,
  dynamic-index fallback, scratch conflicts and missing/stale demand proofs.
- Final incoming-stack offsets, domain/stack canaries, fixed/o65 placements,
  and IRQ/NMI suspension through the longer DP/register lifetimes.
- LF/CRLF through any new newline-sensitive fixture instrumentation.

Compare NewList plus other Exec list routines and independent record/descriptor
fixtures. Recompile the same 960-routine Exec inputs with a recorded local
compiler override; report total bytes, frames, local peaks and every growth
case. Do not promise an exact NewList byte count before the schedules and alias
proofs are implemented. Keep the compiler pin and play image unchanged.

Every slice must deliver executable behavior and be committed with its focused
checks. No general NIR/SemIR rewrite, ABI/data-layout change, new scratch region,
full hosted-system qualification or unrelated backend suite is part of this
plan. Reserved bank-zero delta: **0 fixed bytes and 0 bytes per task**; reduced
routine stack demand does not reduce the existing task stack reservations.

Related foundations: [storage demand](MIR65816_STORAGE_DEMAND_PLAN.md),
[expression consumers](MIR65816_EXPRESSION_CONSUMERS_PLAN.md),
[address selection](MIR65816_ADDRESS_SELECTION_PLAN.md), and
[physical home analysis](MIR65816_HOME_ANALYSIS.md).


## Implementation checkpoints

Slice 1 removes borrowed and alias homes before final allocation. A conservative
layout preview retains the existing exact stack/call admission checks; final
resolution independently checks the compacted homes. Two new native cases pass
for banked field stores and mutable-pointer snapshots, together with the three
pointer-forwarding runtime cases (including IRQ/NMI) and six pointer-value cases.
The 18 forwarding and 11 demand unit cases pass; the integration batch follows
with the remaining slices. Reserved bank-zero delta: 0 fixed / 0 per task.
