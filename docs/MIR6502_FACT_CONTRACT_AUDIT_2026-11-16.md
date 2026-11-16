# MIR6502 fact-contract audit — 2026-11-16

The fact layer needs contract repair before more placement exceptions. The
problems occur mainly when one analysis projects another analysis's facts:
program reads become argument obligations, possible writes become definite
definitions, and selected instruction effects are interpreted differently by
different entry analyses. More information is not sufficient if its meaning
changes at those boundaries.

This audit identifies **two unsafe analysis contracts** and several confirmed
precision failures. It does **not** demonstrate a new end-to-end TN miscompile.
No production compiler behavior was changed.

Audited revision: `2c5ae7add99f42aeb22ed34fc068ee60f2c9ea80`.
Scope: MIR6502 operation effects, call summaries, entry requirements, home
liveness/definitions, call composition, residence consumers, and fact freshness.
The frontend, NIR semantics, other backends and external runtime implementations
were not comprehensively audited.

## Evidence and method

The diagnostic invokes the actual private analyses, rather than reconstructing
their decisions from emitted assembly. It inventories the selected executable
after the normal final-output selection. Independent source controls use
optimized NIR/MIR6502 with small-leaf inlining disabled so calls remain visible.
TN uses the normal optimized configuration. Both use ActionCart at `$3000`.

Evidence and reproduction are in
[6502-call-facts-audit](benchmarks/6502-call-facts-audit/README.md).
The source controls cover fixed-ZP versus ordinary-RAM state, byte/word
comparisons, integer/pointer signatures, global writes and an assembly leaf.
Synthetic probes isolate structured region projection, may-write liveness,
result composition and read/write directionality.

TN remains **9,811 packaged bytes**, byte-identical to the acyclic-call-homes
qualification image, SHA-256
`e27ee7cc259d1cdb49ce2b9b067e052e2aa04ffac959edcd7e37c707327aa291`.
This is an audit baseline, not evidence that fixing every blocker will save bytes.

## The contracts that currently overlap

| Fact | Producer | Required meaning | Important consumers |
| --- | --- | --- | --- |
| Canonical argument lanes | `abi::canonical_call_lanes`, frame ABI | Values this particular call must establish | Entry validation, handoff, residence |
| Actual entry dependencies | `known_callees/entry_inputs`, machine liveness | Incoming state read before a definite definition | Call composition, handoff, residence |
| Memory read footprint | `summarize_*_reads` | Complete conservative set of possible reads | Observation checks, deletion guards, replay |
| Memory write footprint | `summarize_*_writes` | Complete conservative set of possible writes | Value invalidation, ZP preservation, replay |
| Definite definitions | Operation classification, home transfers, `defined_results` | Written on every relevant execution path | Liveness kills, reaching definitions, entry dependency removal |
| Register/flag provenance | Machine-value analysis and exit summaries | Known value relationship at the applicable point | Reload/compare removal, forwarding |
| Physical address and ownership | Frame bindings, fixed backing, layout, storage observations | Which object owns a byte and who may explicitly observe it | Allocation, alias checks, private-store deletion |
| Fact validity | Entry snapshots; pass ordering and terminal replay | Facts still describe the body and placement being transformed | Every rewrite using a cached summary |

`Exact` read/write summaries mean a **complete may-effect footprint**, not that
every listed byte is accessed on every execution. A callee's output definition
requires a separate proof. Likewise, an incoming state dependency is not
necessarily a formal argument.

## 1. Unsafe: named memory-region offsets disappear

In [known_callees.rs](../src/mir6502/analysis/known_callees.rs), both
`summarize_structured_reads` and `summarize_structured_writes` construct
Global/Static/Local/Param bases with `offset: 0`, ignoring `region.offset`.
AbsoluteRange and ZeroPage retain their offsets.

The probe supplies a region starting at byte 3 with size 2:

| Region kind | Required footprint | Published footprint | `may_write_mem(byte 3)` |
| --- | --- | --- | --- |
| Global | bytes 3, 4 | bytes 0, 1 | true, because all globals conservatively alias |
| Static | bytes 3, 4 | bytes 0, 1 | **false** |
| Local | bytes 3, 4 | bytes 0, 1 | **false** |
| Param | bytes 3, 4 | bytes 0, 1 | **false** |
| AbsoluteRange | bytes 3, 4 | bytes 3, 4 | true |
| ZeroPage | bytes 3, 4 | bytes 3, 4 | true |

The input offset is retained by the NIR-to-MIR effect conversion in
[abi.rs](../src/mir6502/abi.rs). Other region consumers, including
[late_cleanup/pruning.rs](../src/mir6502/late_cleanup/pruning.rs), already use it.
This is an inconsistent projection, not a missing source alias proof.

The write query is consumed by
[machine_values.rs](../src/mir6502/analysis/machine_values.rs), including known-call
value invalidation. An incomplete footprint can preserve a stale memory-value
fact. Read under-approximation is also unsafe for observation-based deletion.
The probe demonstrates incorrect analysis answers; it does not establish a
source-to-executable failure through these paths.

**Repair boundary:** use one checked region-to-byte projection, preserving
identity, offset and extent. Unrepresentable ranges must fail conservatively.
Cover reads and writes, and a consumer that must invalidate byte 3.

## 2. Unsafe: a finite may-write becomes a definite overwrite

[effects.rs](../src/mir6502/analysis/effects.rs), `apply_structured_effects`, copies
ZeroPage regions from `effects.memory_writes` into `homes.writes`.
[home_liveness.rs](../src/mir6502/analysis/home_liveness.rs) uses that set as
definite kills. [home_definitions.rs](../src/mir6502/analysis/home_definitions.rs)
uses the same transfers to replace reaching definitions.

The isolated sequence is:

```text
store $E4, A
call f                  ; may write $E4; no guaranteed result at $E4
load A, $E4
```

A possible write does not eliminate the old value on the path where `f` leaves
the byte alone. The existing unknown-write path correctly retains that value.
The finite-region path incorrectly kills it. This also makes the old store
appear to have no remaining use to reaching-definition analysis.

| Declared call write footprint | Old value live after store | Old definition reported dead |
| --- | --- | --- |
| Unknown may-write | true | false |
| Finite may-write of `$E4` | **false** | **true** |

This is a fact-level correctness violation. The audit does not claim that the
current private-store deletion guards accept this particular fixed-ZP example,
or that a TN call currently exhibits a conditional write described this way.
Those restrictions can mask an unsafe underlying fact; they do not make it a
valid definition proof.

**Repair boundary:** keep may-write footprints separate from must-definitions
all the way through operation effects and home transfers. A finite set is no
more definite than an unknown set. Explicit stores, explicit call results and
independently proved all-path outputs may kill incoming values. Clobbers may not.

## 3. Precision defect: global state becomes an outgoing argument

[entry_inputs.rs](../src/mir6502/analysis/known_callees/entry_inputs.rs) projects
read-before-write accesses to ZP-backed globals into `Inputs.fixed`.
[handoff.rs](../src/mir6502/parameter_homes/handoff.rs), `inputs_supplied`, then
requires **every** such byte to occur in the outgoing canonical argument lanes.
[ResidenceCalls](../src/mir6502/parameter_homes/calls.rs) reuses that check.

Independent control:

```action
BYTE state=$E8
BYTE FUNC ReadState() RETURN(state)
BYTE FUNC Hold(BYTE keep)
  ReadState()
RETURN(keep+1)
```

`ReadState()` has no arguments. Its read footprint correctly names `state`.
Its entry fact additionally lists `$E8`, and the residence input check returns
`extra-entry-fixed-byte`. Moving the same global to `$0600` makes that check
pass. Its role as program state has not changed.

TN's `ArenaCursor()` similarly has no arguments and reads `allocp` at `$E8/$E9`.
Those bytes are existing state. Its composed fact lists them as fixed inputs;
the caller-supply interpretation is wrong. Currently a separate flag rejection
is encountered first, so fixing carry alone will not unblock it.

**Repair boundary:** distinguish canonical ABI inputs, existing program-memory
dependencies, and additional physical entry requirements. Preserve global reads
in memory/observation facts and continue reserving source-owned ZP. Do not simply
discard all non-argument fixed-byte dependencies: assembly and implicit scratch
inputs still require a contract.

## 4. Precision defect: entry-flag analyses disagree

There are several answers to “does this routine need incoming carry?”:

- `scalar_entry_ignores_flags` uses ordinary machine liveness and an integer-only
  signature gate.
- `MirKnownCalleeEntryFlags` uses the same signature gate but a narrower
  definite-flag transfer.
- `MirCalleeEntryFact` accepts the broader direct parameter ABI and uses that
  narrow transfer, optionally with composed call dependencies.

In [machine_liveness.rs](../src/mir6502/analysis/machine_liveness.rs),
`entry_flag_transfer` replaces flag definitions with a selected-form whitelist.
It supports some indirect/memory comparisons but omits an ordinary immediate
unsigned relational comparison.

For `IF x<40`, the emitted code starts:

```asm
CMP #$28
BCS false_arm
```

The central operation classifier says C/N/Z are defined, and the legacy
`scalar_entry_ignores_flags` result is true. The precise entry and composed
entry analyses nevertheless require incoming C. CMP establishes the carry
actually used by BCS; this requirement is spurious. The word comparison control
also exercises the expanded byte comparisons.

`ArenaCursor` contains emitted CMP instructions before its carry tests, but its
composed contract requires C. That requirement propagates into `Alloc`, `Free`,
`Push` and `Pull`.

**Repair boundary:** put guaranteed definitions of selected instruction forms
in one shared contract. Do not substitute broad may-clobbers for definite writes:
folded comparisons, self-moves and recipes which reuse flags remain distinct.
Entry consumers should agree when they are answering the same question on the
same body. Different analysis scope should be explicit.

## 5. Precision gap: result definitions depend on parameter types

[lower.rs](../src/mir6502/lower.rs) preserves a direct parameter ABI which
supports pointer parameters, but the older `MirScalarSignature` is integer-only.
The former does not retain a corresponding direct result contract.
[acyclic::defined_results](../src/mir6502/analysis/known_callees/entry_inputs/acyclic.rs)
requires the latter before proving the actual return-home writes.

Consequently, a byte-returning function with a pointer parameter cannot publish
the same definite `$A0` result as the equivalent function accepting a CARD.
The two source controls emit identical `Fetch` bodies. A synthetic caller with
an explicit post-call `$A0` load isolates the composition difference while
keeping the body and physical argument lanes the same.

In TN, `Next` has no scalar signature because of its pointer interface. Its
entry inputs are correctly A/X, and its may-write footprint correctly includes
`$A0/$A1`. The result proof does not certify those output definitions. `Ord` and
`Items` therefore inherit an extra `$A0` entry dependency. For `Ord`, `$A3` is a
real argument lane; `$A0` is the spurious additional requirement. Calls from
`PopUp` fail with `extra-entry-fixed-byte`.

This is a deliberately narrow proof whose backing representation is now
insufficient for its broader consumers. It is conservative, not a wrong-code
claim.

**Repair boundary:** retain the typed direct result ABI in MIR and prove its
definitions on every returning path. Do not infer results from arbitrary scratch
stores or expand the old integer-specialization signature's meaning silently.
MIR must not look back into SemIR to recover missing result semantics.

## 6. Precision limitation: named global writes exclude all ZP

`mem_may_alias_fixed_slot` in
[known_callees.rs](../src/mir6502/analysis/known_callees.rs) treats every Global as
possibly aliasing every fixed ZP byte. It has no program or layout context.
Thus a routine whose exact footprint writes a global fixed at `$0600` reports
that it does not preserve `$E4`. Cross-call residence derives its forbidden
bytes from this query, excluding the entire pool for a value live across that
call.

TN's `ArenaStop` writes `ArenaFault` in ordinary global storage. Its write
identity propagates into arena callers, so repairing their inputs still leaves
a preservation obstacle.

**Repair boundary:** use known backing and authoritative placement to answer
physical overlap. The existing
[fixed_storage_address](../src/mir6502/analysis/known_callees/observations.rs)
already resolves explicit backing and alias chains conservatively. Ordinary
unplaced objects still need layout evidence. Do not treat every named global
as automatically disjoint from ZP.

## 7. Conservative restrictions that should not be confused with bad facts

### Assembly leaf knowledge does not compose

The bounded machine entry analyzer can certify some assembly leaves, and the
machine-summary path can publish bounded writes and exit provenance. However,
acyclic composition rejects every MachineBlock, including a leaf with an
existing entry certificate. Residence separately rejects these bodies.

The independent `asm_leaf` control has a known A/X entry, known writes and no
extra fixed inputs, but no acyclic entry. TN's `MovePage` also has a leaf entry
certificate and no composed entry. `CalcAdr`, `Internal` and `Ascii` have useful
write summaries but no complete leaf entry certificate in this inventory.
Their situation is not equivalent to the certified assembly control.

These are explicit supported-subset limits. Future composition should reuse a
certified leaf contract where available; manual clobbers alone do not prove
entry reads, definite results or all control-flow behavior. Cycles, unresolved
external transfers and genuinely opaque bodies must continue to fail closed.

### Unknown writes still contaminate read precision

`summarize_op_reads` tests `memory.has_unknown_effects` as well as read-specific
fields. A StoreIndirect has bounded pointer-byte reads and an unknown pointee
write, but publishes Unknown reads. The probe confirms the directional loss.
The reverse problem, unknown reads contaminating write summaries, was addressed
by earlier work; this remaining direction must not be described as that same
unfixed bug.

Unknown pointee reads themselves are often correct. They must not automatically
make compiler-private homes observable. Existing ownership and explicit-address
observation rules remain necessary. This audit does not propose complete alias
analysis to make every read footprint finite.

## TN impact at the audited revision

The selected program contains 95 routines. The independent entry analyses
publish 10 leaf certificates and 16 acyclic certificates; these sets overlap
and are not additive. Read footprints are finite for 11 routines, write
footprints for 27. Unknown footprints are not, by themselves, defects.

The 356 direct MIR call sites have these residence-input outcomes:

| Outcome | Call sites |
| --- | ---: |
| Input check passes | 177 |
| Unsupported residence callee | 122 |
| Unknown residence callee entry | 36 |
| Extra entry flags or stack | 16 |
| Extra entry fixed byte | 5 |

These are input-check outcomes, not counts of legal or profitable placements.
They exclude calls encoded inside assembly and do not establish that every
passing call preserves any particular byte.

Examples of overlapping blockers:

- `ArenaCursor` and its callers: false carry, global-state bytes interpreted as
  arguments, then conservative global-write overlap.
- `Ord`/`Items` as callees: unproved typed result definitions propagate `$A0`.
- `PopUp`: the above native blockers plus unsupported machine dependencies in
  its wider call closure. No single correction proves the whole routine.
- `Sort`: its composed entry is known and has no extra inputs; its write summary
  is Unknown. Its indirect stores are a real reason for conservative writes.
- `Copy`: several unsupported/unknown dependencies remain; it is not evidence
  that an individual native entry repair is ineffective.

## Freshness, placement and safeguards to retain

There are already useful protections:

- Entry facts snapshot their source body, analysis stage, machine payloads,
  global backing and transitive dependencies. `is_current` validates those
  dependencies. The acyclic cache borrows one immutable program and rejects
  cycles.
- Every composed call validates canonical argument lanes. Call-site validation
  is not replaced by a once-per-callee check.
- Residence distinguishes logical source values from physical scratch writes;
  clobbering a resident byte does not initialize the caller's logical value.
- Materialization allocates virtual ZP before publishing physical preservation
  summaries, then rebuilds summaries after later ZP remapping.
- The terminal transaction rebuilds and checks callee contracts, replays
  preservation, verifies placement and retains final-image cost acceptance and
  rollback.

The weaker area is the general `MirKnownCalleeSummaries` API: summaries are keyed
by RoutineId without an embedded body revision. Materialization constructs a
snapshot and then rewrites routines against it. Validity relies on those groups
preserving the published contract. Terminal replay is more explicit than this
earlier phase discipline. This audit found no separate stale-summary witness;
record this as an invariant to clarify, not another confirmed compiler bug.

Physical overlap queries should say whether they use fixed backing, proposed
placement or final layout. Moving an object or changing an instruction recipe
can invalidate physical footprints or guaranteed flag definitions even if the
source-level operation remains equivalent.

## Recommended repair order

1. **Repair unsafe projections:** retain region offsets and separate finite
   may-writes from definite home definitions. Add focused fact and consumer
   regressions before widening any optimizer's admission rules.
2. **Repair entry contract meaning:** separate caller-supplied ABI values from
   existing state; preserve typed direct results; unify guaranteed selected-form
   flag definitions. Cover both positive cases and missing real inputs/results.
3. **Repair physical preservation queries:** use backing/placement evidence,
   preserving ownership and unknown-alias fallbacks. Then consider composition
   of already-certified machine leaves.
4. **Make validity explicit and requalify:** document which transformations
   preserve each published fact, rebuild where they do not, and measure TN plus
   independent controls through the existing final transaction.

This can remain a sequence of small MIR6502 slices. It does not require a new
general effect framework, whole-program alias analysis, or another parallel set
of placement-specific facts. The important change is to make the existing
producers and consumers agree on meaning, scope and lifetime.
