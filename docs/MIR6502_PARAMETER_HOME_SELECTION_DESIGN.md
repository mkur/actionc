# MIR6502 parameter home selection

Status: leaf placement and terminal parameter handoff are implemented and qualified.
Enabled for optimized builds; the conservative configuration remains disabled.
The [entry-to-call handoff extension](#proposed-extension-parameter-handoff-to-call-arguments)
below has its own gate, subordinate to parameter placement and peepholes.
The [callee capture boundary refinement](#proposed-refinement-callee-entry-facts-through-parameter-capture)
is also implemented and qualified. The
[ordinary-call and local-home extension](#proposed-extension-zp-homes-across-ordinary-calls)
is implemented and qualified. The implemented
[storage-observation refinement](MIR6502_STORAGE_OBSERVATION_DESIGN.md) preserves
routine ownership through terminal cleanup. The
[multiple-leaf-call extension](#proposed-extension-multiple-calls-to-leaf-routines)
is implemented and [qualified](benchmarks/6502-multiple-leaf-calls/README.md)
using the existing ordinary-call batch. The
[whole-image cost revision](#proposed-refinement-whole-image-cost-acceptance)
is implemented and [qualified](benchmarks/6502-whole-image-cost/README.md).
Compiler baseline `8626beff` applied the per-routine veto.

The [broader-placement extension](#proposed-extension-zp-placement-through-acyclic-call-chains)
is implemented and [qualified](benchmarks/6502-acyclic-call-homes/README.md).
It extends the same home-selection authority
through acyclic native call chains, retaining per-object call-site lifetimes and
native `=*` actual-entry analysis. Entry-contract composition and coordinated
placement use the existing single ordinary-call batch.

The [certified assembly-leaf extension](#proposed-extension-certified-assembly-leaves-in-acyclic-call-chains)
is proposed for fact-repair issue 9. It reuses existing leaf certificates at
native call boundaries; implementation and qualification are pending.

The completed [leaf implementation plan](MIR6502_PARAMETER_HOME_SELECTION_IMPLEMENTATION_PLAN.md)
covers the leaf scope. The [handoff implementation plan](MIR6502_PARAMETER_ARGUMENT_HANDOFF_IMPLEMENTATION_PLAN.md)
records the completed slices and qualification gates for the extension.
The [capture implementation plan](MIR6502_CALLEE_ENTRY_CAPTURE_IMPLEMENTATION_PLAN.md)
records the completed entry-fact refinement.

## Decision

Choose a parameter's working home before generating its entry capture and
home-dependent pointer setup. A shared, typed binding must govern both decisions
and ordinary storage layout. MIR6502 owns this policy; Action semantics, verified
NIR, and the public calling convention remain unchanged.

The first optimization scope is ordinary internal Action leaf routines with
non-exposed BYTE/WORD scalar parameters, including mutable pointer parameters,
and at most four incoming argument bytes. Keep a parameter in its incoming
register when its complete live range permits it; otherwise consider a reserved
zero-page home before retaining ordinary inline storage. Unsupported routines
continue through the existing path.

This addresses the coordination problem described in the
[pipeline audit](MIR6502_OPTIMIZATION_PIPELINE_AUDIT.md): capture, forwarding,
pointer preparation, and storage deletion currently make overlapping decisions
about the same parameter. A new cleanup pass after those decisions would leave
that problem intact.

## Baseline behavior and motivating example

[Core materialization](../src/mir6502/materialize.rs) reserves pointer
scratch, allocates virtual ZP, prepends the ABI parameter prologue, and constructs
an initial layout. Later passes forward incoming registers, elide write-only
homes, or coalesce a narrow leaf word parameter with the result pair.

On the legacy path, the [prologue and parameter passes](../src/mir6502/materialize/abi.rs)
capture arguments into inline parameter storage. General ZP promotion targets
spills, not these parameter objects. Furthermore,
[parameter availability](../src/mir6502/analysis/param_availability.rs) learns
register/home equality from capture stores; legacy entry starts with no such facts.
Removing the captures without changing that analysis would lose useful facts.

The current [Fnamecmp source](../samples/tn/modern/TN.ACT) illustrates the cost:
two mutable pointer parameters become four inline bytes, repeatedly copied into
pointer pairs. Measurements at revision `8039b30b`, MIR6502/ActionCart, origin
`$3000`, gave:

| Variant | Routine code | Inline parameter bytes |
| --- | ---: | ---: |
| Baseline compiler at `8039b30b` | 185 | 4 |
| Handwritten, working pairs `$A0/$A1` and `$A2/$A3` | 47 | 0 |
| Handwritten, working pairs `$AE/$AF` and `$AC/$AD` | 51 | 0 |

The handwritten variants also use shared Y induction, comparison flags, fewer
pointee reads, and shared return tails. They are direction-setting examples,
**not placement-only size targets**. Their prior VM comparison covered ordinary
RAM buffers, byte comparisons, colon handling, page crossings, and Y rollover;
it did not establish arbitrary volatile-read or callee-preservation equivalence.
The first slice must report its own emitted savings.

## Parameter identity and location roles

Retain parameter identity as `(RoutineId, ParamId, byte offset)`. Its source
name remains diagnostic metadata. A parameter holding a pointer is distinct
from the pointee, and mutation creates a new value of the same parameter.

Each routine has one parameter binding table. Its contract distinguishes the
following roles; the ABI and call plans supply the boundary locations:

| Component | Contract |
| --- | --- |
| Incoming ABI locations | Canonical byte positions supplied by the caller; immutable description of entry. |
| Working home | Inline storage, register lane(s), reserved ZP byte/pair, or no home when all accesses are proven unnecessary. |
| Outgoing call argument locations | Required locations at a particular call, supplied by its typed call plan; terminal handoff can share them with working homes. |
| Outgoing result locations | Existing result ABI, with its own lifetime and required writes. |

The [canonical ABI](../src/mir6502/abi.rs) supplies argument bytes 0–2 in A/X/Y
and byte 3 in `$A3`. There are no incoming `$A0–$A2` mirrors. Thus Fnamecmp enters
with `s` in A/X and `t` in Y/`$A3`. Capturing into `$A0/$A1` and `$A2/$A3` needs
three stores; the fourth byte already occupies its proposed home. This is legal
only if all four working bytes survive until their last uses. The result may
overwrite `$A0` afterwards.

The binding describes placement, not a second set of parameter values. Every
read and mutation must use the selected working home. Additional register copies
are availability facts governed by normal invalidation, not competing homes
that require permanent synchronization.

Before home materialization, typed parameter operands retain logical identity.
Materialization resolves them through the binding: a register read becomes a
register operation; a memory read resolves to its bound storage. Keep the typed
binding available for effects, maps, and verification after that conversion.
Do not scatter rewrites to anonymous fixed addresses or introduce separate
`ParamInX`/`ParamInZp` identity variants.

For migrated routines this separates identity from the current
`Param`/`ParamAbiOnly` allocation distinction. Only an inline binding allocates
ordinary frame bytes. A register binding has no address; a ZP binding has exactly
its reserved address, with no shadow inline cell. Retain legacy handling for
unmigrated routines until their consumers move to the shared contract. For a
migrated routine the binding table is authoritative: verification rejects a
missing binding or conflicting legacy allocation metadata.

## Selection boundary and lifetime rules

Select bindings on a verified, resolved pre-capture MIR snapshot, after the
chosen inlining and pre-home strategies. Put this boundary before the current
scratch reservation/allocation, ABI prologue, and initial layout sequence.
Selection and scratch reservation must be coordinated, not run as independent
allocators that can claim the same byte.

The snapshot still contains pseudo operations. Eligibility therefore requires
structured constraints for their eventual register, flag, scratch, and helper
effects. Source-level leaf status alone is insufficient: an operation that may
lower to a helper is ineligible in the first slice. Unsupported expansion or
incomplete constraints cause fallback, not guessed preservation. Validate the
actual expanded body against the binding before accepting it.

Initial bindings choose one working location for each live parameter lane;
they do not split live ranges or insert deferred spills. Register placement
requires valid definitions and no interfering clobber on every path through
the entire live range, including loop backedges and joins. If that cannot be
proved, select memory. Schedule entry transfers as parallel copies so capturing
one byte cannot destroy another incoming byte. Account for flags as well as
A/X/Y; an eliminated load does not make its flag outputs dead.

Pointer homes used by indirect addressing are coherent adjacent ZP pairs. A
parameter update changes the complete pointer with the original word arithmetic
and carry semantics. Pointer setup may consume the selected pair directly, but
must not advance or overwrite it as temporary scratch unless that implements
the corresponding parameter mutation. A nonzero index is not permission to
rebase the parameter permanently.

Use the existing reservation/allocation machinery for all contenders. Its
general virtual pool is currently `$E0–$EF`; ABI/result bytes and `$AA–$AF`
have additional fixed roles, described in
[runtime helper effects](RUNTIME_HELPER_EFFECTS.md). These are lifetime-dependent
roles, not universally free or permanently private address ranges. A live
parameter reservation must constrain every implicit scratch user as well as
explicit instructions. Start with conservative whole-live-range reservations;
do not add a general allocator or opportunistic cross-call sharing.

Source-owned ZP exclusions take precedence. In particular, TN's `$E0/$E1` and
`$E6–$E9` cannot become parameter storage. A requested pair that overlaps a
source reservation, fixed operation requirement, another live binding, or an
early result write is unavailable. Resource exhaustion selects inline storage.

## Exposure, effects, and verifier guarantees

Taking the address of a parameter cell, including through typed relocations or
machine references, exposes its storage and retains its existing home in the
first slice. Dereferencing the pointer *value* held by a parameter does not by
itself expose the parameter cell. Apply the established storage/effect contract
to unknown accesses; never infer privacy from an address being in ZP. Once a
binding establishes compiler-owned working storage, ordinary pointers follow
the existing non-observation rule for active compiler temporaries and scratch.
Explicit source ownership and ABI/assembly observations still take precedence.

Do not globally change all parameters from `ProgramOrAbi` to `Temporary`.
Relocation eligibility is a per-object proof, and relocation does not itself
authorize deleting source reads. Preserve pointee reads, their ordering and
their observable behavior. Initially reject indirect writes and opaque memory
effects that could invalidate the proposed residence; hardware/absolute access
does not acquire ordinary-memory semantics through a new home.

Before accepting a binding, verification must establish:

- ABI width and byte order are unchanged, and every use is reached by the
  correct entry transfer or parameter definition.
- Every executable parameter access resolves through one valid binding;
  register-bound or absent parameters have no memory/address references.
- No exposed object loses or changes its required physical home.
- Register clobbers, ZP overlap, pair coherence, and result handoff satisfy the
  binding on every reachable path, including implicit expansion effects.
- Planning layout, final emission, maps, and effect analysis agree on physical
  storage. No removed inline allocation remains as a fictitious address.

Seed entry register availability from the canonical ABI and binding, then apply
the actual entry transfers. Later mutations invalidate equality with the old
incoming value. Facts at CFG joins survive only when all incoming paths agree.

## Consolidating existing mechanisms

| Existing mechanism | Responsibility after migration |
| --- | --- |
| ABI prologue builder | Emit only transfers required by the selected bindings. |
| Parameter consumer/reload forwarding | Consume binding and availability facts; do not require a dummy capture store. |
| Write-only parameter-home elision | Supply the shared no-home decision; stop independently changing migrated frames later. |
| Leaf word-parameter/result coalescing | Become a placement candidate with a checked lifetime handoff; bypass the separate remapper for migrated routines. |
| Pointer preparation | Use a resident pair when valid and honor its reservation in all other preparations. |
| ZP allocation and storage ownership | Consume binding constraints alongside existing source and scratch reservations. |
| Planning/final layout and maps | Allocate inline bindings only; report register/ZP bindings without inventing ordinary storage. |
| Late cleanup | Remove ordinary dead work while preserving the chosen bindings; do not remap parameters. |

The existing block-local pointer-scratch ownership heuristic does not establish
a parameter's residence across blocks. Any consumer needing that fact must use
the explicit binding and its lifetime proof.

This work does not remove the retired-private-capture support in
[terminal cleanup](MIR6502_LATE_CLEANUP_DESIGN.md). That supports a different
late transformation. New parameter bindings must not require retired parameter
addresses or additional historical address maps to compensate for late remaps.

## Cost, rollback, and preservation

Expose the selected pre-capture snapshot from orchestration so the baseline and
one deterministic batch of proposed bindings can use the same chosen upstream
strategies. Resume materialization from that snapshot; do not recursively rerun
inlining and every enclosing pre-home cost gate for each parameter.

The first selector uses at most one batch trial against the complete fallback,
with no subset search or iterative placement repair. Reuse the existing emitted
cost and preservation checks. Under the
[revised profitability contract](#proposed-refinement-whole-image-cost-acceptance),
require a strict complete-image saving; individual routine growth is diagnostic.
Include final layout, branch relaxation, runtime helpers, and packaging. Unknown
cost, failed verification, reservation conflicts, or invalidated proofs reject
the batch and retain the fallback intact.

Freeze bindings before layout-sensitive downstream rewrites. Build callee
register/flag/fixed-memory summaries from each materialized candidate, and run
the downstream preservation checks against that candidate's placement. If an
upstream accepted caller rewrite already consumed a changed callee contract,
prove that contract still holds or reject the candidate; recomputing a summary
alone does not repair the caller. Do not reuse baseline physical witnesses with
new addresses. A later transformation needing a different binding must fall
back rather than silently changing the plan.

This requires an explicit orchestration boundary. Trace output should show
selected homes, rejection reasons, and final code/home/image deltas. Trace
enablement must not affect selection.

## First implementation and qualification boundaries

First establish the shared binding representation with current inline behavior.
Then admit bounded register residence and ZP placement for the leaf scope above,
including ordinary mutable pointer parameters. Larger SArgs captures,
descriptors, machine bodies, externally observable entry conventions, calls and
helper-producing operations retain the existing path.

Qualify with independent BYTE/WORD leaf examples and TN. Cover entry-copy
hazards, mutation, loops/joins, register and flag clobbers, last-use/result
handoff, pair interference, source ZP reservations, address exposure, helper
fallback, and ZP exhaustion. VM comparisons must cover page crossings and
pointer wraparound where applicable, return ABI, and calls from optimized
callers. Measure final artifacts and maps, separating code, inline data, ZP
usage, and whole-image savings; compare feature-off, feature-on, and traced runs.

Use focused MIR6502 tests during development and the affected backend's full
qualification for final integration, including applicable runtimes, ABI modes,
origins, and LF/CRLF fixture paths. No NIR contract change is intended.

Shared Y induction, pointee-load elimination, comparison/flag reuse, return-tail
merging, cross-call parameter residence, and general register allocation are
separate follow-ups. Placement should make them easier to express without
depending on them to demonstrate a useful first saving.

## Proposed refinement: whole-image cost acceptance

Implemented and qualified 2026-11-15 against `8626beff`; both final guards use
whole-image acceptance. See the completed
[whole-image cost plan](MIR6502_WHOLE_IMAGE_COST_IMPLEMENTATION_PLAN.md) and
[qualification](benchmarks/6502-whole-image-cost/README.md).
This refinement supersedes per-routine non-growth requirements for the final
placement and terminal-cleanup transactions. Historical qualification receipts
retain the policy and outcomes that actually produced them.

### Profitability belongs to the complete candidate

For the size objective, accept a valid candidate only when its completed packaged
image is strictly smaller than the transaction's selected baseline. One routine
may grow if reductions elsewhere more than offset it. For example, four bytes
of growth and thirty bytes of saving qualify when final emission confirms the
26-byte net saving. Image-size equality and overall growth remain rejections.

Use the existing finalized image measurement at the same origin, target, runtime
and emission settings. Count code, actual allocated storage, helper bodies and
branch expansion in the finalized bytes after relocation resolution and packaging.
A sum of per-routine costs is not authoritative: shared storage, pooled/deferred
data and layout changes can make that sum disagree with the actual image.

Keep per-routine code and ordinary-home deltas for diagnostics. Growth can be
reported prominently without granting it veto power. ZP consumption remains a
reported resource and must satisfy the allocator's existing capacity/conflict
checks; it is not an additional weighted profitability metric.

### Narrow implementation boundary

Apply the same final-image rule to these existing predicates:

| Predicate | Consumers | Revised condition |
| --- | --- | --- |
| `Executable::improves` in [preparation.rs](../src/mir6502/preparation.rs) | Leaf parameter placement, terminal handoff and ordinary-call home selection through `try_proposal` | Strict packaged saving; retain the existing routine-ID inventory compatibility check |
| `improves_cost` in [late_cleanup/mod.rs](../src/mir6502/late_cleanup/mod.rs) | Terminal store/producer/register cleanup and optional spill pruning | Strict packaged saving after the transaction's existing delta/proof checks |

Remove only their per-routine non-growth conjunctions. Retain the cost maps and
instruction totals used by diagnostics. Keep routine-set compatibility as a
separate structural constraint; mismatched routines are not an example of
profitable local growth. Optional pruning remains independently gated and off
by default. This policy revision needs no new optimizer pass, feature switch,
allocation strategy, materialization or measurement emission.

Earlier `does_not_grow` filters in materialization belong to a separate review.
Some filter or rank candidates before the final transaction is reached. This
revision does not remove them, reinterpret their layout measurements as final
images, or claim that every size-improving candidate can now be discovered.

### Correctness and rollback remain independent

Profitability is evaluated only for successfully verified and emitted candidates.
Retain permitted-delta checks, binding/ABI compatibility, register/flag and memory
proofs, callee contracts, authoritative layout validation, final-placement replay
and all earlier-decision preservation. A smaller image cannot compensate for a
failed proof. Failed candidate emission has no usable cost; restore the selected
baseline. Failure to emit the baseline remains a compilation error.

Each transaction continues to own its immutable selected baseline and exact
rollback. This changes acceptance, not candidate construction, batching or the
number of attempts. Do not introduce a local growth allowance, percentage limit,
subset search or another trial to implement the policy.

Compare a candidate with the baseline actually selected at that boundary.
Accepting a net saving does not prove it is better than every unmeasured subset
of candidates. The multiple-leaf-call extension below uses the existing combined
ordinary-call batch and its exact pre-trial baseline. The earlier separate-trial
proposal is withdrawn; this cost rule adds no selection stage or subset search.

### Diagnostics and focused evidence

Report baseline/candidate packaged bytes, the net delta and affected routine
code/home deltas, including positive deltas in accepted trials. Replace the
ambiguous `no-final-saving-or-routine-growth` result with a final-image cost reason;
keep routine-inventory incompatibility and invalid/proof-failed trials distinct.
Trace state must not affect acceptance or add hidden compilation work.

Minimal qualification establishes both predicates' boundaries, one real accepted
case with local growth and an overall saving, and retained exact rollback on
proof failure. Reuse the storage-observation control with scoped observation off:
its recorded 229-byte cross-call trial grows Walk by two bytes but beats the
261-byte fallback. This is evidence for the new policy, not a source-specific
exception or a guaranteed future byte count. Keep scoped observation enabled
for normal production qualification.

Use the existing independent VM oracle and one final TN cart comparison. Broader
backend/runtime/origin matrices remain deferred under the user's minimal-testing
direction. Record the actual work and output changes without attributing old
per-routine-veto rejections to correctness failures.

## Implemented scope and remaining extensions

The initial register candidate has one live formal, consumed by the first entry
load in one block; the other formals satisfy the shared absent-home proof.
Existing consumer forwarding must eliminate every register-bound memory access.
This is an entry-use optimization, not general register allocation across CFG
edges. Single-byte entries retain the existing forwarding policy.

Memory candidates reserve complete objects in deterministic order: a compatible
incoming fixed lane, `$A0–$A3`, then `$E0–$EF`. Pointer-reading bodies can also
use `$AA/$AB` or `$AE/$AF`; `$AC/$AD` remains copy-and-rebase scratch. Direct and
dual pointer consumers use the authoritative pair, and comparison setup declines
when its staging would overwrite another bound object. The effect check resolves
logical parameter reads through their bindings and requires exact consumer-pair
agreement. Physical writes remain subject to expanded CFG lifetime checks.

Pointer-reading candidates currently require an immediate operand in every word
arithmetic operation. Data-dependent pointer adjustments retain their existing
fused staging strategy: relocating them can increase code despite removing an
inline home. This is a general shape restriction, not a program-name exception
or a trial-and-repair subset search. Scalar arithmetic retains the previously
qualified native scope. Dynamic indexing and further fused pointer consumers
need separate qualification before widening this boundary.

The retained checkpoint can cross capture-preserving comparison selection: its
proofs use exact source reads, temp definitions/uses, and dead exit machine state,
without physical alias or callee-preservation assumptions. Other accepted
upstream families with untracked placement/callee obligations still discard the
checkpoint. Resumption reruns core expansion and all downstream preservation
checks. The cost decision records changed routine costs as well as image size.
Under the revised cost rule, an individual increase does not veto an overall
saving; correctness and preservation failures still reject the complete batch.

Final trace identities travel with executable artifacts as diagnostic metadata.
Acceptance never reads those IDs. After the batch decision, the manifest and
home summary select the corresponding existing snapshot/layout; they do not
infer selection from the latest speculative emission or emit the program again.

## Proposed extension: parameter handoff to call arguments

Prepared 2026-11-14 against compiler `471ef1c8`. Connect entry capture, working
placement, and outgoing argument setup through the existing parameter binding
and value-availability analyses. A parameter whose last use supplies a call
argument needs a valid location through setup and consumption at entry to that
callee. It need not survive the callee's execution.

This extends the placement authority above and the scheduling contract in
[argument staging reduction](MIR6502_ARGUMENT_STAGING_DESIGN.md). Keep logical
parameter identity, current value identity, and ABI destinations distinct.
Sharing a physical location does not alias two formal parameter objects or
change the calling convention. SemIR and verified NIR remain unchanged;
MIR6502 owns selection, lane placement, scheduling, and all required proofs.

### Evidence: Strcpy

TN's [Strcpy](../samples/tn/modern/TN.ACT) forwards its parameters:

```action
PROC Strcpy(CHAR POINTER s, t)
  MovePage(s, t, t^+1)
RETURN
```

At `471ef1c8`, optimized MIR6502/ActionCart at `$3000` emits 41 code bytes plus
four ordinary parameter bytes. It captures both words, copies `t` into
`$AC/$AD`, computes the length, reloads three argument bytes, and tail-jumps to
[MovePage](../samples/tn/modern/LIB.ACT). X already retains `s.high` throughout.

A shared placement can use these locations:

| Value | Incoming ABI | Working location | Outgoing MovePage ABI |
| --- | --- | --- | --- |
| `s.low` | A | `$A0` | A |
| `s.high` | X | X | X |
| `t.low` | Y | `$A2` | Y |
| `t.high` | `$A3` | `$A3` | `$A3` |
| `t^+1`, byte result | Computed in the body | A, then `$A4` | `$A4` |

```asm
Strcpy:
    sta $A0
    sty $A2
    ldy #0
    lda ($A2),y
    clc
    adc #1
    sta $A4
    ldy $A2
    lda $A0
    jmp MovePage
```

This is 20 code bytes with no ordinary parameter allocation: 21 fewer code
bytes and four fewer storage bytes. `$A0`, `$A2`, and `$A3` carry parameter
bytes; `$A4` carries the computed outgoing byte. No `$A1` working cell or
`$AC/$AD` scratch pair is needed in the wrapper. `$A0` and `$A2` become valid
only after the explicit stores; they are not incoming ABI mirrors.

The assembled reference matched the generated wrapper in 2,560 VM calls over
all 256 length bytes, page crossings, both overlap directions, and identical
buffers. The comparison covered the complete buffer region and stack balance.
This is exploratory evidence for ordinary RAM under the existing scratch
contract, not compiler qualification, a minimum-size proof, or a promise of a
25-byte whole-image saving. General admission must come from typed facts;
neither `Strcpy` nor `MovePage` may receive a name-based exception.

### One placement authority, byte-level locations

`MirParamHome` supports inline storage, registers, a coherent ZP object, absent
storage, and mixed working words such as `s = [Zp($A0), Reg(X)]`. The mixed form is
`MirParamHome::Lanes(Vec<MirParamLane>)`, where a lane is `Reg(MirReg)` or
`ZeroPage(MirFixedZpSlot)`. The declaration supplies the checked BYTE/WORD width;
vector position supplies the byte offset. Uniform bindings normalize to the
same lane mapping. Mixed storage resolves bytes individually, reserves only
memory lanes, and has no fabricated contiguous address. Maps retain existing
symbol APIs and report memory fragments as `name+offset`, one byte each;
register lanes have no memory symbol. Physical ownership, layout certificates,
and trace records use those same fragments. Retained registers are seeded from
the canonical ABI, while capture establishes memory/register copy availability.
Wrapper discovery follows leaf selection under its own configuration gate.
Use one checked byte-location vocabulary for it and flattened `MirArgHome`
requirements, retaining the canonical ABI helper as the only argument-offset
mapping. Uniform register/ZP forms and mixed forms must normalize to one
unambiguous lane mapping; they must not create competing placement tables.

Each lane retains `(RoutineId, ParamId, byte offset)` and the reaching value
definition. A location is not a value identity. Entry facts come from the ABI;
actual captures establish the selected working locations. Existing availability
analysis tracks additional copies and invalidates them on clobber or mutation.
Do not introduce a second register-content tracker in call materialization.

Choose one working location per lane for the entry-to-call interval. Incoming
and outgoing transfers are boundary operations; arbitrary mid-body spill
insertion and live-range splitting remain outside this extension. This lets
X carry `s.high` for the whole interval while only `s.low` needs a ZP capture.

A dereference still requires a coherent adjacent ZP pair at its read site.
`t = [$A2, $A3]` satisfies that constraint. A mixed word has no whole-object
memory address; it can be passed lane by lane but cannot be treated as a
contiguous pointer cell. A consumer that cannot resolve it must use a separately
proved pair copy or reject the candidate. Taking a parameter's address retains
the existing addressable representation.

Reserve the actual occupied ZP bytes through the existing reservation machinery.
Respect source globals, absolute locals/accesses, wrapping source ranges,
uncaptured fixed inputs, temporaries, pointer scratch, and result locations.
Maps may report real memory fragments and printed MIR may show register lanes;
neither may invent an address or contiguous allocation for a mixed word.

### Entry and call setup use the same values

A typed handoff plan identifies the routine, call site, source definitions and
lanes, chosen working locations, outgoing destinations, and required boundary
transfers. It belongs to the selected preparation artifact and its analysis
generation. Reconstruct stale plans; never recover them from trace text,
physical-address coincidence, or the last speculative callback.

The call materializer consumes the binding and current availability facts:

- A byte already in the required destination needs no transfer. This is a
  value-equality proof, not just matching register or memory names.
- Captures and outgoing transfers obey parallel-copy dependencies. A write
  cannot destroy an unread source lane, pointer component, callee target, or
  another argument. Unsupported cycles retain staging or decline the trial.
- Computing another argument can destroy A, X, Y, flags, or a scratch pair.
  Model the complete expansion of that computation, including implicit
  effects; callee preservation says nothing about these setup operations.
- Keep source reads, writes, faults, and their order/count fixed. A current
  register copy may replace a reload from private working storage; it does
  not authorize moving or eliminating the source dereference that produced
  a value. Preserve word byte order and carry dependencies.

In the example, capture `s.low` before using A, capture `t.low` before clearing
Y, leave X and `$A3` untouched, then establish `$A4`, Y, and A. This establishes
all five outgoing bytes without retaining two independent copies of each word.

### Consuming a value at a call

Separate three proof boundaries:

| Boundary | Obligation |
| --- | --- |
| Before the call | Every live parameter byte survives all intervening setup and memory effects. |
| Callee entry | Every canonical argument and additional observable input has the correct value. |
| After the call | Only values actually live afterwards require a callee-preservation or save/restore proof. |

At the call, argument consumption precedes the invocation's clobbers. A last
argument use must not disappear merely because an outgoing store overwrites its
working location, and a clobber must not count as a new parameter definition.
Extend the existing logical-to-physical liveness projection for this ordering;
do not globally make calls harmless barriers or stop verifying at their opcode.

Leaf selection rejects calls. Terminal handoff admits a proved terminal call
after establishing the entry obligations and the absence
of protected values after it. Callee indirect writes may remain conservative:
`MovePage` writes through its destination and need not preserve the consumed
pointer carriers. Opaque setup, unaccounted entry observations, or a parameter
live after the call still block the candidate.

Use shared callee/entry analyses to cover noncanonical register, flag, and fixed
memory inputs, including reads before overwrite. A direct machine callee is
eligible only when its structured contract or bounded analysis establishes the
needed facts. The caller itself still contains no machine block in the first
scope. Broad write effects do not alone prohibit terminal handoff; an unknown
read/entry dependency cannot be ignored. Missing shared facts require a focused
analysis extension or fallback, not an ad hoc scan in parameter selection.

Retain the existing source-observation rules for active compiler scratch and
unexposed parameter storage. Removing a home still requires its exposure and
observation proof. Explicit source ownership and assembly references win over
ABI-window preferences; no numeric ZP range is universally private. Keep public
memory accesses, call target/count, result ABI, and stack behavior unchanged.

Changed wrapper effects also matter to its own callers. Build candidate callee
summaries and replay every already-consumed preservation obligation. A wrapper
may now write `$A2` instead of `$AC/$AD`; matching its ordinary arguments and
buffer result does not establish that all optimized callers remain valid.

### Initial admission and exclusions

Start with a straight-line ordinary Action wrapper with at most four incoming
argument bytes, unexposed scalar BYTE/WORD parameters, and one known direct call
as their last use. The outgoing canonical argument list may contain up to five
bytes, including a computed byte as in `Strcpy`. Parameters are read-only in
this first wrapper scope; existing mutable leaf coverage stays unchanged.

Initially the call is followed only by a void return. Retain the existing
tail-call decision: a `Call; Return` may become a jump only through its normal
ABI/stack proof. Do not require an early tail-call opcode or infer permission
solely from source syntax. Set up the same arguments for either final form.

Allow only already-qualified native computations and indirect-read consumers
whose expanded effects can be checked against the lane reservations. No earlier
call/helper, indirect write, opaque barrier, or machine block occurs in setup.
Loops, joins, parameter mutation, multiple calls, indirect callees, aggregate or
variadic arguments, SArgs, and observable wrapper entry conventions keep the
current path. Returning a call result needs a later explicit result-lifetime
extension. General residence across calls remains separate: `Strcat`, which
uses its pointers after `MovePage`, is a required fallback control.

### Integration, bounded work, and rollback

Extend the existing selector, ABI capture builder, and call materializer;
do not add a late parameter-remapping peephole. The placement decision remains
before scratch allocation, capture, and layout, with upstream choices frozen.
Downstream forwarding and terminal cleanup consume the selected bindings.

The fallback is the **current optimized artifact, including accepted leaf
placement**. A failed wrapper proposal must not discard `Fnamecmp`'s existing
saving by falling back to the artifact before parameter selection. Retain the
accepted leaf proposal's typed input/bindings with its executable result and
the selected core checkpoint; do not reconstruct them from emitted addresses.

After current leaf selection, consider one deterministic batch of wrapper
bindings while retaining the accepted leaf bindings. Resume the same core
continuation once for this extension. Reuse the already-finalized fallback
output. This permits at most one additional core materialization relative to
today's optimized path; with both leaf and wrapper trials it means at most two
additional materializations relative to disabling parameter placement entirely.
There is no subset search, repeated upstream search, or iterative schedule
repair. An empty candidate set adds no materialization. Measure downstream
emissions and preservation work separately; the extra continuation can be costly.

Run candidate-specific helper resolution, effects, layout-sensitive rewrites,
terminal cleanup, preservation replay, verification, and final emission. Require
a strictly smaller packaged image relative to the leaf-qualified fallback under
the revised cost contract. Any unsupported expansion, conflict, stale obligation,
image-size equality or image growth restores that complete artifact,
including its maps, relocations, summaries, and reservations. An untracked
upstream dependency continues to make the checkpoint unusable.

Freeze bindings before authoritative downstream layout. Retain the exact
selected emitted artifact and trace identity on acceptance or rollback. Report
admitted/rejected shapes, mixed lanes, retained incoming bytes, omitted captures,
call transfers, final bytes and work counts; tracing cannot affect decisions.
Gate this new scope independently during qualification without disabling the
existing leaf optimization. Its production default is a qualification decision.

### Verifier and qualification requirements

In addition to the existing binding checks, require complete lane coverage,
legal widths/order, no duplicate simultaneous physical ownership, and consistent
normalization of register/ZP/mixed bindings. Ownership follows lifetimes: an ABI
destination may reuse a working byte after its last required source read, or
retain that byte when the required value is already present. Availability copies
do not constitute new independent owners. No memory operand or relocation may
address a register lane. Planning layout, consumer effects, final maps, and
emitted operands must agree on every memory fragment and coherent pointer pair.

Check entry capture and outgoing call establishment against actual expanded
effects, including register/flag clobbers, last-use ordering, and any additional
callee entry inputs. Check parameter deadness through the closed CFG and result
or return paths even though initial candidates are straight-line. Retain exact
rejection for unsupported calls; do not weaken leaf lifetime verification.

Qualification must include independent forwarding wrappers as well as TN:

- Distinct low/high bytes; X retained while A is reused; Y captured before an
  indexed read; fourth byte already in `$A3`; fifth byte produced into `$A4`.
- Changed outgoing positions, partial overlap, duplicated arguments, copy cycles,
  later setup clobbers, source-owned ZP, insufficient scratch, and exposed homes.
- Equal numeric values from different definitions, parameter mutation, and
  subsequent call/result uses, demonstrating that shape similarity is no proof.
- Machine callees with extra entry flags/register/fixed-byte reads, changed
  wrapper clobbers observed by optimized callers, and unknown-call fallback.
- All byte lengths, scalar boundaries, page crossings, overlapping and identical
  buffers, ordered pointee reads/writes, arguments, result ABI and stack balance.
- Cart/standalone, shifted origins, actual LF/CRLF fixture paths, maps and MADS
  reassembly, trace equivalence, exact rollback, and bounded compiler work.

Keep leaf-only optimized output as the feature-off control. Retain TN's 84-byte
leaf saving, exercise independent nonregression controls, and report wrapper
code, ordinary storage, occupied ZP, whole-image size, compile time, and memory.
The 20-byte reference is a measurable target for the unchanged-ABI `Strcpy`
shape; a generated result must earn it through these general proofs. Full
MIR6502 qualification is required before enabling this scope by default.

### Terminal consumption proof boundary

The handoff frame contract retains the direct target and canonical argument
width, not a pre-expansion instruction index. Discovery owns the source body,
generation-scoped reaching definitions and a shared callee-entry fact. Expanded
verification reconstructs the unique terminal call and recomputes entry inputs.
Call argument reads precede invocation clobbers; only parameters dead after those
reads may lose their working locations. Setup writes never discharge their own
liveness obligations.

Entry obligations are separate from exit preservation and exact write footprints.
The shared machine decoder computes registers, individual flags and fixed bytes
read before definite overwrite through closed control flow. An unknown entry
observation rejects the proposal. A known terminal callee may have unbounded
pointee writes; it still must receive every required input, and existing wrapper
preservation contracts are replayed by the continuation.

### Direct-wrapper selection policy

The extension makes one deterministic batch after leaf selection. Direct
forwarding may retain incoming X/Y lanes; other live lanes use reserved working
ZP. The existing parameter call consumer resolves memory lanes through the frame
binding so ordinary transfer lowering recognizes fixed-slot identity copies.
Register consumers still require the shared availability proof. An incoming A3
lane is retained only when the outgoing lane is the same definition; unsupported
permutation cycles use the original staging strategy. Already-elided homes do
not trigger another capture proposal. The final-cost transaction can reject the
entire batch.

### Resident pointer producers

A byte read may consume a parameter pointer whose two lanes form one coherent
reserved ZP pair. Its pointer definition must come directly from that parameter;
this does not admit data-dependent pointer reconstruction. Indirect reads use Y,
so another parameter may retain X but cannot retain Y through such setup.

The existing parameter-consumer rewrite may replace a captured word temp with
its resident parameter identity at both the indirect read and terminal call.
It proves both word definitions, checks every use and intervening source write,
and preserves the position of each pointee read. Shared pointer materialization
then consumes the pair directly, while shared call lowering omits identity
transfers. There is no additional value tracker or late address remapping pass.
The terminal call consumes the last uses before arbitrary invocation clobbers;
post-call uses, including Strcat's, retain the existing placement.

Entry proofs remain conservative: unsupported machine instructions, opaque
callees and scalar callees without an available scalar entry contract reject
selection. A bound scalar callee whose pre-capture working-home reads cannot
yet be discharged by the entry analysis can also reject a wrapper. This is a
coverage restriction, not permission to infer entry behavior from formal widths.
The refinement below addresses the capture-boundary part of this restriction.

### Qualified production policy

`Mir6502Config::optimized()` enables terminal handoff; conservative configuration
does not. Both the peephole master gate and parameter-home selection must also
be enabled. Unsupported shapes and unsuccessful final-cost trials retain the
complete leaf-qualified artifact. This is a size-oriented policy: one additional
core continuation can cause substantial downstream emission and preservation
work. The [qualification receipts](benchmarks/6502-parameter-handoff/README.md)
report that cost separately from upstream search, which is not repeated.

Compiler-generated TN Strcpy and independent CopyCounted reach 20 code bytes
with no ordinary parameter storage, saving 25 bytes each. TN retains its prior
leaf savings. Qualification covers the full affected backend, both runtimes and
origins, actual LF/CRLF paths, MADS reassembly, ordered-memory VM oracles and
independent leaf nonregression controls. This evidence supplements the initial
handwritten reference; it does not establish a general minimum-size theorem.

### Proposed refinement: callee entry facts through parameter capture

Status: implemented and qualified at `4ea5886d`, against baseline `72131db3`.
The shared handoff proof composes with ordinary Action callees whose working homes
were selected by leaf placement. MIR6502 owns this refinement; SemIR/NIR meaning
and the public ABI are unchanged. Independent wrappers save 19 bytes each; TN
remains 9,853 / 9,960 bytes. See the
[qualification receipts](benchmarks/6502-callee-entry-capture/README.md).
The [implementation plan](MIR6502_CALLEE_ENTRY_CAPTURE_IMPLEMENTATION_PLAN.md)
defines the reproducer, shared capture boundary, correction and qualification slices.

#### Concrete failure and required meaning

Suppose a callee receives one word in A/X and has selected `$A0/$A1` as its
working home. Its entry capture copies those incoming bytes before the body
uses the home:

| Boundary | Required or established values |
| --- | --- |
| Before callee capture | The caller supplies the word in A/X. The old contents of `$A0/$A1` are irrelevant. |
| Capture | Transfers from A to `$A0` and X to `$A1` establish the working word. |
| Body entry | The body can read the word from `$A0/$A1`. |

At the baseline, [scalar entry analysis](../src/mir6502/analysis/known_callees/entry_inputs.rs)
projected parameter reads onto selected physical homes without their initialization
on the retained pre-capture input. It could report `$A0/$A1` as extra incoming
requirements, causing [handoff admission](../src/mir6502/parameter_homes/handoff.rs)
to reject a caller that correctly supplied A/X. The independent
[capture fixture](../fixtures/runtime/callee_entry_capture.act) establishes this
conservative rejection and the resulting size opportunity. It also exposes the
normal Return's implicit SP dependency, which is separate from extra inputs.

**A callee-entry fact describes requirements immediately before the callee's
first executed instruction, including any compiler-generated capture.** Reads
of working homes after capture are body requirements. They become caller
requirements only if the capture does not establish the needed values. Canonical
arguments remain governed by the typed call ABI, even if the body ignores one.

#### Shared capture effects and representation ownership

Before capture is materialized, compose the planned capture's ordered effects
with the body analysis. Derive that capture from the same typed description
used by [ABI prologue generation](../src/mir6502/materialize/abi.rs), using the
[canonical ABI mapping](../src/mir6502/abi.rs) and selected parameter lanes.
Share or factor that description rather than implement a second capture policy
inside handoff discovery. This analysis requires no emitted addresses or trial
compilation of the callee.

After materialization, analyze the actual entry operations and body. Do not
prepend another hypothetical capture. The query and returned fact must identify
which representation they describe through typed provenance. A binding table,
an apparent instruction prefix, or an old operation index cannot establish
whether capture has already happened. Final handoff verification recomputes the
requirements on the actual expanded callee before accepting the trial.

The composition must account for:

- Reads of incoming ABI lanes and definite writes to individual working bytes,
  in the capture's actual order. Partial writes discharge only their own bytes.
- Retained register lanes and identity fixed-slot lanes. Keeping an incoming
  byte in `$A3` requires that entry byte; it does not create a store or make the
  byte independent of its incoming value. Absent homes create no capture.
- A/X/Y and individual flag effects of capture and expansion. In particular,
  plain stores preserve flags, so a body read of an incoming flag still matters.
  Unproved scratch, helper or machine effects make the entry contract unknown.
- Any fixed-memory or register read before its definite overwrite. Never erase
  requirements merely because an address belongs to the ABI window or appears
  in a parameter binding. All control-flow paths contributing to the read must
  have the required definition.

Only compiler-owned capture participates in this composition. Machine callees
and observable entry conventions keep their existing shared entry analysis;
formal declarations alone do not authorize inventing a prologue. Source-owned
storage, exposure and reservation checks continue to govern placement.

Native current-location (`=*`) callees may also supply an `ActualEntry` fact
from their existing expanded operations. Entry placement does not change their
Action argument ABI or imply unknown writes. This permission does not synthesize
observable-entry capture: `PlannedCapture` remains unsupported for these native
entries. Their ABI classification, own storage policy, entry labels and
patchability remain intact. Shared facts still require the supported finite
signature, closed native leaf body and complete effects; machine/OS, SArgs and
nonleaf residence admission are separate extensions. The
[native current-location plan](MIR6502_NATIVE_CURRENT_LOCATION_CALLEES_IMPLEMENTATION_PLAN.md)
uses these facts to qualify caller residence without granting unrelated
inlining, handoff or storage-removal permissions.

Keep the correction in shared callee-entry analysis. It must consume complete
typed MIR ABI/declaration, binding and effect facts. Pointer parameters require
the same completeness; the presence or absence of `scalar_signature` alone
does not establish a safe entry contract. Missing facts retain the conservative
fallback. No SemIR lookup or source-name inference is permitted.

The initial implementation certifies register-store capture, retained register
lanes and identity fixed slots. Memory-sourced capture remains unknown because
its pseudo Store omits expansion effects on A and N/Z. SArgs remains excluded.
The typed `MirFrame::direct_parameter_abi` declaration supplies completeness for
finite byte/word integer and pointer signatures without changing the meaning of
`scalar_signature`. The entry query validates it against current frame slots.
Each parameter records a scalar byte/word or a two-byte data-pointer ABI
representation. An Array-class reference can satisfy the entry contract through
the pointer representation while keeping its Array storage identity. Same-size
records, mismatched descriptors and unsupported widths remain unknown. This
does not relax array-home placement or the existing capture restrictions.
An ordinary Return's implicit SP use is part of the balanced ABI, while explicit
stack dependencies remain extra entry requirements. General rewrite liveness
continues to preserve Return's SP use.

#### Fact validity and acceptance

An entry fact belongs to the selected callee and its representation. Its validity
depends on the routine body/entry CFG, ABI declarations, parameter bindings and
capture description, plus the existing machine-payload and global-backing
dependencies. Changing any dependency invalidates the fact. Materializing,
eliding or rewriting capture requires a new fact for the resulting body, even
when a physical address happens to remain equal. Facts from rejected placement
trials cannot qualify the selected fallback.

Discovery may use a conservative pre-capture contract. Final acceptance checks
the current expanded contract against the actual outgoing arguments; it must
not simply compare an old summary with itself. Newly exposed flag, register or
fixed-byte inputs reject the trial unless the existing handoff proof supplies
them. Exit-preservation replay remains a separate obligation.

Use the existing handoff gate, single wrapper batch, final code/home/image cost
acceptance and complete rollback. This refinement adds no core continuation or
callee-emission search. It does not extend wrapper lifetimes across calls, add
call-result forwarding, or admit unknown effects and unsupported call forms.

#### Required evidence

Start with an independent ordinary Action caller/callee pair demonstrating the
false requirement and a possible final-byte saving. Cover BYTE/WORD, inline,
ZP and mixed homes, retained registers, and an incoming fixed-slot identity.
For supported capture forms, verify that the pre-capture contract conservatively
covers the corresponding materialized entry requirements without the false
`$A0/$A1` requirements in the example. Also check capture omission and stale
facts after binding, body, phase or global-backing changes.

Negative controls must preserve genuine extra fixed-byte/register/flag inputs,
partial or conditional definitions, source ZP conflicts, unknown callees and
post-call uses. VM oracles must observe arguments before capture and check public
results, ordered memory accesses and stack balance. Include optimized callers,
both runtimes/origins, actual LF/CRLF loading and trace/untraced agreement.

Measure TN and independent controls against the qualified handoff baseline.
Report actual gains or unchanged output, candidate rejections, and continuation
counts. Existing Strcpy, Fnamecmp and leaf savings are nonregression controls;
their current results do not establish a benefit from this proposed refinement.

## Proposed extension: ZP homes across ordinary calls

The [implementation plan](MIR6502_CROSS_CALL_HOME_SELECTION_IMPLEMENTATION_PLAN.md)
defines parameter-first delivery, local binding support and final qualification.

This extension is implemented and
[qualified](benchmarks/6502-cross-call-homes/README.md#final-qualification-and-default-slice-6).
`Mir6502Config::optimized()` enables it; the conservative default leaves it off.
The parent peephole and parameter-home gates remain authoritative. The original
TN trial failed the no-routine-growth rule. The implemented
[storage-observation refinement](MIR6502_STORAGE_OBSERVATION_DESIGN.md) removes
the two global cleanup vetoes using routine/object/lifetime evidence. TN now
[measures 9,791 packaged cart bytes](benchmarks/6502-storage-observation/README.md#tn-and-optimized-default),
with Ord at 51 code bytes and Items at 37, without ordinary homes or growth in
other routines. Its cleanup exception remains limited to private temporary-store
deletion; residence and cost requirements in this note remain authoritative.

Extend the existing placement contract to ordinary calls followed by further
computation, including calls inside loops. First place parameters in ZP; then
admit eligible source locals through the same placement authority. Keep one
working home per object, coherent pointer pairs, the public ABI, and the existing
fallback. This extension does not introduce register allocation across calls,
split lifetimes, save/restore insertion, or a second late allocator.

MIR6502 owns the new placement and preservation proofs. SemIR/NIR continue to
provide typed storage identity, exposure, initialization and call facts; they
do not choose physical homes. Emission resolves and checks final placement.
No executable source-name tests or lookups back into SemIR are permitted.

### Motivation and current boundary

[Ord and Items](../samples/tn/modern/LIB.ACT) repeatedly inspect a pointer, call
`Next`, and replace the pointer with its result. At `a77210d8`, optimized
MIR6502/ActionCart at `$3000` emits 80 code bytes plus five inline storage bytes
for `Ord`, and 56 plus three for `Items`. The inline pointer requires repeated
copies into an indirect-addressing pair; other accesses use absolute addressing.

These objects are excluded before placement or costing. The
[leaf selector](../src/mir6502/parameter_homes.rs) excludes calls, the
[handoff selector](../src/mir6502/parameter_homes/handoff.rs) requires a terminal
void call, and the [ZP selector](../src/mir6502/parameter_homes/zero_page.rs)
considers parameters rather than source locals. This is not evidence of ZP
exhaustion. Loops themselves already participate in leaf lifetime verification;
the new boundary is residence around an ordinary call on those CFG paths.

The three objects in `Ord` have different obligations:

| Object | Obligation at `menu = Next(menu)` |
| --- | --- |
| `menu`, a pointer parameter | Its old value supplies the argument. That value is dead after consumption; the assignment defines the next value used at the loop header. |
| `item`, a pointer parameter | Its current value must survive the call for the next comparison. |
| `c`, a byte local | Its incremented value must survive the call, both for later iterations and for the return. |

At this baseline, `Next` writes `$AE/$AF`, `$AC/$AD` and `$A0/$A1`. Those pairs
cannot hold a value that must survive this invocation under a no-write proof.
This observation motivates candidate filtering; actual callee facts must authorize
placement, including after either routine changes. Neither those pairs nor the
remaining ZP bytes are universally private or available.

### Logical definitions and call boundaries

Extend the existing [logical-home lifetime projection](../src/mir6502/parameter_homes/lifetimes.rs)
and [CFG liveness](../src/mir6502/analysis/home_liveness.rs). Logical reads and
definitions retain `(RoutineId, ParamId/LocalId, byte offset)` identity. Project
them onto selected physical bytes for interference checks. A definite write to
the logical object kills the previous value of its written lanes; a scratch,
ABI or callee write to the same address is a potential clobber, never such a
definition. Calls must not make their own preservation obligations disappear.

Use the existing ordered call expansion to distinguish:

1. Argument evaluation and setup, including every source read and temporary or
   outgoing-lane write. A home cannot be overwritten before its final required
   read, even if the object is dead after invocation.
2. Callee entry, where canonical arguments and any additional register, flag or
   fixed-memory inputs must have their required values. Use the shared capture
   composition described above.
3. Invocation, where every object value live afterwards needs a preservation
   proof for each occupied byte.
4. Result delivery and assignment, which establish the returned value and then
   the new logical destination. Result-copy scratch must also respect all other
   live objects.

For the first scope, preserve explicit result delivery and assignment. The old
`menu` need not survive `Next`; its complete new value must be established before
the next dereference or comparison. A physical result write does not implicitly
implement the source assignment merely because its address matches a home.

Solve liveness over the closed CFG to its normal fixed point. Include zero-trip
loops, backedges, conditional calls, early exits and joins. A conditional
definition kills an incoming value only on the paths where it executes; partial
word writes affect only their lanes. Availability facts at a join require
agreement on all incoming paths, and register copies remain subject to ordinary
call clobbers even when their memory home survives.

### Callee contracts and ZP reservations

Reuse [known-callee summaries](../src/mir6502/analysis/known_callees.rs) for entry
requirements and conservative memory-write coverage. A selected byte live across
a call must be disjoint from all possible invocation writes. A pointer needs
this proof for both bytes. The initial proof accepts absence of writes; it does
not infer preservation from a callee apparently restoring a value later.

Coverage includes entry capture, callee working homes, allocated temporaries,
implicit scratch, result homes and any summarized nested calls. Resolve bound
logical storage and virtual ZP allocations to their actual physical bytes.
Unallocated virtual slots, unresolved backing and unknown effects cannot be
treated as non-aliasing. Absolute and fixed-ZP spellings of the same address
alias. A private-inline-RAM preservation query is not a proof for shared ZP.

Check outgoing setup separately from invocation preservation: a callee can
preserve a byte that argument setup destroys. Also retain entry-input and
source-observation checks; absence of writes alone does not authorize changing
an observable fixed-memory input. Keep calls as ordering barriers except for
the specific facts established by the shared analyses.

Candidate selection excludes source-owned bytes, other live objects, caller
scratch requirements and relevant callee writes through the existing reservation
machinery. In the first scope, use conservative whole-object reservations and
exclude invocation writes even for a parameter redefined immediately afterwards.
This permits a preserved pair plus an explicit result copy without requiring
call/result overlap. Later lifetime-based overlap must have its own proof.
Keep the existing slot policy, including `$AC/$AD` as copy-and-rebase scratch;
do not expand the available range as part of this work.

Start with one known direct ordinary Action call site per routine, which may
execute repeatedly in a loop, and otherwise already-qualified native operations.
Retain the four-byte incoming-parameter limit and unexposed BYTE/WORD scalar and
pointer scope. Callees must have complete supported entry and write contracts.
Multiple call sites and acyclic nested callees can use the same contract once
qualified: each live byte must survive every relevant call, including transitive
writes. Recursive cycles, indirect or unknown calls, SArgs, machine bodies,
unsupported helpers and opaque or indirect-write effects retain the fallback
for this new scope. Existing terminal handoff keeps its independently qualified
admission rules.

### Source locals use the same authority

After parameter residence is qualified, extend the binding/reservation queries
to source locals identified by `(RoutineId, LocalId)`. Parameters retain their
ABI entry transfer; locals acquire values from their actual source definitions.
There is no synthetic parameter capture for a local and no second value tracker
or separate local ZP allocator. Generalize shared storage queries where needed
without an unrelated module or IR migration.

`MirFrame.local_homes` is an optional complete table for ordinary `LocalId`
declarations. `MirLocalHome` admits only `Inline` and a complete adjacent
`ZeroPage` home. Fixed/aliased declarations do not acquire bindings. Shared
`object_lane`, `object_pointer_pair` and `resident_bytes` queries connect parameter
and local identities to physical bytes; parameter ABI/capture roles remain
separate. Missing/duplicate entries, overlapping owners, unreserved bytes,
out-of-range accesses and conflicting allocation metadata are verifier errors.
When this table exists, a missing entry must also remain unresolved in layout
and callee-effect queries; it cannot recover an inline home or private-memory
facts from the legacy declaration.

The initialization proof uses the existing backward CFG home-liveness engine
with logical-local transfers. A byte live at entry depends on a previous
invocation and cannot relocate. Expanded physical reads project back to the
logical local, while only logical stores kill that dependence. Calls and scratch
writes cannot initialize it. Check this rule even for explicitly supplied MIR
bindings, before automatic selection or final emission.

Initially admit unexposed BYTE/WORD scalar and pointer locals whose complete
value is definitely assigned during every invocation before any read. `c = 0`
in `Ord` and `Items` establishes that property. A declaration is not evidence of
per-invocation initialization: entry reads, partial or conditional initialization,
and values retained between invocations must keep their existing storage unless
their semantics are separately proved. Fixed-address objects, aliases,
initializer-backed storage and exposed cells remain outside this first scope.

Every load, mutation, effect query, pointer consumer and map must resolve through
the selected binding. Only inline bindings allocate ordinary frame bytes; a ZP
local has its reserved bytes and no shadow inline cell. Address-taking or machine
references cannot silently change target. The existing source-observation rule
for compiler-owned storage applies only after the object's relocation proof;
moving a pointer cell does not authorize removing or reordering pointee accesses.

### Result-home coalescing is a separate extension

After parameters and locals are qualified, consider a working pair at the
canonical word result home `$A0/$A1`. For `menu = Next(menu)`, this could remove
the copy back to a preserved pair. It requires an explicit identity between the
typed call result, the source assignment and both home lanes, not merely matching
addresses or the fact that the old value is dead.

The proof must cover final argument consumption before overwrite, definite result
delivery on every returning path, absence of intervening conflicting uses, and
the assignment's complete word semantics. Any omitted copies must preserve live
registers and flags. Subsequent calls, pointer scratch and loop joins must respect
the new value. Returning `c` may overwrite `$A0` only after `menu` is dead.
Keep explicit result copies in the initial parameter/local implementation; do
not add a special case for `Next` or introduce a new calling convention.

### Selection, invalidation and acceptance

Add this scope to the existing pre-capture selector and retained core continuation.
Use the current optimized artifact, including accepted leaf placement, terminal
handoff and terminal cleanup, as the fallback. Do not rematerialize upstream
strategies per object or trial-compile callees to discover usable slots.

Discovery may consume the selected materialized callee contracts. They are
candidate filters, not final certificates: rematerialization or another routine's
proposal can change capture, homes, temporary allocation and scratch. Recompute
contracts from the expanded candidate and verify all resident callers against
that candidate. Two routines cannot independently claim a byte when the caller
needs its value during the callee's use. Unknown or invalidated contracts reject
the batch; do not add a placement fixed-point search to repair them.

Use one deterministic proposal batch and at most one additional core continuation
for this extension. Adding local candidates later joins that batch rather than
adding a second search. An empty batch adds no materialization. Keep bindings
fixed for downstream layout-sensitive work and terminal cleanup; check physical
aliases against authoritative placement and replay all already-consumed callee
preservation obligations. Recomputing a summary does not repair a caller rewrite
that relied on the old one.

Acceptance requires valid bindings, expanded lifetime/entry proofs, final
preservation replay and a strictly smaller complete packaged image under the
revised cost contract. Individual routine code/home increases are reported. Include captures, argument and
result copies, branches, helpers and removed inline storage in the cost. Report
occupied ZP separately; using more ZP is a resource cost, not an image saving.
Any failed proof, unknown cost, image-size equality or image growth restores the
complete selected artifact, bindings, maps, reservations and facts. Trace output must explain
rejections and costs without affecting selection. Qualify behind a separate gate
before deciding the optimized default.

### Qualification and delivery order

Deliver parameter residence first, local residence second, and result-home
coalescing separately. Each uses the same storage/lifetime authority. Qualification
must include independent controls alongside `Ord`, `Items` and whole TN:

- Byte, word and pointer values live across a call, consumed by it, or redefined
  afterwards; distinct lane values; one-lane clobbers; argument-setup and
  result-copy interference despite a preserving callee.
- Backedges, zero and multiple iterations, conditional definitions/calls,
  partial writes, joins and early exits. Local controls distinguish definite
  per-invocation initialization from entry reads and persistent values.
- Callee capture and scratch conflicts, changed callee placement, absolute/ZP
  aliases, source reservations, exposure and ZP exhaustion. Multiple/nested calls
  exercise either proved coverage or the documented initial fallback; recursion
  and unknown effects must remain conservative.
- Optimized callers that consumed changed callee contracts, exact rollback,
  trace/untraced agreement and the additional continuation/emission counts.

VM oracles must check arguments, results, ordered observable memory accesses,
return ABI and stack balance, including pointer page crossings and applicable
wraparound. Preserve existing leaf, Strcpy handoff and late-cleanup gains. Measure
feature-off/on final routine code, ordinary storage, ZP use and packaged image
size, and record compiler work. A source-shape expectation or handwritten target
is not a saving. Complete affected MIR6502 integration, VM/runtime and assembly
qualification, including cart/standalone, shifted origins and actual LF/CRLF
fixture paths, before enabling the new scope by default.


## Proposed extension: multiple calls to leaf routines

Implemented and qualified 2026-11-15 against `0880cd2f`. Optimized builds
enable the discovery gate; conservative configuration leaves it off. See the
[qualification](benchmarks/6502-multiple-leaf-calls/README.md).
The [implementation plan](MIR6502_MULTIPLE_LEAF_CALL_HOMES_IMPLEMENTATION_PLAN.md)
uses one combined ordinary-call batch and the qualified whole-image cost rule.
The user's minimal-testing direction remains in effect.

### Scope and existing authority

Admit an otherwise eligible caller with two or more direct call sites when
**every callee satisfies the existing ordinary leaf-call contract**. Distinct
sites may call the same routine, different routines, or lie on different branches;
any site may execute repeatedly in a loop. Count sites in the retained MIR body,
not source syntax or runtime invocation count. Zero-call and one-call routines
keep their established selection paths.

Retain the current four-byte incoming-parameter limit, native operation scope,
source-ZP exclusions, complete byte/word bindings and per-invocation local
initialization requirement. Each callee must pass `ResidenceCalls::writes`:
ordinary Action ABI, supported actual entry, supplied canonical arguments and
known write coverage. Its expanded body must have no call, runtime helper,
machine block or barrier. The callee restriction applies to every site.
Unsupported calls make this caller ineligible; their effects cannot be omitted
while placing its objects. Other eligible callers can still enter the batch.

This extension does not traverse call-graph levels or admit nested or recursive
callees. It changes neither return-home coalescing nor register residence,
split lifetimes, save/restore insertion, source-store deletion or ABI roles in
scoped observation. MIR6502 continues to own placement; SemIR/NIR contracts and
source-language semantics remain unchanged.

`FindItem` is a motivating case: it calls `Key(menu)` and `Next(menu)` while its
pointer and character remain in ordinary homes. The measured baseline has 74
instruction bytes plus three home bytes. Both callees are ordinary leaves, but
Key currently has an unknown write summary. The generalized selector now reaches
FindItem and rejects its first call on that existing contract; the call-count
restriction no longer blocks discovery. TN remains 9,791 bytes after qualification.
No routine name or handwritten size target participates in selection.

### Combined reservation, separate call proofs

For each eligible caller, obtain every call's contract from one immutable
selected expanded program. A candidate ZP byte is available only if all calls
preserve it. Equivalently, reserve the union of their possible writes:

```text
forbidden(byte) = any call whose write contract does not preserve byte
```

Use `MirKnownMemoryWrites::preserves_fixed_byte` and the existing reservation
policy for both parameters and initialized locals. Include actual capture,
working homes, virtual allocations, scratch and result writes. Keep unresolved
coverage conservative. Reserve both lanes of a pointer coherently. A call on
one branch still contributes its writes; this first extension does not make
placement depend on which path executes or whether a particular value is dead
at that call.

Reuse one `ResidenceCalls` context per program snapshot. Shared callee summaries
and entry facts may be cached by routine ID, but validate argument supply at
**every call site**, including repeated calls to the same callee. Never let the
first valid call certify a later call's setup or additional entry inputs.

The union filters placement; it does not replace expanded lifetime verification.
The existing verifier already visits every call and must continue to check:

- Outgoing setup in execution order, before invocation. A preserved byte can
  still be overwritten by argument setup before its final read.
- Each logical byte live after each call against that call's current writes,
  including loop backedges, conditional calls and joins.
- Explicit result delivery and logical reassignment. A may-write by a call or
  a scratch store never initializes a parameter/local or kills its old value.

The existing invocation-home certificates already inspect every call. Multiple
sites must satisfy the same certificate and historical-exposure restrictions;
the scoped cleanup exception gains no new ownership or ABI permission here.

### Selection and exact fallback

Add eligible multiple-call routines to the existing ordinary-call proposal along
with eligible single-call routines. Discover all candidates from the same
immutable `SelectedPlacement` after leaf placement and terminal handoff. Use one
`ResidenceCalls` context for that selected expanded snapshot, reserve the union
of writes separately for each caller, and apply proposed bindings to one cloned
input in the existing deterministic routine order.

Run `try_proposal` once for that combined proposal through the existing
`cross-call-homes` stage. The whole completed image must be strictly smaller than
the selected pre-trial image. Individual routine growth is diagnostic. There is
no separate multiple-call stage, second continuation, single-call pre-trial or
subset retry. The discovery gate changes which callers enter this batch, not
how many batches run.

The exact fallback is the `SelectedPlacement` entering the ordinary-call trial,
including accepted leaf/handoff choices. It is not a separately measured
single-call result. An unsupported caller is omitted during discovery; if a
combined candidate later fails verification, preservation, emission or cost,
restore that complete baseline. This bounded policy does not search for the best
subset or promise to preserve gains from an unmeasured single-call-only batch.

Selected callee facts remain discovery filters. Resume core expansion, rebuild
facts from the expanded candidate and verify every affected caller. Keep bindings
fixed inside terminal cleanup and retain final-placement replay, invocation-home
certificates and earlier-decision preservation. Multiple sites grant no new
permission to discard observable reads or ABI obligations.

The `enable_multiple_leaf_call_homes` discovery gate admits callers with two or
more sites. With it off, preserve the current exactly-one-call scope. It remains
subordinate to peepholes, parameter-home selection and cross-call selection;
conservative configuration keeps it off, and optimized configuration enables it
following the focused qualification. Zero-call routines remain outside this
proposal. Keep `cross_call_home_trials` at zero or one and reuse existing result
and per-routine cost diagnostics. Add call count and failed-site identity where
needed to explain eligibility without a separate tracing pipeline.

### Focused qualification

The user's minimal-testing direction scopes this extension to a grouped proof
test, small integration/VM controls and one final TN ActionCart `$3000` A/B.
Broader backend, raw-NIR, origin, MADS and full TN runtime matrices remain deferred.
The implementation plan specifies the commands and avoids repeated passing runs.

Required evidence is limited to the changed boundary: two supported callees and
a repeated target; the second callee's distinct or one-lane clobber; an invalid
second call; conditional/loop execution; expanded re-verification and exact
rollback to the transaction's selected baseline. Reuse existing tests for unchanged
exposure, source ownership, ABI and initialization rules. A small independent
source must save final bytes and execute with correct arguments, results and
stack balance. Normalize host fixture text and exercise real LF/CRLF loading.
Keep observable source-memory oracles separate from private scratch contents.

Reuse the qualified 9,791-byte TN baseline from `0880cd2f` during development;
its retained map confirms FindItem at 74 code bytes plus three ordinary-home
bytes. At final qualification, compare the new gate off/on at ActionCart `$3000`
with scoped observation and the other optimized defaults enabled. Record actual
homes, all changed routine footprints, artifact hashes and compiler work. Both
arms remain bounded to one ordinary-call continuation. An unchanged TN result
is acceptable if independent savings and correctness are established; report the
remaining blocker and retain exact fallback.

## Proposed extension: ZP placement through acyclic call chains

The [implementation plan](MIR6502_ACYCLIC_CALL_HOMES_IMPLEMENTATION_PLAN.md)
divides this extension into four separately committed slices with focused checks.

Proposed 2026-11-15; revised 2026-11-16 for implemented value lifetimes and native
current-location callee admission. Implemented and
[qualified](benchmarks/6502-acyclic-call-homes/README.md) on 2026-11-16: enabled
in optimized configurations, disabled in the conservative default. Extend
direct parameter/local residence beyond leaf routines and their immediate
callers. Keep the existing binding tables, per-object call-site liveness,
reservation policy, call ABI and whole-image transaction. Entry-dependent locals
still require retained-value preservation; this extension does not make them
invocation-local. MIR6502 owns the change; no source-language or NIR contract
change is required.

### Motivation and first boundary

The [current qualification](benchmarks/6502-native-current-location-callees/README.md)
records optimized ActionCart TN at `$3000`, **9,811 packaged bytes**, unchanged
by native `=*` leaf admission. The
[lifetime qualification](benchmarks/6502-zp-home-lifetimes/README.md) records
Sort's `h` at `$E2/$E3`, its loop-carried `gap` spill at `$EC`, and
PrintNonzeroDigit's `divisor` at `$A2`, alongside the established small-routine
homes. Sort has 238 instruction bytes and no inline scalar/spill home. Its old
Fnamecmp entry-flag blocker has been resolved.

The motivating chain is `Caller -> Ord -> Next`. Ord already has ZP homes
because Next is a supported leaf. A value in Caller that survives Ord previously
failed the nonleaf preservation check. It can now qualify when the complete
composed entry and write contracts are supported. Values dead at that call, or first
defined afterwards, can already qualify under the lifetime rules. Distinguish
source homes from spills, transient ABI/scratch use and source-owned ZP;
an absent mapped home may also reflect capture/storage elimination.

The extension addresses distinct boundaries:

| Boundary | Prior behavior | Implemented extension |
| --- | --- | --- |
| Native write summaries | Already propagate through acyclic direct calls | Reuse their transitive coverage and directional read/write distinction. |
| `ResidenceCalls::writes` | Certifies ordinary and native `=*` leaves; rejects callees containing calls | Certify supported acyclic native call closures for values live across invocation. |
| `MirCalleeEntryFact` | Analyzes native `=*` actual entries, but native entry analysis still rejects calls | Compose supported nested entry obligations through actual setup and CFG. |
| Call-input admission | Ordinary native dead boundaries can use the existing fallback; native `=*` calls require a complete actual-entry fact | Add composed facts without weakening input checks or removing existing ordinary dead-call eligibility. |
| Placement discovery | Filters selected callee writes per object and call-site lifetime | Also exclude proposed descendant homes at the calls crossed by that object's values. |
| Caller/object eligibility | Mixed inline/ZP homes, supported operations, unexposed scalars, at most four incoming bytes | Retain current ownership, entry-dependence, ABI and operation rules. |

The first new case is `Outer -> Middle -> Leaf`, including several callees,
diamonds, repeated sites and loops within a routine. A newly certified nonleaf
contract requires a supported native call closure with complete entry
requirements and conservative write coverage. This includes native
`ActionObservable` (`=*`) callees through their actual entries. Keep their ABI
classification, entry/patchability rules and own storage policy unchanged;
analyzing their effects grants no new inlining, handoff or home-movement
permission. Traversal uses `RoutineId`, not names or source order. A routine can
supply a callee contract while all its own objects stay inline.

Recursive call components, calls reaching such components, indirect/unresolved
targets, OS calls, machine bodies, barriers, SArgs and unsupported helpers remain
outside the new composition scope. Failure to certify such a closure rejects
the new contract, not every home in its caller. Preserve existing ordinary
dead-call behavior, including its recursive-call controls; it does not grant
recursive preservation. Native `=*` entries still require complete actual-entry
facts even at dead boundaries. Unknown writes cannot preserve a live home.
Unknown pointee reads alone do not invalidate independently complete writes,
but observations, inputs and ordering retain their existing proofs. Source-owned
and ABI storage do not become unobservable.

Call depth is only one remaining TN limit. Copy still fails the pointer-read/
variable-word-arithmetic body guard; its SwapWin call must independently qualify
through the composed entry contract. PopUp encounters Range's five-byte SArgs entry, and its `menu` and
`c` independently have retained-value and address-exposure obligations. Assembly
and OS boundaries retain their current restrictions. Complete write summaries
can still exclude the entire pool: the existing Global-write alias rule is
conservative about every ZP byte. This extension does not repair those separate
limits or promise homes for these routines. No TN-specific admission rule is
permitted.

### Compose entry obligations in the shared analysis

Extend [entry analysis](../src/mir6502/analysis/known_callees/entry_inputs.rs),
not a private approximation inside the selector. Reuse the existing register,
flag and home liveness machinery with supported call transfers. Analyze callees
before callers, caching each result within one immutable program snapshot.
Reject recursion or an unavailable dependency instead of treating it as an empty
input set. The new nested-call proof operates at `ActualEntry`, analyzing each
callee's real expanded capture and body. Preserve the existing `PlannedCapture`
scope; in particular, do not synthesize a native `=*` prologue or broaden
terminal handoff as a side effect. Discovery uses selected actual entries;
final residence checks rebuild those facts from the expanded candidate.

At each nested call, the callee's required registers, individual flags and fixed
bytes are observations at that boundary. Propagate them backwards through actual
argument setup and all CFG paths. A requirement established by the enclosing
routine is not an extra input to that routine; a requirement not established on
every relevant path remains an entry obligation. Keep canonical ABI validation
at every site, including later calls to a previously validated target.

The [call-input and program-state contract](MIR6502_CALL_FACT_CONTRACT_DESIGN.md)
clarifies which observations belong in this entry contract. A typed global read
remains a memory observation even when its backing is ZP; it does not become an
extra caller-supplied argument. Genuine implicit machine inputs retain their
entry requirements, and source-owned storage retains its reservation and
preservation checks.

Preservation, possible writes and definite definitions remain distinct. A
may-write footprint must not kill an incoming-value dependency. Only an existing
certified definition, including a typed ABI result guaranteed on return where
available, can do so. Otherwise retain the dependency or reject the entry proof;
do not infer a result definition from an incidental `$A0` store or a map signature.
Calls do not logically initialize caller locals merely by touching their homes.
Do not export mutable native exit N/Z facts to make an entry proof succeed.

Definite call definitions use the independent declared direct result ABI and an
all-returning-path physical definition proof for each byte. Pointer parameters
do not erase a byte, word or data-pointer result contract. Void, Unknown,
conditional writes and possible clobbers cannot manufacture result definitions;
at least one reachable normal return is required. The
[typed result contract](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#typed-direct-result-contract)
defines this proof without changing the integer-only signature's meaning.

An entry fact that depends on another routine must become stale when that
dependency's body, binding, ABI or relevant backing changes, even if the outer
body is unchanged. Keep the existing source/phase identity checks and include
transitive dependencies, or confine the dependent facts to the immutable owning
analysis context. A routine-ID-only cache shared between selected and trial
programs is insufficient. Extend freshness tests alongside this contract.

### Complete transitive write coverage

Reuse [known-callee summaries](../src/mir6502/analysis/known_callees.rs) and
`MirKnownMemoryWrites::preserves_fixed_byte`. A caller value that survives an
invocation must avoid its entire write footprint: entry capture, working homes,
argument/result delivery for nested calls, allocated temporaries, implicit data
scratch and all descendant writes. A byte restored before return still belongs
to the possible write set. Both lanes of a word or pointer require coverage.
Liveness decides where the footprint constrains a home; it does not narrow the
published footprint itself.

Resolve each routine's bound/virtual homes in that routine's own frame before
using them as physical ZP exclusions. A descendant's `ParamId` or `LocalId` must
never be interpreted in the caller's frame. Preserve existing storage identities
for other memory queries; unresolved physical coverage cannot become a claim of
ZP preservation. Fixed-ZP and absolute spellings alias. Implicit balanced CPU
stack traffic remains governed by the existing stack/ABI contract; explicit
stack-address accesses remain memory effects.

This is a precision extension of the shared facts, not a new alias analysis.
Unknown read footprints stay Unknown. Exact writes grant no permission to reorder
or delete pointee reads, discard observable source stores, or weaken explicit
assembly/ABI observations.

### Coordinate proposed homes in one batch

Simply allowing non-leaf callees is insufficient: two routines could choose the
same apparently unused byte from old summaries, then clobber one another in the
candidate. Coordinate discovery without trial-compiling each call-graph level.

Start from the same immutable selected executable and retained pre-capture input
used by the current ordinary-call trial. Use one deterministic callee-before-caller
traversal of its supported acyclic call graph, with stable routine-ID ordering
between unrelated routines. Build one proposal. Reuse the implemented logical
home analysis for each parameter/local byte and each direct call site; do not
replace it with a routine-wide union. For each candidate object:

- At a call crossed by its value, exclude the selected callee's complete
  transitive writes and every ZP home proposed for that callee or a descendant
  in this batch. If either word lane is live, apply the exclusions to the whole
  home. Missing preservation evidence makes this object ineligible across that
  call, without excluding unrelated objects that do not cross it.
- At a call where the value is dead, add no invocation-preservation exclusion
  for it. Its last argument read precedes invocation effects; a later source
  definition begins a new value lifetime. Still check call inputs and actual
  argument setup, explicit observations and expanded physical conflicts.
- Keep the existing source-owned, scratch, ABI and same-routine object
  reservations. Preserve the exclusion of return slots at dead calls: reusing
  them needs a separately proved source-value/result handoff.

The additional exclusions are a small per-routine set of planned resident bytes,
propagated upwards by union. They are placement constraints, not published callee
effects. Keep selected writes even if a proposed home appears to make an old
write unnecessary. This conservative rule avoids depending on predicted cleanup
and does not need a placement fixed point. Already selected bindings stay fixed;
only eligible remaining inline objects are considered by their existing path.

Sibling routines may reuse a byte when their invocations do not overlap; there
is no global unique-home assignment. An ancestor value live across either
sibling must avoid its writes and proposed descendant homes. A value dead at
that call may share those bytes under the existing checks, even if the same
object receives a new value later. Keep one fixed authoritative home per object
and conservatively disjoint source homes within a routine. This uses existing
value lifetimes without adding region-specific homes, save/restore sequences,
backing copies or a new allocator.

For `Caller -> Ord -> Next`, a caller value live across Ord excludes the writes
of both Ord and Next, plus any proposed homes in that subtree. A caller local
defined after Ord has no preservation obligation for that call. If Caller calls
Ord twice and the value survives only the second site, only that site's
invocation footprint constrains it. Validate inputs at both sites even when the
target's summary is cached.

Proposed-home exclusions do not predict every change in generated scratch or
entry capture. After the single core continuation, rebuild contracts from the
expanded candidate and verify every resident caller, including deeper chains
and routines whose own bindings did not change. A new hidden clobber or entry
requirement rejects the batch. Never publish the discovery exclusions as proof
that the emitted candidate preserves a byte.

### Selection, layout and acceptance remain one transaction

Add nested-call candidates to the existing combined ordinary-call proposal with
the established single/multiple-leaf callers. This section extends the previous
leaf-only discovery rule and specifies callee-before-caller order for the new
scope. It does not add a trial per depth, a second ordinary-call batch, a subset
retry or a separate preliminary compile to preserve an unmeasured subset's gains.
`cross_call_home_trials` remains zero or one; an empty proposal adds no continuation.

The qualified `enable_acyclic_call_homes` discovery gate is subordinate to the existing
peephole, parameter-home, cross-call and value-lifetime gates. The multiple-call
gate continues to govern the allowed number of sites. With the new gate off,
retain the previous selection behavior and ordering. The optimized default is
enabled after qualification; the conservative default stays disabled.

Final acceptance requires valid bindings, actual entry/argument supply at each
site, expanded lifetime verification, authoritative physical layout and replay
of every earlier consumed preservation obligation. Facts and certificates from
the selected program cannot substitute for proofs about the candidate. Keep
bindings fixed during terminal cleanup; layout changes still require the existing
physical alias checks and preservation replay.

Require a strictly smaller complete packaged image. Report individual routine
growth without reinstating the old per-routine veto. Include captures, copies,
helpers, branch expansion and removed inline storage in final cost, and report
ZP occupancy separately. Unknown cost, equality, growth or any failed proof
restores the complete selected artifact entering this trial: program, bindings,
bytes, maps, relocations, reservations and facts. The new batch can fail as a
whole; this extension makes no promise to retain hypothetical subset savings.

### Further widening follows measured blockers

The completed [fact-driven admission implementation plan](MIR6502_FACT_DRIVEN_HOME_ADMISSION_IMPLEMENTATION_PLAN.md)
qualifies precise rejection reasons, existing helper contracts, global/static
address formation and bounded constant word shifts in separate slices. The
[qualification](benchmarks/6502-home-admission/README.md) records useful
placements and the remaining arithmetic, write-footprint and overlap limits.
Explicit runtime helpers are now admitted through their selected invocation
contracts in the cross-call/lifetime path, including helper-only routines.
Planning accounts for helper scratch and actual result homes; the resumed
candidate rebuilds its own contracts and residence proofs. No helper fact
grants purity or narrows full observations to successful-return effects.
Word `LeaAddr` of a `Global` or `Static` is also qualified through its existing
structured address materialization. This does not expose unrelated source
objects or establish pointee bounds. Numeric observations, exposed-object
exclusions and final physical alias checks still apply.
For scalar integer arithmetic, the planner uses the materializer's constant
multiply and unsigned division/remainder reductions. It checks the resulting
operations under the ordinary admission rules. Native word shifts include
counts 0–4, lane projections for 8–15, and zero results for counts of 16 or
more. BYTE shifts accept counts representable by the immediate operand.

For multiplication, division, remainder and shifts that call a helper, the
planner uses the backend's typed selector and ABI staging. It matches the
ordered helper identities in each retained block to the selected executable
and validates each invocation against its bound implementation. This covers
widening byte products, narrow divisors, shared quotient/remainder calls and
shifts used as call arguments. Fixed argument bytes are reserved during setup,
including when the source dies at the call. The successful-return footprint
constrains homes live across the call. The trial replays the original routine
with its proposed bindings and checks its expanded code and complete image.

Spill discovery excludes resident source bytes across all routines and numeric
observations available before layout. Final verification resolves the old and
new spill locations against the complete source-storage inventory.

Retain the mixed bindings and per-object exposure/lifetime filtering that are
already implemented. Exposed or entry-dependent siblings can stay inline;
their presence does not itself reject an eligible scalar. Preserve current
ordinary pointee-write admission and the final physical-observation proof.
Broader call depth adds no new storage shape or native operation.

Separate qualified follow-ups may:

1. Certify additional operations, including the currently excluded combination
   of pointer reads and variable word arithmetic. Account for helpers and scratch
   that appear only after expansion.
2. Extend placement of a native `=*` routine's own homes through the relevant
   ownership and ABI checks. Availability of a callee contract alone does not
   authorize this change.
3. Admit supported machine/helper boundaries only through explicit, complete
   contracts for the relevant entry, writes, preservation and observations.
   Existing clobber/effect declarations may supply these facts; do not invent a
   manual-annotation language or pointer-range inference for TN as part of this
   extension.

More than four incoming bytes, new ABI/result coalescing, recursive residence,
within-routine home coloring, split-home regions, retained-value reservations,
save/restore insertion and loop register allocation remain deferred. Existing
per-call value-lifetime checks are part of this extension, not a deferred
optimization. The proposed smaller Ord register sequence is independent of
expanding ZP homes. No larger ZP pool is assumed.

### Focused evidence and diagnostics

Keep the user's minimal-testing scope. Use grouped checks of the changed shared
analyses and small independent execution controls, then one final TN gate-off/on
comparison. Reuse passing coverage for unchanged ownership and ABI rules.

- Entry/write composition: a three-level chain, a diamond and repeated/conditional
  calls, with ordinary and native `=*` middle routines; an extra fixed-byte or
  carry input supplied on all paths versus one path; typed result delivery versus
  a mere possible write. Include recursive and unsupported descendants and a
  changed grandchild with an unchanged outer body. Native `=*` actual-entry
  composition must leave planned observable capture and its own home policy
  unchanged.
- Placement/lifetimes: parent and child both proposed, a grandchild's one-lane
  clobber, sibling reuse, setup/result interference, loop-carried values and
  mixed eligible/exposed/entry-dependent objects. Cover a value dead before a
  nested call, one defined after it, and one live across it; repeat the same
  target with only one crossed site. Check actual caller/descendant sharing at
  a dead boundary and preserve the existing ordinary unknown/recursive dead-call
  fallback. Force an expansion-time conflict and check exact rollback.
- One VM group: a live byte/word or pointer across nested calls, page crossing,
  canonical arguments, host-computed results, observable memory sentinels and
  stack balance. Use default/optimized ActionCart and a small Standalone control;
  normalize and exercise LF/CRLF when fixture instrumentation depends on newlines.
- One final TN ActionCart `$3000` A/B: use the same compiler/source/configuration
  except for the new gate. The qualified 9,811-byte image is a reference, not an
  oracle for future revisions; record baseline drift explicitly. Retain Ord,
  Items, FindItem, Strcpy, Instr, Fnamecmp, Next, Sort and PrintNonzeroDigit as
  controls. Measure callers of Ord and other supported nonleaf routines; report
  remaining body, entry, write and capacity blockers in larger routines.

Extend existing diagnostics with the failed routine/site/dependency, object and
crossed call site, entry or write blocker, proposed descendant exclusions,
actual selected homes, all changed routine footprints, complete image size and
artifact hashes. Distinguish
unsupported body/call, unknown facts, source/ABI conflicts, ZP exhaustion and
cost rejection. Count program-level trials and record one compile-time sample;
do not add a benchmark matrix or repeated timing runs. Gate-off/on programs
retain zero or one ordinary-call continuation each.

A successful independent nested-call control must both execute correctly and
save final bytes. TN savings are measured rather than promised. If an otherwise
eligible TN caller remains inline, record the actual limiting contract instead
of widening effects or weakening the transaction to force placement. Broaden
checks only for an affected contract or observed failure; this documentation
change itself requires content/link checks, not compiler test runs.

## Proposed extension: certified assembly leaves in acyclic call chains

Implemented 2026-11-17 for [issue 9](MIR6502_FACT_REPAIR_PRIORITIES.md), following
the [implementation plan](MIR6502_ASSEMBLY_LEAF_COMPOSITION_IMPLEMENTATION_PLAN.md).
The acyclic-call contract accepts assembly leaves already certified by the
shared analysis. This widens coverage within MIR6502; it adds
no source syntax, annotation language, NIR form or assembly instruction support.
The initial extension supersedes the blanket machine-callee exclusion only at
the certified boundary described here. Machine routines' own homes remain under
their current ABI and storage rules.

### Existing evidence and the missing connection

Before this extension,
[acyclic entry composition](../src/mir6502/analysis/known_callees/entry_inputs/acyclic.rs)
rejected every `MachineBlock` before consulting the existing leaf entry producer.
[ResidenceCalls](../src/mir6502/parameter_homes/calls.rs) separately rejected
machine callees in both input and write queries. Both consumers now use the
certificate within the acyclic scope defined below.

The [audit](MIR6502_FACT_CONTRACT_AUDIT_2026-11-16.md#assembly-leaf-knowledge-does-not-compose)
and the issue-8 selected TN inventory distinguish three cases:

| Callee | Existing evidence | Consequence for this extension |
| --- | --- | --- |
| Independent `asm_leaf` / `Fetch` control | Certified A/X entry, finite writes, Unknown pointee reads | Its leaf entry can participate in composition; result-definition limits below still apply to a wrapper returning its value. |
| TN `MovePage` | Certified entry: A/X/Y and `$A3/$A4`; Unknown writes | Compose entry requirements and admit a correctly supplied dead boundary; do not preserve a live caller home across Unknown writes. |
| TN `CalcAdr`, `Internal`, `Ascii` | Useful finite writes, no complete leaf entry certificate | Remain outside this new admission rule. Finite clobbers cannot replace the missing entry proof. |

An input certificate, a complete may-write footprint and a guaranteed result
definition answer different questions. Keep partial knowledge available without
turning any one of these answers into the other two.

### Reuse the leaf producer as a terminal call-graph node

When the acyclic walker encounters a machine routine, request its existing
`MirCalleeEntryFact::analyze(..., ActualEntry)` certificate. On success, cache
that fact as a terminal dependency and compose it into native ancestors through
the existing call transfers. Do not feed a machine body through the native
`analyze_with_dependencies` path, even with an empty dependency map: that path
currently rejects machine bodies deliberately. Do not duplicate the decoder in
the walker or selector.

The certificate must cover the complete actual routine. The current
[bounded producer](../src/mir6502/analysis/known_callees/bounded_memory.rs) admits
one closed machine block with supported typed operands; its
[entry walk](../src/mir6502/analysis/known_callees/bounded_memory/entry_inputs.rs)
checks reachable control flow and requires a supported return. Keep those shape,
instruction, relocation and work-budget limits. A routine is not a certified
leaf merely because it has no `MirOp::Call`: raw JSRs, unresolved transfers,
unsupported instructions, mixed native/machine bodies, opaque or OS effects and
failed decoding remain unsupported. Native recursion and missing dependencies
retain their existing rejection.

The same ordinary Action and `ActionObservable` call ABI checks apply. An `=*`
declaration does not waive certification or make an entry compiler-owned. Keep
`PlannedCapture`, terminal handoff, spill discovery, inlining and movement of an
assembly routine's own parameters/locals outside this extension. The existing
acyclic discovery scope enables this admission; leaf-only residence contexts
retain their current behavior. No additional trial per assembly leaf is needed.

### Entry admission and live-value preservation

Replace the unconditional machine-callee rejection in the acyclic residence
context with the shared certificate requirement. Both queries use the same
actual-entry evidence, but retain different obligations:

| Query | Required evidence |
| --- | --- |
| Entry composition | Current certified leaf inputs, propagated through native setup and CFG paths. |
| `ResidenceCalls::inputs` | Valid target ABI, canonical arguments and matching declared result at this particular site; certified inputs satisfy the existing supply check. |
| `ResidenceCalls::writes` | Successful input admission plus the current complete may-write summary; every byte crossed by a live value must be preserved. |

An uncertified machine body must not enter through the ordinary native
missing-entry fallback, including at dead boundaries. Conversely, a certified
entry with Unknown writes need not reject objects dead at the call or first
defined afterwards. Preserve the existing per-object, per-site lifetime and
setup/result-conflict checks. A missing write proof still excludes every object
whose value crosses that invocation.

Revalidate canonical arguments and declared results at every call site even
when its target certificate is cached. At a residence boundary, retain the
current supply predicate: extra flags, stack requirements or fixed/register
inputs outside the outgoing canonical lanes fail. This extension does not add
a setup-value solver. Within native entry composition, the existing backward
transfer may discharge a child's requirement established on every relevant
path. A typed global remains program-state observation; a literal scratch read
retains its implicit-input role, even at a coincident global address.

Use the existing known-callee write producer and backing-aware preservation
query. Include transient writes and restored bytes, both pointer lanes, and all
descendant writes. Retain the existing manual-clobber contract where that
producer already accepts it; a declaration does not prove entry reads, result
definitions or closed control flow. Do not promote call-context pointer bounds
from a terminal cleanup proof into a program-wide assembly summary. Unknown
reads remain Unknown and retain their ordering/observation constraints; they
do not erase independently complete writes.

### Result definitions remain an explicit limit

The bounded assembly entry path currently publishes an empty `defined_results`
set. Preserve that answer in the first extension. A declared BYTE/CARD/pointer
result, a finite write to `$A0/$A1`, or a known exit-A relation is not an
all-returning-path definition certificate.

Consequently, a native wrapper that reads an assembly result may still acquire
an extra fixed-byte input and fail residence admission. The existing `Fetch`
control must expose that limitation rather than have it hidden by a fabricated
result definition. Use a void assembly PROC and native wrappers with no
dependency on an assembly result as the first positive placement control.

A separate narrow follow-up could export typed result-byte definitions from the
bounded walk's existing definite-definition state at every reachable return.
It would need the [typed result contract](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#typed-direct-result-contract)
and conditional/no-return negative controls. It is not required to connect the
already-certified entry and write facts in this first scope.

### Preserve observations, ownership and transaction boundaries

The existing leaf fact retains the routine, machine payloads, backing and entry
stage. Native ancestors retain it as a dependency. Changes to that payload,
effects, ABI, binding or relevant backing invalidate ancestors even when their
own instructions are unchanged. Cache successful and unavailable answers only
within the immutable owning program; reconstruct contexts for expanded
candidates. Keep issue 8's
[compatibility and replay obligations](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#summary-validity-across-transformations).
Certification must not bypass the core fallback or validate earlier rewrites
retroactively.

Certified assembly leaves contribute their selected physical writes to the
existing callee-before-caller proposal. They receive no proposed home moves of
their own. Retain descendant exclusions, source-owned ZP, ABI reservations and
sibling reuse. The MIR-only planning graph is scheduling information, not proof
that a machine payload contains no hidden call; the leaf certificate supplies
that boundary proof.

Keep explicit machine reads/writes and storage references in the existing
[physical observation checks](../src/mir6502/parameter_homes/physical.rs).
An entry certificate does not make an absolute read unobservable or a referenced
source home movable. Unknown observations and final-layout conflicts retain
their current handling. Check expanded residence for all affected callers and
preservation against the candidate's own map.

Use the same single combined ordinary-call proposal and continuation. Acceptance
still requires complete proofs and a strictly smaller packaged image, with no
per-routine size veto. Failure restores the whole selected artifact. Do not
add per-leaf retries, another allocator or a private-alias exception. Recovering
issue 8's 115-byte TN fallback cost is independent work; assembly admission does
not repair `Pull`'s expanded write contract.

### Focused acceptance and reporting

Keep testing focused on the connection and its consumers:

- Shared analysis: direct and nested native calls to a certified assembly leaf,
  including a diamond/repeated target. Leaf and acyclic entry answers agree;
  result definitions remain empty. An unsupported opcode, raw call, open branch,
  opaque/OS effect or changed machine payload must not acquire a certificate.
  Check stage/backing/descendant invalidation and canonical arguments at a second
  call site. Reuse existing decoder tests for unchanged instruction coverage.
- Residence: a finite scratch-write leaf preserves a disjoint live word and
  rejects either overlapping lane. An entry-only leaf with Unknown writes
  allows a dead lifetime but rejects a live one. Retain Unknown-read barriers,
  explicit observation conflicts and rejection of uncertified machine entries
  even when writes are finite. Exercise exact rollback after a candidate conflict.
- One small VM group: a void assembly leaf under native wrappers, with live
  caller byte/word values, observable sentinels and stack balance. Cover both
  runtimes, default/optimized controls and actual LF/CRLF source loading. Require
  a positive independent placement example to execute correctly and save bytes;
  do not depend on an incidental TN saving.
- One final TN/independent-control comparison: record entry and write admission
  separately, selected homes, complete image size, changed routines and hashes.
  The issue-8 reference is **9,926 bytes**, not the historical 9,811-byte image;
  compare the pre-extension artifact under identical configuration and report
  any later baseline drift. Retain the existing acyclic gate-off output as a
  scope control; that gate also controls native chains, so its full size delta
  must not be attributed solely to assembly admission.
  Expect `MovePage` to retain its live-write blocker and the uncertified TN
  entries to remain unsupported unless separately repaired. Record trial counts
  and one compile-time sample; no full suites by default.

Diagnostics should distinguish missing leaf certification, unsupplied inputs,
unknown writes, overlap/observation, missing result definitions and cost rejection.
Include the target and failing site/dependency using existing trace facilities;
do not add a second fact registry.

### Qualified outcome

The [qualification receipt](benchmarks/6502-call-facts-audit/assembly-leaf-composition-qualification.json)
records 162 focused unit checks, two VM groups and the 14-control/95-routine
audit. The independent void-assembly chain shrinks from **88 to 81 bytes**.
TN shrinks from **9,926 to 9,921 bytes**: `Tag.i` moves to `$A2` and `Tag.a`
to `$A3`, saving two inline bytes and three instruction bytes. `IsTagged` can
compose `Value`'s certified entry and preserve `Tag`'s live home; the later
`Store` call retains Unknown writes and crosses only dead candidate values.

The existing 13 controls retain their sizes. `MovePage` retains Unknown writes;
`CalcAdr`, `Internal` and `Ascii` still lack leaf entry certificates. Assembly
result definitions remain empty. The one combined ordinary-call trial and all
six issue-8 core fallbacks remain; no extra per-leaf trials were introduced.
