# MIR6502 storage ownership and observation across routines

Status: implemented with focused local qualification, 2026-11-15.
Audited baseline: `67e9343a`. [Measured results](benchmarks/6502-storage-observation/README.md).

The [implementation plan](MIR6502_STORAGE_OBSERVATION_IMPLEMENTATION_PLAN.md)
defines six commit-sized slices and the user's requested minimal local testing.

The proposed [unexposed-object storage extension](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md)
separates address control from placement and applies it to native pointer writes.
It has a [separate implementation plan](MIR6502_UNEXPOSED_OBJECT_STORAGE_IMPLEMENTATION_PLAN.md);
the implemented scope and historical evidence below remain unchanged.

This note refines the interaction between
[parameter/local home selection](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-zp-homes-across-ordinary-calls)
and [terminal cleanup](MIR6502_LATE_CLEANUP_DESIGN.md). It defines a narrow
exception to cleanup's global physical-overlap vetoes. It does not change source
memory semantics, the ABI, allocation, or the existing store-deadness proof.

## Decision

Keep the identity, owner and observation role of a storage byte when resolving
it to an address. A verified, unexposed parameter or initialized local placed in
ZP belongs to its routine's invocation. Its presence at an address does not by
itself make an unrelated routine's temporary definition observable there.

Use this distinction inside the existing terminal transaction when deciding
whether a private temporary store is eligible for deletion. The existing
reaching-definition analysis must still prove that the particular definition is
dead. Calls, registers, flags and final placement retain their existing proofs.

Keep the complete declaration inventory and its conservative queries. Add a
contextual query for this consumer; do not weaken `source_storage_disjoint`
globally or reclassify all parameters as temporaries. Allocation pruning and
movement do not inherit this exception.

## Evidence and the two blocking checks

The [cross-call qualification](benchmarks/6502-cross-call-homes/README.md)
records a rejected TN trial: `Ord` becomes 51 code bytes with no inline homes,
and `Items` becomes 37. `MoveMenuBar`, `Draw`, `Tag` and `TagAll` each grow by two
bytes. The strict per-routine cost rule rejects the net 48-byte image saving.
Final TN remains 9,847 cart / 9,954 standalone bytes.

In the trial, `Ord.menu` occupies `$E2/$E3`. In `MoveMenuBar`, forwarding already
passes the second `Block` argument in X, but cleanup retains the staging store:

```asm
; A already contains menu^; symbolic routine name used below.
    SEC
    SBC #$01
    STA $E3       ; retained only in the rejected placement trial
    TAX
    LDA #$80
    LDY #$00
    JSR Block
```

This is not a save/restore sequence. `Next` is not called in this fragment.
Its preservation of `$E2/$E3` permits residence in `Ord`; it does not make this
store necessary or prove it dead.

`Ord.menu` passes the existing
[exposure check](../src/mir6502/parameter_homes.rs): dereferencing its value and
passing that value to `Next` do not expose the parameter cell's address. At the audited baseline, the
problem arose at two later boundaries:

1. [Emission's `source_storage_ranges`](../src/mir6502/emit.rs) resolves every
   declared parameter/local to `(address, size)`. It includes compiler-selected
   ZP bindings, discarding their owner, exposure and invocation facts.
   [StorageOwnership::private_store](../src/mir6502/analysis/storage_ownership.rs)
   rejects a temporary store overlapping any such range. With `$E3` in
   `Ord.menu`, the store is excluded before deadness analysis.
2. [Cleanup's `read_aliases`](../src/mir6502/late_cleanup/mod.rs) flattens reads
   from all routines into another address-only list. `Ord`'s reads of its own
   parameter, including physical fixed-ZP and pointer-pair reads, become
   apparent observers of other routines' stores at that address. This would
   independently veto deletion after the first check was refined.

Both checks must consume the same ownership evidence. Fixing only declaration
protection or only the read list leaves the other veto in place.

## Semantic boundary

Three questions remain separate:

| Question | Authority |
| --- | --- |
| Is this a compiler-managed home with an allowed storage role? | Typed declarations, bindings, exposure and residence verification |
| Can a particular store definition reach a consumer? | Existing CFG home-definition and liveness analyses |
| Can a physical observer see the changed byte? | Observation provenance, ABI/callee contracts and final placement |

An address is not an object identity. Conversely, different `RoutineId`s alone
do not prove that two uses cannot interact. A caller's live value can span a
callee invocation, and ABI bytes intentionally pass values between routines.

The existing rule for active compiler temporaries and scratch remains in force:
ordinary source pointer access does not expose compiler implementation storage.
Explicit absolute storage, raw physical/assembly observations, ABI access and
escaped source objects retain their existing protection. This proposal grants
no new permission to remove, merge or reorder pointee reads.

Action locals can retain values between invocations. Only locals already proved
initialized before every read on every invocation qualify for the exception.
Other locals and ordinary inline source storage keep their current treatment.

## Shared facts, owned by MIR6502

Extend the existing ownership and physical-memory facilities with structured
records. The following are contracts, not prescribed public Rust types:

| Record | Required information |
| --- | --- |
| Storage object | Typed identity and byte offset; owning routine where applicable; current binding; complete emitted placement |
| Storage protection | Globally protected, or a verified invocation home with a current proof |
| Observation | Owning routine and generation-scoped site; read/address/ABI role; logical object/lane when certified; physical range and provenance |
| Proof dependencies | Bodies, bindings, exposure inputs, allocation, callee facts and layout representation used to justify the record |

Use existing IDs for parameters, locals, spills, virtual ZP, routines and blocks.
Names are diagnostic metadata. Reuse the existing generation/site and
representation-validity mechanisms; no trace record grants optimizer permission.
Unknown identity, range, role or stale evidence cannot authorize an exception.

MIR6502 owns these facts. Emission resolves their final ranges without erasing
their identity. SemIR/NIR contracts remain unchanged; MIR6502 must not consult
SemIR or source names to reconstruct ownership.

The declaration inventory stays complete, including unused objects, aliases,
descriptor backing and fixed declarations. Its unconditional protected-range
projection remains available to existing conservative consumers. A separate
contextual query evaluates all overlapping records for a particular store.
One qualifying object cannot mask another protected alias at the same address.

## Certificate for an invocation home

The shared binding verifier may produce evidence for a complete ZP-bound BYTE,
WORD or pointer object only when all of the following hold:

- The declaration and binding are complete and consistent. The object is not
  address-exposed, fixed, aliased, initializer-backed or externally observable
  through an entry/storage convention. Historical exposure evidence used by
  the selected pipeline remains a restriction.
- Every covered byte receives a genuine definition before every read in each
  invocation. Parameters use their actual ABI capture; locals use actual
  logical assignments. A physical scratch write or a callee's may-write cannot
  establish this property. Conditional/partial initialization and entry-value
  dependence fail the certificate.
- All expanded uses agree with the binding. Pointer pairs, absolute aliases,
  implicit scratch and call setup/result writes satisfy the existing logical
  lifetime and physical-interference proof.
- Current callee entry/write facts and the expanded body satisfy residence
  verification. Unsupported recursive/reentrant, opaque-entry and machine-body
  cases receive no new certificate. The existing execution/ABI model is not
  extended to interrupts or concurrent invocations.

Reuse the transfers and solver in
[home liveness](../src/mir6502/analysis/home_liveness.rs),
[resident lifetimes](../src/mir6502/parameter_homes/lifetimes.rs) and
[local initialization](../src/mir6502/parameter_homes/locals.rs). Extract shared
queries where needed; do not build a second dataflow engine or infer a certificate
merely because the selector installed a binding. Explicitly supplied MIR must
meet the same requirements as automatically selected MIR.

For `Ord.menu`, entry capture defines both bytes before use. Its own reads
therefore depend on that invocation's pointer value, not on an earlier
`MoveMenuBar` temporary stored at the same address. A later invocation again
captures fresh arguments. If a value is live across a nested call, the existing
residence proof must still establish preservation. An invalid overlapping
allocation cannot be justified retroactively by proposing a store deletion.

## Observation provenance

Classify each relevant access without losing its source:

| Observation | Treatment |
| --- | --- |
| Typed read of a certified resident object | Belongs to that object and invocation; still a real read within its lifetime |
| Fixed-ZP or pointer-pair read certified as consuming that resident value | Same treatment, with a site-specific witness |
| Explicit numeric, machine, hardware or structured physical-region read | Retains physical observation protection |
| Canonical argument, result or additional callee-entry input | Retains its ABI boundary role |
| Unclassified physical read or incomplete evidence | Conservative fallback |

The shared binding/lifetime analysis exposes certified read-site and lane
queries for the current immutable body. A typed logical read supplies its
identity directly. For an expanded fixed-ZP or pointer-pair read, require a
unique binding and reaching logical-definition evidence from the same routine.
Its numeric address or presence in `frame.fixed_zero_page` is insufficient.
Unattributed reads retain the old physical veto. Raw absolute and assembly reads
must not be relabeled merely because they coincide with a resident home.

These facts are proof metadata; executable MIR does not gain a second memory
identity. Rebuild them for another program snapshot. Do not remove resident
reads from general callee summaries: callers and residence verification still need those effects.
Only the contextual cleanup query may conclude that a particular foreign
resident read cannot consume the candidate temporary definition.

## Narrow deletion rule

The first consumer is terminal deletion of direct byte stores from a register
to declared virtual-ZP temporaries. The new exception covers only overlap with
certified resident objects owned by other routines. Existing spill/fixed-scratch
behavior remains available through its current rules; this proposal adds no
deletion of parameter/local stores, captures or result writes.

For each proposed deletion:

1. Establish the existing temporary ownership, non-exposure and exact placement.
   Check every overlapping source declaration. Any global protection, incomplete
   inventory, uncertified resident or same-routine source overlap keeps the
   existing veto.
2. For an overlap consisting solely of certified foreign invocation homes,
   permit the candidate to reach the existing structural store-deletion proof.
   An overlapping argument/result/entry-home role is excluded from this first
   relaxation, even if that byte also has a working-home binding. Derive ABI
   roles from typed contracts, not an address whitelist.
3. Use the existing reaching-definition proof for all paths, joins, backedges,
   calls and return boundaries. Ownership eligibility does not prove deadness.
   Real caller/callee inputs remain uses; unknown may-writes never kill a
   definition. No new call-graph scan or separate call-reader policy is added.
4. Evaluate physical observations with provenance. Ignore only the certified
   foreign object reads whose invocation definitions isolate them from this
   temporary. All other explicit/ABI/machine observations retain their vetoes.
   Opaque physical effects on a relevant path cannot be waived by this fact.
5. Replay the same proof against final candidate placement and preserve all
   earlier decisions and consumed callee contracts before accepting the change.

The contextual ownership and observation queries share one analysis snapshot.
Both global guards are refined together; neither is simply removed. Queries
must identify the candidate definition, rather than answer whether an address
is private for the whole program.

## Integration, invalidation and acceptance

Keep discovery in the existing terminal cleanup transaction. Retain frames,
bindings, reservations and virtual-ZP mappings. Do not add allocation search,
per-object rematerialization, save/restore, subset retries or a new cleanup pass.

Logical evidence is valid only for its current verified body and dependencies.
Physical evidence uses the transaction's authoritative emitted map. Instruction
deletion can move code and ordinary storage even with fixed frames. Validate
baseline and candidate maps, overlaps, observation roles and all recorded
preservation obligations using the existing replay boundary.

The implementation binds its certificate to an immutable expanded `MirProgram`.
A site is resolved only inside that borrowed baseline; pointer identity rejects
use with another program, including an identical clone. This fixes body, binding
and callee dependencies together. One certificate snapshot serves physical
discovery, delta validation and final-placement replay. Replay retains baseline
operation sites and resolves them against the candidate's emitted inventory.
Historical programs restrict exposure without adding their former allocations.
Candidate verification and preservation replay remain required after deletion.

Changing a body, binding, exposure input, allocation, entry contract or relevant
callee effect invalidates the corresponding evidence. A routine generation
alone does not refresh another routine's facts. Historical proof references
remain constraints; they are not additional simultaneously live allocations.
Missing facts or failed replay restore the selected input artifact.

Accept only strict complete-image savings. The agreed
[whole-image cost refinement](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-refinement-whole-image-cost-acceptance),
now implemented and [qualified](benchmarks/6502-whole-image-cost/README.md),
removes the separate per-routine-growth veto.
It does not change observation eligibility or deletion proofs. Preserve exact
rollback of MIR, bindings, maps, relocations and selected upstream strategies.
The historical 48-byte rejection and the later qualified 56-byte saving remain
recorded under the cost policy used for those measurements.

Leave `observable_reads` and `source_storage_disjoint` conservative for spill
pruning, byte movement and other address-sensitive consumers. Their permissions
would require separate qualification. Producer purity and register/flag
liveness rules are unchanged, including when normal cleanup cascades remove a
producer after a store disappears.

## Diagnostics and qualification

Report the store's typed home, owning routine/site and emitted address, plus
each blocking declaration or read and its provenance. Distinguish declaration
protection, uncertified resident overlap, ABI input, explicit physical read,
live definition, stale proof, replay failure and cost rejection. Diagnostics
must cover early eligibility rejection, which currently occurs before the
`late-cleanup-store-retained` read-alias event. Do not retain only an unexplained
address or infer legality from trace output. Tracing must not change selection.

Qualification must cover both vetoes independently and their composition:

| Case | Required outcome |
| --- | --- |
| Unrelated certified parameter at the temporary's byte, including an unused declaration | Declaration overlap alone does not retain a dead temporary store |
| Typed, expanded fixed-ZP and pointer-pair reads of that parameter | Certified foreign reads do not create a use of the temporary definition |
| Real temporary read in the owner, successor, backedge or call arguments | Store remains |
| Resident caller value live across a clobbering callee | Residence verification rejects the allocation; deletion is not its repair |
| Source absolute/global/alias at the same byte, even unused | Store keeps the conservative protection |
| Address-taking, data/machine relocation or explicit numeric read | No exception |
| Local initialized on all paths versus partial, conditional or persistent entry value | Only the complete per-invocation case qualifies |
| ABI lane, additional entry read, result role or ambiguous fixed-ZP provenance | No first-slice exception |
| Changed binding/body/callee summary, missing inventory or shifted final alias | Stale/invalid evidence is rejected; exact rollback |
| Spill pruning, movement and existing source-owned TN ZP controls | Existing protection remains |

Use independent controls before TN. VM oracles check results, call arguments,
stack balance, repeated invocations, source-memory sentinels and ordered pointee
reads; they must not require a deleted private temporary's final byte value.
Include word lanes, page crossings and arithmetic wrap where relevant.

Measure final code, inline storage, ZP use, packaged images and retained stores
for TN and independent controls. Compare with only the new behavior toggled;
record decisions and exact artifact hashes. Qualify cart/standalone, shifted
origins, raw/optimized NIR, traced/untraced equality, actual LF/CRLF fixture paths
and MADS reconstruction in broad CI/release qualification. At the user's later
request for fast iteration, the implementation plan scopes local validation to
focused proof/VM controls and one final TN comparison. Record the broader checks
that were deferred; do not describe the local run as full backend qualification.

The shared inventory should be built once per transaction snapshot and reused
across store candidates. Reuse dataflow analyses and bound rebuilds by the
existing rewrite loop. Record analysis work, compile time and memory: TN already
takes about 20 seconds with cross-call trials, so another materialization or
whole-program analysis per candidate is not an acceptable hidden cost.

## Exclusions and implementation direction

Keep source-variable DSE, general local promotion, inline-home relocation,
allocation coalescing, return-home coalescing, interrupt/reentrancy support and
general interprocedural alias analysis outside this note.

The implementation preserves the existing allocation and cleanup architecture.
`enable_scoped_storage_observation` is enabled by the optimized configuration and
disabled by the conservative default. Both `enable_peepholes` and
`enable_late_private_store_cleanup` remain parent gates. No special handling
of `Ord`, `Next`, `MoveMenuBar` or TN is permitted.
