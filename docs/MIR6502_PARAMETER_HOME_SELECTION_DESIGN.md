# MIR6502 parameter home selection

Status: implemented and qualified, 2026-11-14. Enabled for optimized builds;
the conservative configuration remains disabled.

The [implementation plan](MIR6502_PARAMETER_HOME_SELECTION_IMPLEMENTATION_PLAN.md)
defines the dependency order, commit boundaries, and qualification gates.

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

[Core materialization](../src/mir6502/materialize.rs) currently reserves pointer
scratch, allocates virtual ZP, prepends the ABI parameter prologue, and constructs
an initial layout. Later passes forward incoming registers, elide write-only
homes, or coalesce a narrow leaf word parameter with the result pair.

The [prologue and parameter passes](../src/mir6502/materialize/abi.rs) ordinarily
capture arguments into inline parameter storage. General ZP promotion targets
spills, not these parameter objects. Furthermore,
[parameter availability](../src/mir6502/analysis/param_availability.rs) learns
register/home equality from capture stores; entry starts with no such facts.
Removing the captures without changing that analysis would lose useful facts.

The current [Fnamecmp source](../samples/tn/modern/TN.ACT) illustrates the cost:
two mutable pointer parameters become four inline bytes, repeatedly copied into
pointer pairs. Measurements at revision `8039b30b`, MIR6502/ActionCart, origin
`$3000`, gave:

| Variant | Routine code | Inline parameter bytes |
| --- | ---: | ---: |
| Current compiler | 185 | 4 |
| Handwritten, working pairs `$A0/$A1` and `$A2/$A3` | 47 | 0 |
| Handwritten, working pairs `$AE/$AF` and `$AC/$AD` | 51 | 0 |

The handwritten variants also use shared Y induction, comparison flags, fewer
pointee reads, and shared return tails. They are direction-setting examples,
**not placement-only size targets**. Their prior VM comparison covered ordinary
RAM buffers, byte comparisons, colon handling, page crossings, and Y rollover;
it did not establish arbitrary volatile-read or callee-preservation equivalence.
The first slice must report its own emitted savings.

## One identity, three different roles

Retain parameter identity as `(RoutineId, ParamId, byte offset)`. Its source
name remains diagnostic metadata. A parameter holding a pointer is distinct
from the pointee, and mutation creates a new value of the same parameter.

Each routine has one parameter binding table, with three separate components:

| Component | Contract |
| --- | --- |
| Incoming ABI locations | Canonical byte positions supplied by the caller; immutable description of entry. |
| Working home | Inline storage, register lane(s), reserved ZP byte/pair, or no home when all accesses are proven unnecessary. |
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
