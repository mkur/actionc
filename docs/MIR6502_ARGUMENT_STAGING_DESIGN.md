# MIR6502 argument staging reduction

Status: proposed design, 2026-11-11. Starting compiler: `476cfecf`, after
qualified direct comparisons and carry reuse. This note defines the contract
for extending existing call-argument selection; implementation remains separate.

Prepare call arguments in their final ABI locations when a proven schedule
can preserve them there. Retain protective captures whenever later evaluation,
physical aliases or observable memory effects require them. The goal is fewer
intermediate stores and reloads with the same argument values, ordered source
accesses, call behavior and public ABI.

## Evidence and existing behavior

TN's [Strcat](../samples/tn/modern/TN.ACT) contains:

```action
MovePage(s+s^+1, t+1, t^)
s^==+t^
```

With modern MIR6502 and optimized NIR, its current 125-byte body stages the
computed destination in `$A0/$A1`, the source in `$A2/$A3`, and the length in
`$A4`. It reloads A/X/Y immediately
before `MovePage`, then rebuilds the original pointers for the following
length update. The [frozen callable scorecard](../surveys/tn/c-oscar64/kernels.json)
records a 51-byte body and a 111-cycle advantage in two callable cases.
Those are whole-body differences, including pointer handling and ABI choices;
they do not establish the saving available from argument staging alone.

The [canonical Action ABI](../src/mir6502/abi.rs) places the first three
argument bytes in A, X and Y, followed by `$A3`, `$A4`, and subsequent bytes.
For `MovePage(CARD dst,src BYTE len)`, the destinations are:

| Argument | Low byte | High byte |
| --- | --- | --- |
| `dst` | A | X |
| `src` | Y | `$A3` |
| `len` | `$A4` | — |

Caller-side `$A0/$A1/$A2` copies are staging in this sequence, rather than
required ABI mirrors. These are nevertheless physical, potentially observable
cells. `MovePage` initializes its own `$A0/$A1/$A2` cells on entry. Its body
also reads and writes indirectly, so its preservation annotations alone do
not establish that earlier staging stores are unobservable.

Existing [call plans](../src/mir6502/call_plan.rs) already assign typed values
to canonical homes. [Call materialization](../src/mir6502/materialize/calls.rs)
has direct indexed-word, arithmetic, loaded-argument and result-forwarding
paths as well as protected Action staging. The
[rewrite pilots](../src/mir6502/rewrite/pilots.rs) already prove producer
definition identity and deadness. Extend those paths and shared proofs.

## Ownership and scope

SemIR owns Action! evaluation and call semantics. Verified NIR supplies typed
captures, reads, computation, call targets, signatures, storage identities and
conservative effects. MIR6502 selects registers, pointer carriers, argument
copies and their schedule. Emission writes the selected sequence and supplies
final physical layout for qualification. Selection consumes typed MIR facts;
it does not consult SemIR, printed IR, source names or TN addresses.

The first implementation admits one same-block producer/call region, a direct
call, BYTE and complete WORD arguments, and canonical layouts of up to five
argument bytes. Begin with already captured values, constants, relocatable
addresses and ordinary direct scalar sources whose existing read points can
be retained. Use a bounded set of schedules extending the existing selectors.
Keep source reads and producer order fixed in the first slice.

Indirect/indexed producers qualify only through existing supported consumers
or a subsequent slice proving their complete address-setup effects and ordered
reads. TN's unrestricted `Strcat` pointers may therefore retain staging until
those stronger facts are available. A neutral case must improve independently.

Indirect callees, variadic and aggregate calls, wider scalar layouts, tail
calls, CFG-wide scheduling, loop motion and scheduling across nested calls
retain their current paths initially. No new NIR form or ABI is proposed.
If existing MIR operations cannot describe a selected schedule's inputs and
clobbers exactly, document and verify the required MIR form before admitting it.

## Separate proof obligations

Three transformations require different evidence:

| Transformation | Required proof |
| --- | --- |
| Keep a captured byte in its final register | The same complete definition reaches the call; all intervening setup preserves it |
| Replace a capture with a later source-home read | The later read equals the original capture and preserves observable access order/count; excluded from the first slice |
| Delete a staging store or home | Every possible observer is accounted for, including physical aliases and callee reads before overwrite |

A logical temp with one user does not prove its assigned home unobservable.
A preserved pointer pair does not prove its source pointer cells unchanged.
A callee's register-preservation summary does not prove that caller-side
argument preparation preserves those registers.

## Scheduling contract

At the call, every argument byte must equal the byte supplied by the baseline
evaluation, in the same canonical home. The selected schedule must establish
all of the following:

- Each removed definition has complete lane provenance and no remaining
  reader in the routine, successors, backedges or edge arguments. Captures
  with other uses remain available to those uses.
- Original source reads, writes, calls and faults keep their execution order
  and count. Folding arithmetic over captured values grants no permission to
  move a pointer, absolute or hardware read. Aggregate effect equality alone
  does not prove this order.
- Each step declares its register, individual flag, pointer-pair and memory
  inputs and writes. A later indexed load can overwrite Y; word arithmetic
  can overwrite A/X and carry. Establish Y offsets explicitly and preserve
  complete low/high lanes and carry chains.
- Once an argument byte occupies its final register, every later step either
  preserves it or saves/restores the exact captured byte. Reading a source
  home again requires a separate equality proof.
- Memory placement behaves as a dependency-respecting parallel copy. A write
  cannot overwrite an unread argument source, pointer byte or callee target.
  Cycles and partial overlaps keep protective staging unless the existing
  overlap-safe transfer handles them.
- Changed incidental A/X/Y/flag state is dead or explicitly repaired wherever
  it remains observable, including additional machine-callee entry inputs and
  the caller's post-call paths. Keep the call target, arguments, results,
  return homes, clobbers, stack balance and call count unchanged.

For example, a complete captured word can remain in A/X while a later ordinary
byte load initializes Y, if that load preserves A/X and its original read point.
It cannot remain there through an arbitrary later pointer calculation.
In `Strcat`, computing `t+1` needs A and reading `t^` needs address/Y setup;
deleting the destination save without a new proven schedule loses its value.

Use deterministic templates with a fixed analysis/search budget. Exhaustion,
unsupported effects or failed verification selects the existing staged path.
No optimization failure should reject an otherwise valid program.

## Physical observation and call effects

Resolve aliases using final placement, including named and absolute accesses,
both bytes of parameter homes, fixed/allocated zero page, `$A0..$AF`, public
return slots, callee prologues, helpers and stack staging. Incoming pointers
can point into those cells or the caller's own frame. Frame/address exposure
blocks store or home deletion unless an existing bounded proof excludes every
relevant access. Compiler-created storage alone is not a privacy guarantee.

Known calls use the existing transitive
[callee summaries](../src/mir6502/analysis/known_callees.rs). Resolve each named
memory byte in its owning routine; an unqualified callee-local or parameter ID
cannot stand for caller storage. Missing ownership or incoming-read-before-
overwrite facts must remain blockers or be added to the shared analysis in a
focused slice. Unknown, opaque or recursive behavior supplies no preservation
or non-observation proof. Do not infer those facts from a procedure name or an
informal assembly annotation. Bounded machine analysis must cover every
admitted path and preserve its current conservative fallback.

Actual callee clobbers matter for observations at entry and values retained
across the call. Pre-call argument scheduling must instead use the effects of
each setup operation. Reusing original pointers after `MovePage` belongs to
the existing [pointer-retention work](MIR6502_POINTER_RETENTION_PLAN.md),
with its separate source-equality and physical-write proofs. Qualification
must isolate staging savings from any pointer-retention improvement.

## Integration and profitability

Extend typed expression/call planning before generic byte expansion and home
allocation, while producer identities and source-read order remain explicit.
Reuse immutable pre-home CFG/use-def/reaching-definition contexts and the
transaction driver. Schedule facts identify the routine, block, call site,
argument byte, defining lane, final home, protected carriers and snapshot
generation. Reconstruct them on application; stale plans cannot commit.
Post-home register-copy cleanup continues through its existing shared path.
Reuse the [scalar capture cleanup](MIR6502_SCALAR_CALL_ELISION_PLAN.md)
observation proofs for store/home deletion and the existing
[predicate-argument proofs](../surveys/tn/call-predicate-implementation-plan.md)
where applicable. This extension does not replace those independently qualified
families or introduce another alias scanner in the call materializer.

Use the current explicit call-materialization effect contract and verify its
requirements. That contract permits selected ABI setup; it does not waive
ordered-read or store-observation proofs. Any additional effect projection
needs a narrow definition, independent validation and focused negative tests.

Gate the extension independently inside optimized peepholes, preserving a
feature-off control. Rank schedules cheaply, then use existing final code/home
and image costing after normal materialization, allocation and branch repair.
Require no growth in affected routine code, ordinary homes, final XEX size or
occupied zero page, plus a measured byte or cycle improvement. Restore rejected
routines and recheck combined placement and alias proofs. Cost/proof failure
retains the previous valid materialization and prior accepted optimizations.

Report candidates, final applications and useful blockers: later-register or
flag clobber, ordered read, extra use, source/ABI overlap, observable store,
unknown callee, unsupported shape and final cost. Reporting must preserve the
selected image. New schedules must retain better existing indexed/arithmetic
selectors; the [direct-comparison qualification](benchmarks/6502-direct-comparisons/README.md)
demonstrates why legal selection alone cannot decide profitability.

## Qualification and acceptance

Freeze fresh raw/materialized MIR, assembly, image hashes, routine sizes and
storage extents from the starting compiler before implementation. Record the
already-qualified subsets so each new saving has an identifiable cause.

| Layer | Required observations |
| --- | --- |
| Proof/schedule | Every supported byte/word home, repeated values, mixed widths, dominating definitions, complete lanes, extra/edge uses, copy overlap/cycles, stale generations and rollback |
| Machine state | Later setup clobbering A/X/Y or flags, word carry/borrow boundaries, nonzero high bytes, explicit Y offsets and unchanged required entry/exit state |
| Effects/aliases | Ordered exactly-once source accesses; pointer targets at argument, return, scratch and parameter homes; exposed frames; hardware reads; nested calls and opaque/recursive callees keeping fallback |
| Execution | Independent argument-capture callees and host oracles; repeated calls, unchanged inputs, stack canaries, both runtimes, raw/optimized NIR and actual LF/CRLF parsing |
| Existing selectors | Direct/indexed byte and word calls, arithmetic arguments, nested-call preservation, predicate arguments, result forwarding, indirect calls and protected field transfers |
| Application/cost | Neutral fixtures, TN/TNDBG and representative library/graphics calls; feature-off/on code, XEX, homes, zero-page occupancy, target cycles and applied/blocked sites |

The [nested-call](bugs/CLASSIC_NESTED_CALL_ARGUMENT_BUG.md) and
[deferred-first-argument](bugs/CLASSIC_DEFERRED_FIRST_ARGUMENT_BUG.md)
regressions document relevant failure modes. Borrow their independent oracles
for MIR6502 without broadening into classic-backend changes. Preserve the
alias-safe all-reads-before-writes behavior of existing protected field transfers.

Run focused selector/proof tests during implementation, then the affected
MIR6502 unit, integration, emission/load sweep and call/runtime targets under
[AGENTS.md](../AGENTS.md). Include TN execution rather than listing size alone.
Any shared semantic/NIR change requires its separately mandated checks.

Acceptance requires a smaller or faster neutral call with no image growth,
correct argument bytes and observable state for every supported schedule,
explicit fallback for rejected cases, and truthful TN/TNDBG measurements.
An unchanged TN result is acceptable only with the remaining blocker recorded.
The implementation plan should split baseline/proofs, one bounded schedule,
alias/effect qualification and final corpus measurement into separate commits.
