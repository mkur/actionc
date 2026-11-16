# MIR6502 unexposed object storage

Status: base contract implemented and qualified, 2026-11-15.
Shared address-control facts, residence verification and bounded native
indexed/constant-byte-shift admission are implemented.
The [implementation plan](MIR6502_UNEXPOSED_OBJECT_STORAGE_IMPLEMENTATION_PLAN.md)
defines four slices with focused validation and a commit after each slice.

The [ZP lifetime extension](#zp-home-lifetimes-and-persistent-storage),
proposed 2026-11-16, is a design only. Existing invocation-only bindings and
their verifier restrictions remain implemented behavior.

## Decision and motivation

Whether an ordinary pointer access can observe a compiler-managed object must
depend on that object's address exposure, independently of its placement.
Compute that fact before choosing inline, ZP or register storage. Separately
prove the object's lifetime and the absence of conflicting physical writes.

Sort demonstrates the current inconsistency. Its Fnamecmp call now passes the
entry/write contract, but the caller selector and expanded residence verifier
reject indirect writes. The compiler therefore never tries moving its locals.
An indexed array write can physically hit either an inline or a ZP address;
the existing inline fallback is not an alias-disjointness proof.

This extends the shared ownership model in the
[storage-observation design](MIR6502_STORAGE_OBSERVATION_DESIGN.md) and the
[parameter/local placement design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md).
The completed [private-storage boundary work](MIR6502_PRIVATE_STORAGE_BOUNDARY_IMPLEMENTATION_PLAN.md)
already applies a non-exposure rule to compiler-created temporaries. This
proposal extends address-control facts to eligible source objects; it supersedes
the blanket indirect-write exclusion for those objects when implemented.
Historical qualification results and unrelated conservative consumers remain valid.

## Storage access contract

The compiler chooses the physical home of an ordinary object unless the program
exposes or constrains that home. A supported program may use the object by its
language identity or through an exposed address. It cannot depend on an
undisclosed compiler-chosen address, including one guessed through numeric
pointer arithmetic or inferred from another object's accidental adjacency.
This is an implementation-storage contract, not a numerical bounds proof or
hardware memory protection.

Preserve explicit address-taking, aliases, fixed placement, data/machine
relocations, decoded numeric physical observations, declared regions and ABI
requirements. Missing or ambiguous exposure evidence grants no exception.
Source-owned ZP is program storage. Pointer arithmetic, pointee read/write
ordering, hardware effects and Action local persistence retain their meanings.
No runtime bounds checks or new annotation syntax are introduced.

Taking the address of a pointer *cell* exposes that cell. Reading the pointer
value, passing that value or dereferencing it does not by itself expose the cell.
The pointed-to program storage remains observable in all cases.

## Three independent facts

| Fact | Question | Existing authority to extend/reuse |
| --- | --- | --- |
| Address control | Can ordinary pointer access legitimately reach this object's hidden home? | Shared ownership and exposure inventory |
| Invocation lifetime | Is every value read defined in this invocation? | ABI capture and logical home liveness |
| Physical safety | Does any scratch, ABI, callee or explicit physical write overlap a live home? | Allocation, expanded effects, residence verification and final layout |

An affirmative address-control fact does not make a source local a temporary.
It does not authorize deleting its storage, discarding reads, merging objects,
reordering accesses or sharing a lifetime. Keep the source declaration identity
and the existing storage role. Each transformation retains its own proof.

An unexposed local may retain its value across invocations. Initially relocate
only the existing uninitialized BYTE/WORD scalar or pointer locals with a
complete definition before every read on every invocation. Partial/conditional
initialization, entry-value dependence and initializer-backed storage retain
their existing homes. Parameters use verified ABI capture. Calls and physical
scratch writes never count as logical initialization of a caller's object.

This initial admission rule applies to shared invocation homes. It is not a
requirement that persistent values live outside ZP. The extension below bases
direct-home sharing on value lifetimes and treats working copies as optional.

## One shared address-control query

The shared [storage ownership module](../src/mir6502/analysis/storage_ownership.rs)
provides `ObjectAddressControl::compiler_controls_address(owner, object)` from
an immutable program's declaration/exposure inventory. Use routine-qualified ParamId/LocalId and
existing typed memory/storage identities. Names and trace records remain diagnostic.
Unknown identity or incomplete inventory returns no proof.

The query must work for an inline object before placement is attempted. A ZP
binding is neither a prerequisite nor evidence of privacy. Moving an otherwise
unchanged object must preserve its logical exposure classification. Known
physical observations are checked against both placements separately.

Consolidate the exposure checks currently used by
[object_exposed](../src/mir6502/parameter_homes.rs),
[local eligibility](../src/mir6502/parameter_homes/locals.rs) and
[InvocationHomes](../src/mir6502/parameter_homes/observations.rs) behind that
shared authority. Reuse typed machine observations and retained-program exposure
evidence. Do not introduce a competing object inventory or a general points-to
analysis. Build facts once for each existing immutable program snapshot and
reuse them across candidates; a changed body, declaration or exposure needs
current evidence.

Logical exposure can be established before addresses are final. An explicit
numeric observation or overlap which still depends on layout remains a pending
physical obligation. Unresolved obligations cannot approve a final binding.

For the native pointee extension, finalization retains the existing emission's
physical map and complete declaration inventory. It checks resident ranges;
the placement transaction additionally checks the old ranges and retained input
observations before accepting relocation. This adds no emission or materialization.
Decoded machine operands and declared physical regions remain observations;
an undecodable explicit machine operand blocks this extension conservatively.
Discovery rejects that known missing obligation before adding a native writer
to the combined proposal, preserving independently eligible candidates. Concrete
overlaps still require both final layouts; there is no subset retry.
Generated fixed-ZP scratch operands remain the residence verifier's responsibility.
Different invocations may reuse certified compiler-chosen homes, but fixed source
declarations and aliases retain protection in both layouts.

Explicit fixed globals and their typed alias chains share one address query in
the machine scanner and physical collector. The query retains logical identity
and resolves only declared fixed backing, with checked offsets and bounded alias
traversal. Indirect pointer fetches remain separate byte observations, including
zero-page and NMOS JMP page wrapping. This does not establish a pointee footprint
or make generated scratch traffic a source observation. Layout-dependent storage
still requires the existing authoritative placement evidence.

Keep every declaration in the physical storage inventory, including unexposed
objects and unused aliases. Address control is not permission for another
allocation to overlap that object. Keep `source_storage_disjoint` conservative;
contextual consumers must identify the object and applicable lifetime explicitly.

## Ordinary pointee effects and physical effects

### Symbolic observations and ABI descriptors

Post-materialization liveness must read every populated call argument home.
The retained argument values may be placeholders after argument setup, so they
cannot substitute for those ABI reads. A byte used as pointer scratch elsewhere
in the same routine can also carry an outgoing argument; its scratch role never
makes that argument definition dead. Pre-materialization effects keep treating
argument homes as setup destinations.

The [symbolic-observation plan](MIR6502_SYMBOLIC_STORAGE_OBSERVATION_IMPLEMENTATION_PLAN.md)
extends this boundary in commit-sized slices. MIR6502 owns SArgs parameter
capture as one typed operation containing its helper call and inline descriptor.
The descriptor references a parameter ID in its owning routine and an explicit
byte count. Emission resolves the contiguous frame and writes the ABI bytes.
Descriptor bytes are not executable assembly; the helper retains its conservative
memory/register/stack effects and the complete captured frame retains protection.
This representation does not authorize a new SArgs home-placement strategy.

Explicit machine observations retain typed storage identity, signed addends,
operand encoding and addressing behavior. An indexed encoded base can precede
its named object; this does not prove an out-of-bounds runtime access or supply
an index bound. Exact callee-summary checks remain separate. A decodable operand
awaiting layout is a pending physical obligation, distinct from opaque code.

Final resolution validates the encoded base under emission's relocation rules,
then applies ISA indexing and pointer-fetch wrapping. Each retained input uses
its corresponding executable layout, with missing mappings failing the proof.
Both old and proposed homes remain checked, and cleanup retains logical exposure
as well as physical ranges beyond the named object. Unresolved final obligations
cannot approve relocation or deletion. The existing single proposal, whole-image
cost guard, preservation replay and exact rollback remain unchanged.

### Effect classification

For a qualified unexposed object, use the following distinction in placement
and verification:

| Effect | Consequence |
| --- | --- |
| Typed native read/write through an ordinary pointer or array index | Does not observe or clobber the object's hidden home; preserve the actual pointee access |
| Logical access to the object | Reads or defines the object through its authoritative binding |
| Address preparation, emitted fixed scratch or ABI write | Check physical overlap with the live object |
| Supported direct call | Apply existing per-site entry, argument, result and callee-write checks |
| Opaque, undecoded or unaccounted operation | Retain conservative rejection |

Reuse the memory, home and address-pair portions of
[operation effects](../src/mir6502/analysis/effects.rs). Only the ordinary
pointee component receives the object-specific exception. Pointer-cell reads,
address construction, carry/flags, register effects, stack traffic and implicit
scratch remain part of the expanded operation's proof. A partial word write or
one overlapping pointer lane is a real conflict.

Do not globally weaken `Unknown` or clear `indirect_writes`. The complete public
write footprint of a pointer-writing routine remains Unknown. No Exact callee
summary is invented by excluding one caller object. Current call admission
rules, including rejection of unknown writes, remain unchanged in this scope.
An opaque call's possible ABI/scratch clobbers do not become ordinary pointee
effects. Conservative value-fact invalidation may remain where no consumer uses
the new object-specific proof.

## Placement and verification

Replace the indirect-write veto in both
[candidate eligibility](../src/mir6502/parameter_homes.rs) and
[expanded lifetime verification](../src/mir6502/parameter_homes/lifetimes.rs)
for objects satisfying the shared contract. Check those objects even when MIR
bindings are supplied directly rather than produced by automatic selection.

Initially support existing native byte/word indexed reads and writes, plus
constant BYTE Lsh/Rsh counts 0 through 8. Their actual expanded recipes must
account for every physical resource used. A zero shift need not define flags;
this proposal grants no new entry/exit flag facts. Variable shifts, helper-producing
arithmetic and unqualified word-shift recipes retain their current exclusions.
Do not broaden a caller merely because an operation is labelled nonopaque.
An array parameter with a verified scalar word ABI lane is a pointer cell and
may receive a home. This does not promote inline array objects or reinterpret
array storage that has no scalar lane.

Keep current source-ZP exclusions, logical binding resolution, scratch
reservations, definite initialization, call checks and pair coherence. Binding
selection may exclude live call-clobbered slots as today. Expanded verification
must still catch hidden physical conflicts in the candidate.

InvocationHomes consumes address control plus its existing initialization,
residence and ABI proofs. Its current scoped cleanup exception does not become
a general permission to prune source storage or move inline allocations.
Terminal cleanup continues to prove individual stores dead and replay earlier
preservation obligations. It may not ignore an observation merely because a
source object is now classified unexposed.

Use the same retained core continuation and combined ordinary-call proposal.
Keep frames/bindings fixed within downstream terminal transactions. Recheck
explicit physical observations using authoritative baseline and candidate
layouts and retained witnesses. Recompute candidate facts after expansion;
stale facts never validate a caller rewrite. Accept only a valid, strictly
smaller complete image, with exact rollback otherwise. No subset retries,
extra materialization stage or per-object compilation is added.

### Loop-carried compiler spills

An optimized source local can become a compiler spill before placement. Its
absence from the source-local binding table must not permanently exclude it
from ZP. Use `StorageOwnership` to select private byte spills, retaining address
exposure, source-owned bytes and definite entry initialization as exclusions.

The first slice considers CFG-spanning byte spills in callers of ordinary leaves.
Each selected byte has an exclusive invocation-long allocation in `$E0..$EF`;
every call must preserve it. Unknown calls, assembly, helpers, exhausted capacity
and wider spill accesses remain ineligible. Ordinary pointee stores use the
existing private-storage boundary, without relaxing physical scratch checks.

Discover these spills after core materialization and coloring, inside the same
combined cross-call placement trial. Do not carry spill IDs across core replay.
Allocate before late cleanup so its decisions and preservation witnesses are
rebuilt for the new homes. Keep downstream frames fixed. Recheck final callee
writes, scratch conflicts and explicit observations in retained source, accepted
fallback, original core and final layouts. Protect the fallback's entire inline
spill inventory for affected callers when coloring prevents an ID correspondence.
A single measurement of the original core supplies the old spill addresses.
Acceptance remains strict whole-image improvement, with
complete rollback on a failed proof or cost check.

## ZP home lifetimes and persistent storage

Proposed 2026-11-16. This extends placement policy without changing Action local
persistence or relaxing the address-control contract above.

The [implementation plan](MIR6502_ZP_HOME_LIFETIMES_IMPLEMENTATION_PLAN.md)
delivers direct invocation placement and lifetime-based sharing first, in four
slices. Dedicated persistent reservations have a separate initialization and
global-preservation boundary; working copies remain optional follow-on work.

### Decision

A value's storage duration does not determine its address range. Prefer a direct
ZP home when its value lifetime and physical preservation permit it. Separate
call subtrees may share the same bytes when their values do not interfere;
placing a home in ZP does not imply reserving those bytes for the whole program.

The strategy order is:

1. Direct ZP homes, shared wherever proven value lifetimes permit. Extend the
   existing placement and reservation machinery without adding a backing copy.
2. Dedicated ZP homes for values that must persist between invocations, where
   reservation, initialization, preservation and final size justify them.
3. Optional working copies with retained canonical backing, only when measured
   savings justify the extra traffic and storage. They are not a prerequisite
   for either direct strategy.

These arrangements have different proof obligations:

| Arrangement | Canonical storage | ZP reservation | Required value at entry |
| --- | --- | --- | --- |
| Direct invocation home | Selected ZP bytes | Proven value lifetime; existing policy conservatively reserves the invocation | Defined by capture or this invocation before use |
| Dedicated persistent home | Selected ZP bytes | Includes time between invocations; program lifetime unless a shorter interval is proved | Same initial and retained value as the original object |
| ZP working copy | Existing source home | Bounded live region | Copy invalid until populated from a proven current value |

There is no measured pool-exhaustion result that makes working copies the
preferred strategy. They retain backing storage and can add stores and reloads,
which matters when optimizing for image size. Measure ZP pressure and complete
image cost; do not infer exhaustion from the total number of declared homes.

An entry-dependent local remains ineligible for the existing invocation-home
binding. Do not simply remove that verifier check or silently reinterpret
`MirLocalHome::ZeroPage` as a permanent reservation or a cache.

### Sharing follows value lifetimes

Call-graph separation proves that supported activations cannot overlap. Sharing
also requires that no value needed later is lost. For example:

```text
A defines a retained local value
B writes the same proposed ZP bytes
A reads its retained value on its next invocation
```

A and B do not execute simultaneously, but their values interfere. Conversely,
if both define their homes before each read on every invocation and neither can
reenter the other, their independent subtrees can reuse those bytes.

Use the existing call graph, logical liveness and physical reservations:

- Sibling subtrees may reuse bytes when neither has a retained value crossing
  execution of the other. This must hold for their relevant descendants too.
- A caller's home conflicts with descendant writes while its value is live
  across the call. Whole-invocation exclusion remains a valid conservative
  policy until finer liveness is supported.
- Argument setup, result delivery, helpers and scratch participate in the same
  interference check. A disjoint source call graph alone does not certify them.
- Recursion, callbacks, interrupts and unresolved targets require applicable
  preservation/non-overlap facts. Missing facts retain the existing placement;
  this extension does not add general recursive or asynchronous analysis.

Source persistence alone does not force a program-long reservation when all
relevant values are proved dead before reuse. An entry read does require
preserving the reaching value until that read unless a separate transformation
proves it unnecessary. Do not infer death merely from a routine returning.

Reuse the coordinated proposal and sibling-reuse rules in the
[acyclic call-chain design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-zp-placement-through-acyclic-call-chains).
Do not create a second call-graph allocator or another per-subtree compilation
trial. Narrower live intervals can refine this machinery in a separate slice.

### Dedicated persistent ZP homes

A direct persistent binding replaces the canonical inline home. Its reservation
must cover every interval in which the retained value is needed, including time
when the owning routine is inactive. Until a shorter lifetime is proved, use an
explicit program-lifetime reservation visible to all routine allocators,
ABI/scratch users, helper expansion and final layout. Separate call subtrees
cannot reuse these particular bytes merely because their activations are disjoint.
Other invocation homes remain shareable under the preceding rules.

Such placement must preserve initial contents, subsequent stores and repeated
program-entry behavior. ZP contents cannot be assumed to match emitted inline
storage at startup. Account for any required initialization in both semantics
and final cost. Source-owned ZP, fixed aliases, hardware, numeric observations
and runtime/interrupt reservations retain precedence.

Compiler-wide reservation alone cannot certify opaque external code. Relevant
runtime, assembly and external-call contracts must preserve the object while it
is live, including between owner invocations. Unknown obligations keep the
existing home. This design does not invent guarantees for the keyboard handler.

Global ZP pressure is part of profitability: a dedicated home can force larger
code or spills elsewhere, while invocation homes can share. Accept a reservation
only through whole-image measurement and complete rollback of affected placement
state. Implement direct placement and sharing first; persistent reservations are
a distinct extension that does not depend on introducing working copies.

### PopUp.menu and the actual lifetime

In [PopUp](../samples/tn/modern/LIB.ACT), `Push(menu)` reads the previous value
before `menu = mp`; `menu = Pull()` restores it near return. The current local
liveness analysis correctly identifies an entry dependency. A direct ZP home
is possible if its retained value and physical bytes can be preserved over the
required lifetime. Call-subtree sharing alone does not justify overwriting this
entry value. Proving that the save/restore value is unobservable would be a
separate transformation, not an assumption made by placement.

If a direct home cannot be justified, inline storage remains the fallback. A
working copy for the value defined by `menu = mp` is an optional size candidate;
the following table describes that alternative, not the default placement:

| Point | Canonical menu home | Working copy |
| --- | --- | --- |
| Entry and `Push(menu)` | Previous invocation value | Invalid; entry must not read it |
| `menu = mp` | Updated to the current menu | May be populated from the same value |
| Menu traversal and redraw | Remains authoritative | Reads may use it while validity and physical preservation are proved |
| A call without sufficient preservation facts | Remains authoritative under normal source semantics | Invalidate; refill after the call only if profitable |
| `menu = Pull()` | Receives the restored value | The traversal copy is dead; no refill is necessary |
| Return and time between invocations | Retains the restored value | Bytes may be reused by other routines |

Keep the source calls and their order. This design does not remove Push/Pull,
change arena contents or failure behavior, assume the saved value is irrelevant,
or turn a static local into an automatic one. Recursive or callback invocation
does not grant permission to overwrite a live copy: it needs a preservation
proof or forces invalidation before control returns to cached reads.

### Optional working copies and validity

If working copies are implemented, start with unexposed, ordinary, nonvolatile
BYTE/WORD scalar and pointer cells with inline canonical homes. Reuse
`ObjectAddressControl`;
fixed addresses, aliases, address-exposed cells and unsupported storage shapes
retain their present policy.
An entry dependency alone no longer excludes a working-copy candidate, since
the canonical home remains intact. Do not broaden initializer-backed or other
previously excluded storage in this slice.

Canonical storage remains authoritative at every source observation boundary.
Initially keep every logical source store to that home. When a selected region
needs the new value, also populate the working copy or invalidate the old copy.
There is no deferred write-back, dirty state, implicit exit flush or source-store
deletion permission. Extra copy traffic must earn its cost in final emission.

A cached read requires all of the following:

- Every incoming path establishes the complete current value in the copy.
  Reuse existing reaching-definition/value and CFG analyses; a declaration or a
  physical scratch write is not initialization.
- No intervening logical write has made the copy stale. Full definitions can
  update both homes; partial/unknown definitions invalidate the copy unless the
  existing byte facts prove the complete value remains current.
- No intervening physical write has clobbered a selected ZP byte. Preserve word
  lane coherence and the addressing constraints of any pointer consumer.
- Substituting the ordinary home read preserves source ordering, required
  register/flag state and observable reads. It grants no access-reordering rule.

At CFG joins, retain a copy only when every predecessor proves it agrees with
the current canonical value. Otherwise invalidate it and use the canonical home
or explicitly refill. Loop backedges obey the same rule; no validity may be
assumed on the first iteration. The first slice may decline joins requiring
new edge copies or complex region splitting.

### Per-home admission and call boundaries

Classify each object and value region independently. `PopUp` passes `@c` to
Range; keeping `c` inline must not reject an unrelated direct home or optional
working copy. Keep routine-wide ABI and structural verification, but do not
turn one object's exposure or persistence into a veto for every other object.
Unsupported effects can end a proposed region; this is not permission to lower
unsupported executable operations or weaken their normal verification.

Use logical liveness to ask whether a value must survive a particular call. Check
argument setup before invocation effects and argument reads before clobbers.
A value dead before the call requires no preservation proof for that call. A
new result defined after the call has no lifetime before it. This also explains
why PopUp's `item`, consumed between FindItem and Ord, should not require a
contract for Range/Getchar; return-home forwarding remains a separate candidate.

A direct home is authoritative. A live value must survive the callee's physical
clobbers; it cannot be invalidated and reloaded from a nonexistent backing copy.
Preserve legitimate source writes according to their ordinary semantics. When
the required direct-home proof is missing, retain the existing home unless a
separately qualified and profitable preservation strategy applies.

For a copy kept across a call, prove both that its physical bytes survive and
that the corresponding canonical value is unchanged. A callee preserving a ZP
pair can still modify the source object; conversely an unchanged object does
not protect its working pair from ABI or scratch writes. Reuse current typed
entry, write, exposure and preservation facts, including permitted object-specific
pointee reasoning. Do not turn an Unknown whole-memory summary into Exact.

When either proof is missing, invalidate the copy and retain canonical reads
after the call, or refill from canonical storage after the call. Synchronized
stores let unknown readers observe the proper source value without a read-footprint
proof. Reloading after a possible write preserves that write's normal meaning.
Cache invalidation does not bypass the existing call ABI or emission checks.

Reuse the existing and planned call-chain contracts; there is no new recursive
call analysis or assembly-entry admission in this extension. Qualified facts may
keep a direct home or copy live; an unproved call rejects direct residence across
it and can divide an optional copy's regions. PopUp's complete menu loop cannot
currently assume preservation:
Items/Ord/FindItem exceed leaf-only admission, and Range reaches the opaque
Getchar handler. Bottom-up composition of those contracts is separate work.

### MIR ownership and shared machinery

SemIR continues to own local persistence and initialization semantics, carried
by verified NIR. MIR6502 chooses homes, reservation lifetimes and physical bytes.
It must not look back into SemIR or infer lifetime meaning from a name such as `menu`.
Missing semantic facts require a separate typed-IR contract change.

Retain the canonical ParamId/LocalId. Direct placement changes its binding;
reservation facts must distinguish invocation and retained-value obligations.
A direct home has one authoritative location and needs no synchronized duplicate.
When choosing an optional copy, keep the canonical binding and represent the copy
with existing compiler-private MIR homes and explicit loads/stores.
Associate a copy proof with the owning routine, logical object, selected bytes
and CFG definition/use sites. Printed names and trace events are diagnostics.
Do not give one LocalId two ambiguous physical meanings or pretend that its
canonical inline allocation disappeared.

Extend the existing placement, value-tracking and preservation machinery rather
than adding a second local allocator or exposure inventory. Reuse virtual-ZP
reservations and the current pointer-retention consumers where applicable;
competing strategies must not independently allocate copies of the same value
using stale facts. An immutable candidate owns its facts; rewrites that change
definitions, calls, regions or layout require fresh validation.

Source maps and physical storage records identify the selected canonical home,
including a direct ZP binding. A working copy remains a private allocation with
a bounded lifetime. `InvocationHomes` must not classify the persistent object itself as
dead at return merely because one copy has died. Existing private cleanup may
remove redundant copy operations only under its normal proofs.

### Acceptance and focused qualification

Use the existing selected continuation and final-cost transaction. Establish
home allocation and any copies before downstream cleanup, keep frames fixed
within terminal transactions, and replay preservation obligations against the
final candidate.
Validate explicit observations against retained source/witnesses and authoritative
physical layouts. Do not add per-home compilation or a per-routine growth veto.
Invalid or non-improving candidates restore the exact selected artifact and
reservations.

Report instruction savings, removed inline storage, initialization/copy/reload
costs and peak or dedicated ZP use separately. Direct placement may remove the
old inline home; working copies retain it and must not claim its bytes as saved.
Measure competing use of the pool through the final image, without assuming a
distinct reservation for every routine. PopUp is a qualification case, not an
admission special case or a promised size reduction. Its existing call boundaries
may leave no profitable region until their contracts improve.

Focus qualification on the strategy being implemented:

- Direct sharing: two independent subtrees reuse bytes successfully; a caller
  value live across a descendant's clobber cannot use those bytes. Include a
  short-lived local and an exposed sibling parameter.
- Persistent homes: interleave repeated invocations of two routines whose entry
  reads need their old values. Verify that their live retained homes cannot
  overlap, initial contents are preserved, and unrelated scratch cannot overwrite
  them while their owners are inactive.
- Optional copies, when implemented: reuse temporary bytes between invocations
  while canonical values survive; cover preserved and unknown/clobbering calls,
  conditional definition/join and both word lanes.

Check results and retained state against independent oracles. Reuse existing
observation, reservation and terminal-replay controls, including exact rollback.
An implementation plan will choose the affected MIR6502 unit and VM targets,
with LF/CRLF coverage wherever host-text fixtures are introduced. Do not require
working-copy machinery or its tests to qualify direct placement.

## Ownership of the boundary and scope

Source meaning and persistent-local behavior remain owned by SemIR and carried
by verified NIR. MIR6502 derives exposure, lifetime and placement facts from
those typed inputs. Emission resolves final addresses and proof obligations.
No SemIR lookback, string-based identity or NIR shape change is planned. If a
required source fact is absent from NIR, separate that contract migration and
apply its shared checks rather than reconstructing meaning in MIR.

New recursive/nested-call contract analysis, assembly admission, arbitrary
pointer-range analysis, array-object promotion, source-variable DSE and new
ABI/result coalescing are outside this proposal. Direct-home sharing reuses the
call-chain design and its qualified contracts; it does not establish new ones.
Eligible source objects continue to use existing home tables and feature gates.
There is no Sort-specific rule.

## Qualification

Use independent controls for equal exposure facts before/after binding, actual
pointer reads/writes beside live homes, explicit exposure, physical scratch
collisions, stale facts and persistent locals. Execute a native array swap with
a live scalar and an ordinary leaf call, including a word crossing a page.
Check results and program-memory sentinels against an independent oracle.
Do not use an undeclared pointer into a hidden home as a supported-program oracle.

Reuse the final [callee-facts qualification](benchmarks/6502-callee-facts-repair/README.md):
TN is 9,846 bytes, Sort is 250 code bytes with three mapped ordinary-home bytes,
and its Fnamecmp call passes independently. Measure actual selection and savings;
remaining scratch or cost failures are valid outcomes. The implementation plan
scopes local testing to focused unit/VM groups and one final TN measurement.

The [completed qualification](benchmarks/6502-unexposed-object-storage/README.md)
measures **9,838 bytes**, eight fewer than the baseline. Instr gains three ZP
parameter bytes and saves five code bytes. Sort passes native-body admission but
remains inline because TN's explicit assembly-observation inventory is incomplete;
its Fnamecmp call contract passes independently. Existing Ord/Items/FindItem
savings survive. Materializations and emissions are unchanged; final emission
now retains four additional source-storage inventories.

The follow-up [labeled data block plan](LABELED_DATA_BLOCK_IMPLEMENTATION_PLAN.md)
implements explicit `DATA name = [...]` declarations so menu data can leave the
executable machine-block scan while retaining storage and relocation protection.
Unclassified raw machine blocks remain conservative. The
[DATA qualification](benchmarks/6502-labeled-data/README.md) preserves all six
TN table payloads and the 9,838-byte image size. Sort remains inline: the first
unresolved observation is now identified as Block's machine block m7, whose
symbolic `(screen),Y` pointer fetch is not resolved by the observation scanner
despite screen's fixed `$E6` address. This is a separate scanner limitation;
DATA does not weaken the physical-observation requirement.
The completed [fixed-address observation plan](MIR6502_FIXED_ADDRESS_OBSERVATION_IMPLEMENTATION_PLAN.md)
shares typed fixed-storage resolution between the scanner and physical collector.
Its [qualification](benchmarks/6502-fixed-address-observations/README.md) resolves
Block/m7 while preserving the exact 9,838-byte image. Sort's first unresolved
observation is now _Cio/m11's layout-dependent `JMP (ErrorHandler)`; the selected
program also retains unresolved blocks in _StrNam, Range and MakeJmp.

The [symbolic-observation qualification](benchmarks/6502-symbolic-storage-observations/README.md)
resolves all four remaining obligations while preserving Unknown pointee and
helper effects. Sort's two-byte `h` local now resides at `$E2/$E3`, saving six
bytes including its former inline storage. Complete observations also permit
existing terminal cleanup to remove four two-byte staging stores. TN is now
**9,824 bytes**, 14 fewer than the fixed-address baseline, with unchanged
materialization/emission counts and unchanged leaf entry/exit summaries.
Explicit physical conflicts still reject the combined proposal with exact
rollback; complete decoding alone never proves a placement safe.
