# MIR6502 parameter home selection

Status: leaf placement and terminal parameter handoff are implemented and qualified.
Enabled for optimized builds; the conservative configuration remains disabled.
The [entry-to-call handoff extension](#proposed-extension-parameter-handoff-to-call-arguments)
below has its own gate, subordinate to parameter placement and peepholes.

The completed [leaf implementation plan](MIR6502_PARAMETER_HOME_SELECTION_IMPLEMENTATION_PLAN.md)
covers the leaf scope. The [handoff implementation plan](MIR6502_PARAMETER_ARGUMENT_HANDOFF_IMPLEMENTATION_PLAN.md)
records the completed slices and qualification gates for the extension.

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
cost and preservation checks. For this size-focused policy, require no growth
in affected routine code plus ordinary homes and a strict complete-image saving.
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
checks. The cost decision records changed routine costs as well as image size;
an unrelated routine regression rejects the complete batch.

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
a strictly smaller packaged image and no routine growth in code plus ordinary
homes relative to the leaf-qualified fallback. Any unsupported expansion,
conflict, stale obligation, equality, or growth restores that complete artifact,
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
