# MIR65816 argument and result flow

Status: **proposed design; implementation and qualification pending**.

This note defines ownership at native call boundaries within the existing
`action65816.native.v2` ABI. It extends the
[placement contract](MIR65816_PLACEMENT_CONTRACT.md) and
[emission contract](MIR65816_EMISSION_CONTRACT.md); their implemented guarantees
remain authoritative until the new forms are implemented and qualified. The
[argument/result investigation](benchmarks/65816-exec-call-audit/argument-result-flow/README.md)
provides the workload evidence and proposed delivery slices.
The [implementation plan](MIR65816_CALL_FLOW_IMPLEMENTATION_PLAN.md) schedules
the foundations, consumer slices and qualification in stages 0–6.

The central distinction is between incoming values, which a call clobbers, and
its returned value, which has a new definition. A native result may have a
checked consumer or final destination without first acquiring a temporary
memory home. An argument may borrow private storage only while a complete
construction plan proves that storage still contains the value of its source
load.

## Scope and layer ownership

SemIR retains callable meaning, types, lvalue legality and source evaluation
order. NIR retains explicit typed computation, storage identities, loads,
stores and conservative effects. This work requires no new source semantics
or NIR executable forms.

MIR65816 owns native lanes, result destinations, borrowed read locations,
argument schedules, stack bounds and profitability. Emission consumes checked
decisions and records their instructions, effects and source attribution.
Neither layer recovers semantic facts from source syntax, display names or
SemIR lookups.

The first admitted forms are sole adjacent byte/word result zero tests, sole
adjacent result assignments into private locals at widths 1–4, and bounded
private byte/word reads used as arguments in calls with multiple arguments.
Existing immediate call-result returns should use the same result ownership
model while retaining their current eligibility rules.

Initially, result routes apply to resolved Direct, Helper and Runtime targets
with a verified native call contract. Borrowed arguments terminate at Direct
calls, matching the existing bounded borrowing boundary. Indirect calls retain
their current opaque construction and capture paths.

Wide register arguments, result chains through casts, indirect or exposed
destinations, cross-block result residence and new public ABI conventions need
separate admission and qualification. No incoming register, flag or compiler
DP cache gains permission to survive a call.

## Facts and planning boundary

The common placement plan owns these decisions before allocating temporary
homes. It must distinguish result declaration validation from physical home
validation: a result's width and declared native convention can be checked
without requiring an allocated temporary. A missing home is legal only when a
complete alternative owner and all of its reads have been admitted.

Each call-flow decision uses typed facts tied to the immutable routine and
its allocation and selection generations:

| Fact | Required identity and obligations |
| --- | --- |
| Native result origin | Call program point, result temp, checked target/signature contract, width and complete payload lanes |
| Result route | One owner: canonical temp home, native register consumer, or final private object write |
| Result consumer | Exact logical occurrence and program point; payload use is distinct from address use |
| Borrowed argument | Source load, temp, parameter/frame-object identity, full extent and unchanged-storage proof through its terminal use |
| Argument construction | Complete ABI layout, argument ordinals, source locations, physical schedule and transient S displacements |

These are contract requirements, not a prescribed new Rust API. Reuse the
existing program points, use sites, capture/read-location model and checked
native call plan. Avoid an independent planner with a second notion of home
demand. Numeric offsets or equal widths never establish ownership.

The complete logical use census remains mandatory, including unreachable
occurrences, address bases and indices, indirect targets, block parameters,
terminators and distinct parallel edges. A result used as a store address is
not a result assigned to that store's destination. Omitting a home must leave
no unaccounted read or implicit capture.

## Call phases and native result origin

Selection and validation distinguish the following ordered phases:

1. Validate inputs and establish any protected register argument source.
2. Check the complete outgoing/transfer stack allowance before construction.
3. Build and initialize the complete outgoing area.
4. Consume terminal input bindings, transfer to the callee, and accept its
   declared native return contract.
5. Release the outgoing area, preserving a used result's complete lanes.
6. Publish the returned value at the caller's body-S anchor, then perform its
   admitted capture or consumer.

The logical result definition remains the Call operation. Its native origin is
the callee return; its read permission for a following MIR operation begins
after checked cleanup. This permits a new output interval without extending
an incoming interval across the call. Borrowed argument permissions end before
callee execution and cannot authorize result capture or later reads.

The [physical ABI](MIR65816_PHYSICAL_ABI_V2.md) supplies the exact lanes:

| Result bytes | Payload after cleanup | Required unused-lane fact |
| ---: | --- | --- |
| 1 | A bits 0–7 | A bits 8–15 are zero |
| 2 | A bits 0–15 | No promise about X or Y |
| 3 | A bits 0–15 and X bits 0–7 | X bits 8–15 are zero |
| 4 | A bits 0–15 and X bits 0–15 | No promise about Y |

Cleanup must preserve every payload lane and required zero extension, restore
zero transient stack displacement, and finish with the normal native boundary
state. A used result requires preservation even when no memory capture follows.
A discarded result still retains the callee's full declared effects.

The callee supplies no useful flag promise. The initial zero-test form emits a
fresh comparison with zero at the declared width. Reusing N/Z established by
cleanup would require a separate witness tied to the exact result origin,
width, extension facts and intervening selected actions. Mere knowledge that
cleanup currently ends with TYA is insufficient.

## Result destinations and consumers

### Canonical temporary

The conservative route captures into the allocated temp home after cleanup.
Its complete extent is validated independently of the result declaration.
The existing exact A/A-X stores and subsequent home reads remain available
for every valid unsupported or unprofitable use shape.

### Native register consumer

Initially admit only a sole adjacent `Eq` or `Ne` comparison against literal
zero in the same block, for one- or two-byte results. The Call owns the new
native output; the comparison owns its read and ends the interval. A subsequent
branch or materialized Boolean follows the comparison's ordinary ownership
and control-flow contracts.

No unrelated operation, label, second use, edge use or incompatible width may
appear inside this interval. Complete resource checking includes cleanup and
comparison mode changes; no temp-home load may be emitted for the omitted
home. Other comparisons retain the canonical route.

The existing sole adjacent native Return form can be represented as another
explicit register consumer, including its A/X widths and result-preserving
frame teardown. Until that form is migrated and independently qualified,
retain its current reserved-home preflight. This note does not declare those
reserved homes removable by itself.

### Final private local

Admit a sole adjacent Store that uses the result as its payload, with an
unindexed, nonvolatile, unexposed automatic local destination and matching
width 1–4. Validate its stable object identity, mutable local ownership,
resolved byte displacement and complete allocated extent. Parameter-backed
frame objects, pointers, static/absolute storage and computed addresses do
not qualify for this initial form.

The route owns the Call result and its one Store occurrence. It keeps the
payload in native lanes through cleanup and performs the required object write
at the original Store program point. Call and Store retain separate source
spans; the object write belongs to Store. The value is not published as stored
in that object at Call exit. The Store consumes the lanes directly, without an
intermediate temp-home store or reload.

This ordering avoids moving the semantic write into the call window. The
resource proof must cover the returned-lane bridge and the Store's complete
address preparation and writes. A three-byte destination receives exactly
three bytes; no neighboring fourth byte is touched. Further object reads
remain ordinary storage reads, not an extension of this temp's lifetime.

## Bounded private argument borrowing

A borrowed argument replaces a captured source Load with a checked read from
its authoritative private storage during argument construction. The logical
Load and its definition remain in MIR; its physical read may move only under
this explicit unchanged-storage proof.

Initial sources are immutable incoming parameters and unexposed automatic
locals with complete one- or two-byte canonical extents. Admission checks the
routine-wide storage-use facts for address escape, mixed or partial views,
volatility and aggregate/copy involvement. A frame object backing a parameter
does not become a private local merely because its slot has the right width.

From the source load through its terminal call use, the selected interval must
remain in one block with no intervening call/helper, store, copy, machine block,
volatile access or other ordering barrier. The storage must contain the source
load's value at every deferred physical read. Source identity and the checked
interval establish this fact; numeric home equality does not. This is bounded
borrowing, not a general memory-version or record-field reuse analysis.

Each candidate temp initially has one logical use, as a call argument. Several
such inputs may share a call, but the complete call construction must validate
all admitted bindings and ordinary operands together. Inputs not admitted
retain their normal captures. No borrowed binding survives transfer, and no
pointer-field or hardware read moves under this rule.

## Complete argument construction

Source expressions retain their language evaluation order. Constructing an
already evaluated ABI area from high addresses downward does not grant
permission to reorder loads, calls or effects in that evaluation.

The call plan must preflight both its complete layout and the actual chosen
construction sequence before emitting any byte or omitting any source home:

- Every argument has its declared width, extension rule, ordinal and slot.
  Payload and padding cover the outgoing area; padding is zero, including the
  ABI's one-byte area for a no-argument call.
- Every private source read uses the actual S displacement at that construction
  step. Its complete access fits the stack-relative `1..255` range and remains
  above and disjoint from the fresh outgoing area. No truncation or red zone is
  permitted.
- The guard precedes the first outgoing allocation or push. The full outgoing
  and transfer peak, partial construction, final depth and cleanup are checked
  against the routine's allowance.
- Any live register source has a complete preservation schedule through the
  guard and all earlier construction actions. Mode changes include hidden
  accumulator and index lanes; all typed instruction effects participate.

The current one/two-byte accumulator argument mechanism protects its value in
Y before the guard, which clobbers A/X. Reusing that mechanism for a call with
multiple arguments requires proving that every intervening construction action
preserves Y until the source is consumed. There is no default Y-survival rule.
The first borrowing slice reads private homes and does not depend on admitting
this wider register-source case.

Three/four-byte A/X results passed to another call need a separate complete
schedule for the next guard and other arguments. They retain capture for now.
Neither result zero extension nor an unchanged-width cast protects X from that
guard.

## Verification, replay and artifact truth

The sealed placement contract records call input bindings and post-cleanup
result ownership separately. The Call remains a barrier for incoming registers,
flags and all compiler scratch. Adding a declared result output cannot weaken
its callee effects or infer that an earlier residence survived.

A native-output fact must connect the checked callee convention to the actual
cleanup and its publication point. Whole-operation register masks alone cannot
justify that interval; fresh replay derives the output permission from the
native call contract and the selected cleanup actions.

Verification reconstructs eligibility from immutable MIR, data and final
allocation. It checks exact definitions and uses, lane widths, destination
ownership, borrowed storage intervals, partial stack geometry and complete
resource windows. Result reads must follow the admitted origin and cleanup;
final-object writes must remain at their Store point. Allocation or selected
rewrites invalidate generation-bound proofs and require reconstruction.

Fresh selected-action replay rechecks the typed native call contract, actual
clobbers, stack phases, returned lanes, source attribution and real read/write
extents. Recorded admission answers are not permissions. Final reconciliation
must preserve owner/generation identity while branch layout remaps byte
positions. Empty source spans for an admitted omitted capture or deferred Load
must remain attributable to their checked decision.

Artifact maps describe real canonical memory homes. A register-only or
redirected-only temp receives no fictitious stack/DP entry; the final frame
object remains mapped truthfully. Internal native origins and read bindings
are checked placement facts, not serialized memory homes. This design requires
no image-format or public ABI change.

## Fallback and cost policy

Valid unsupported shapes and complete plans that lose on cost use the existing
capture/read and argument-construction strategies. Decide this atomically
before home allocation and emission. Never omit a home speculatively and then
discover that the guard or argument schedule needs it.

A malformed ABI declaration, missing ownership, stale generation or uncovered
logical use is a compilation error, not a profitability fallback. Failure
recovery must not conceal a broken admitted contract.

Use deterministic, size-first comparison of complete sequences: captures,
reloads, mode changes, lane preservation, argument construction, cleanup and
any changed guard/frame cost. Structural capture-byte totals from the audit
are candidate footprints, not predicted savings. Trial allocation must not
increase routine frame, spill extent or local peak relative to the conservative
strategy. No new persistent DP reservation is allowed.

## Qualification obligations

Each admitted form needs focused MIR65816 planner/verifier tests and independent
native execution. Assembly callees must exercise declared lanes while poisoning
unspecified flags and registers. Oracles check exact result values, argument
slots, padding, external access extent, S and native execution state; matching
compiler-generated instruction patterns alone are insufficient.

Cover byte/word zero and nonzero extremes, branch and materialized Boolean
consumers, all final-store widths, exact three-byte tails, multiple borrowed
arguments, intervening clobbers, source/destination alias rejection and partial
stack displacements at range boundaries. Forged owners, stale plans, hidden
uses and parameter/local identity confusion must be rejected. Unsupported
valid shapes must retain working conservative paths.

Qualify raw/optimized and guarded/release modes, relocated images, and task,
IRQ and NMI interruption at changed call phases. Use the existing home-demand,
call-return, argument-construction and terminal-pointer-call harnesses identified
in the investigation. Broaden to the full affected 65816 suite for final backend
qualification, including applicable LF/CRLF fixture paths.

Rebuild the frozen Exec816 profiles with recorded compiler, source and artifact
identities. Report complete code size, per-routine frames/spills/local peaks,
native cycles and private traffic, plus compiler time and memory cost. Preserve
the [stage-7 acceptance gates](benchmarks/65816-record-placement-stage7/README.md#frozen-final-acceptance)
and distinguish native evidence from hosted-provider qualification. The
implementation plan schedules delivery and those checks; this note owns the
boundary contracts.

## Implemented boundary

The common Call table now owns declaration and route facts independently of
physical homes. Placement recomputes those facts from immutable MIR and the
complete logical use census, then seals them with the selected generation.
Selected transfers must reproduce their target and complete native ABI exactly
once at the original source point. Stage 1 retains all existing result homes
and instructions; output ownership and trial allocation follow in stage 2.
