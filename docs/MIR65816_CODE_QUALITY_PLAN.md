# Native 65816 code-quality improvement plan

Status: refreshed on 2026-09-21 against main `86fcef2`, after qualification of the
native state tracker. Direct single-word edge copies, adjacent accumulator
forwarding and the tracker foundation are complete. The next proposed slice is
short conditional MIR dispatch (3c); checked MIR-entry width omission (3a) and
terminal fallthrough (3b) are [implemented and qualified](MIR65816_CONTROL_FLOW.md). The
[implementation plan for slices 3a–3c](MIR65816_CONTROL_FLOW_IMPLEMENTATION_PLAN.md)
defines the initial site inventory, bounded changes and separate acceptance gates.

## Objective and current baseline

Preserve the public ABI and reduce internal data movement, then improve control
flow and register use. Current main already has CFG-aware stack-slot reuse and
a restricted pointer-leaf DP allocator. General lifetime-based stack reuse is
an existing capability. See the
[temporary-allocation contract](MIR65816_TEMPORARY_ALLOCATION.md).

Use the qualified [state-tracker snapshot](benchmarks/65816-state-tracker/after/tables.md)
as the working baseline for new forecasts. Its
[equality report](benchmarks/65816-state-tracker/equality.json) confirms identical
code and all 264 measurement records against the preceding forwarding snapshot.
The tracker introduced proofs without changing generated code or register lifetimes.
Keep each historical snapshot immutable.

Both compilers implement a general unsigned 16-bit sum loop; input 13 is supplied
at runtime, and both return 91. Measurements run from function entry through RTL,
including Action's stack guards and excluding caller setup:

| Mode / compiler | Code bytes | VM cycles | Additional stack bytes |
| --- | ---: | ---: | ---: |
| Optimized actionc | 140 | 1,395 | 16 |
| Optimized vbcc | 22 | 344 | 0 |
| Raw actionc | 164 | 1,767 | 14 |
| Raw vbcc | 32 | 533 | 4 |

The current optimized [Action listing](benchmarks/65816-state-tracker/after/sum_loop.optimized.actionc.lst)
uses native word arithmetic, direct single-word edge copies and adjacent A16
forwarding, while retaining stack homes and stores. The
[vbcc listing](benchmarks/65816-state-tracker/after/sum_loop.optimized.vbcc.lst)
retains the counter in X and the sum in DP. This supports later allocation work;
it does not justify changing public argument placement or removing stack guards.
Use the full corpus, including calls, pointer traffic and wider values, to choose
and qualify general improvements.

## Completed milestones and historical measurements

| Milestone | Completed scope | Evidence |
| --- | --- | --- |
| Empty-edge cleanup | Compact transfers without unnecessary edge-copy setup. | [Results](MIR65816_EMPTY_EDGES.md) |
| Direct single-word edge copies | Bypass staging for one checked word assignment; retain multi-value paths, reservations and guards. | [Results](MIR65816_SINGLE_WORD_EDGE_COPIES.md) |
| Local accumulator forwarding | Adjacent eligible word operations reuse A16 with matching private-home and N/Z facts; retain stores and homes. | [Results](MIR65816_LOCAL_ACCUMULATOR_FORWARDING.md) |
| State-tracker foundation | One typed emission boundary owns instruction effects, execution modes, stack equations and the existing forwarding witness. No additional optimization. | [Results](MIR65816_STATE_TRACKER.md), [qualification](abi/action65816-state-tracker-qualification.json) |

The original roadmap used the
[empty-edge snapshot](benchmarks/65816-empty-edges/after/tables.md). The measured
progress for `sum_loop(13)` is:

| Qualified stage | Raw bytes / cycles | Optimized bytes / cycles |
| --- | ---: | ---: |
| Empty-edge cleanup | 174 / 2,032 | 154 / 1,735 |
| Direct single-word edge copies | 174 / 2,032 | 146 / 1,595 |
| Adjacent accumulator forwarding | 164 / 1,767 | 140 / 1,395 |
| State-tracker foundation (current baseline) | 164 / 1,767 | 140 / 1,395 |

The observed stack peak remains 14 bytes raw and 16 bytes optimized through these
stages. The original direct-copy forecasts and selection details remain in its
[detailed plan](MIR65816_SINGLE_WORD_EDGE_COPIES_PLAN.md); the
[measured results](MIR65816_SINGLE_WORD_EDGE_COPIES.md) confirmed those forecasts.
They are completed work, not forecasts for the next slice.

## Ordered remaining slices

Implement and measure each slice separately. Keep the original roadmap's later
step numbers, splitting its former combined control-flow step into 3a–3c:

| Order | Improvement | Initial scope |
| --- | --- | --- |
| 3a (complete) | Checked MIR-entry width omission | Qualified results and the fresh 3b baseline are in the [control-flow results](MIR65816_CONTROL_FLOW.md). Value/flag barriers, branches, copies and allocation are preserved. |
| 3b (complete) | Jumps to adjacent blocks | Qualified terminal fallthrough preserves edge assignments, branch selection and width policy. Its [snapshot](benchmarks/65816-control-flow/3b/after/tables.md) is the 3c baseline. |
| 3c (next) | Short-branch selection | Use final placement, displacement and bank proofs with compatible fixups/o65 relocation and a long-transfer fallback. Keep allocation and copy scheduling fixed. |
| 4 | Parallel-copy scheduling and coalescing | Separate self-copy removal and direct scheduling from cycle staging, reservation shrinking and later home coalescing. Qualify each part independently. |
| 5 | Scalar DP allocation | Extend allocation to a verified, call-free scalar subset with loops and explicit scratch/lifetime constraints. |
| 6 | X/Y residency across loops | Retain suitable scalar values across basic blocks only when selection honors their live-register, width and clobber constraints. |

Broader local private-word forwarding is a **measurement candidate**, described
below. Its position is not fixed ahead of 3a: count useful sites and executions
before promoting it to an implementation slice. The
[tracker design's stages](MIR65816_STATE_TRACKER_DESIGN.md#staged-implementation-and-acceptance)
describe the additional capabilities; this roadmap prioritizes width omission
for the next measured plan. Revisit that priority if the inventory shows little
benefit or a materially stronger candidate.

These are native MIR65816 strategy and emission changes. Consume verified typed
facts; do not recover semantics from source strings or SemIR. If a later slice
needs stronger NIR facts, introduce and verify those in a separate boundary
change before relying on them.

## Checked MIR-entry width omission: completed scope

The tracker distinguishes proved execution width from permission to omit
a mode-setting instruction. Its byte-identical foundation revoked permission at
every label; qualified slice 3a now grants it only at checked MIR entries.
See the [emission contract](MIR65816_EMISSION_CONTRACT.md). The historical tracker
measurements above remain the start of this sequence; use the qualified 3a
snapshot for 3b forecasts.

Follow the [3a–3c implementation plan](MIR65816_CONTROL_FLOW_IMPLEMENTATION_PLAN.md)
within these limits:

1. Inventory redundant mode requests in raw and optimized emitted code. Separate
   MIR block entries from guard, comparison, shift/copy-loop and indirect-resume
   labels. Record candidate PCs and dynamic counts before changing selection;
   no new numerical saving is assumed here.
2. Prove the entry's native E/M/X contract from the ABI or every incoming MIR
   edge, including loop backedges. Check actual emitted exits against it. Keep
   unknown or incompatible entries on the existing explicit mode-setting path.
3. Initially remove only redundant `REP #$20` at proved A16 MIR entries. Do not
   grant omission permission to arbitrary internal labels or byte-mode joins.
   Preserve routine guards, call/return normalization and indirect-resume behavior.
4. Retain label barriers for register values, flags, homes and the single-use
   forwarding witness. Do not broaden forwarding or combine the slice with jump
   cleanup, copy scheduling, frame shrinking or register allocation.
5. Predict exact instruction/byte/cycle changes, including resulting address and
   fixup adjustments. Require unchanged data-access traffic, frames, guards,
   stack peaks and existing forwarding/copy/fusion execution counts. Validate
   serialized and relocated code, flag behavior and preemption at affected PCs.

The output of the inventory must be a bounded, reviewable slice with independent
expected counts. Freeze a fresh baseline for each later optimization; do not
subtract cumulative forecasts from this snapshot without measuring the intervening
changes.

## Measurement candidate: broader local forwarding

The state tracker can describe more than the current adjacency policy permits.
Inventory private-word reloads separated by reviewed effects that preserve the
exact A16/home relation and required N/Z, such as CLC/SEC or a proved disjoint
private store. Classify barriers and count reached sites across the full corpus;
a plausible instruction pattern alone is not evidence of a useful saving.

A first implementation would keep the current producer/consumer classes, stores,
homes and zero stack displacement. Calls, helpers, joins, source-memory barriers,
volatile accesses and possibly aliasing writes stay conservative. No flag-dead
exceptions or source-memory caching belong to this candidate. Qualify any newly
extended live-register/flag interval under IRQ/NMI suspension.

Give broader forwarding its own proof index and exact traffic accounting. Keep
the existing adjacent-span index's meaning unchanged; neither tracker decisions
nor the compiler's candidate list may serve as the expected-result oracle. If
there are no useful qualified opportunities, leave the candidate deferred.

## Proof obligations for later slices

**Control flow.** Remove jumps only when control reaches the intended successor
directly, including any required edge assignments. Short branches need range and
bank checks after layout, compatible relocation handling, and long-transfer
fallbacks. Keep guard behavior and fault paths intact. Width omission, fallthrough
and branch relaxation have separate acceptance gates.

**Copy scheduling and coalescing.** Prove physical byte-range compatibility and
preserve swaps, cycles, repeated sources, mutable parameter homes and successor
live-ins. Account for A/N/Z effects when eliminating edge loads, as well as their
memory traffic. Retain a checked staging fallback. Do not globally weaken the
current closed-operation interference rule. Remove reservations only after
selection no longer writes them, then recompute frame extent, incoming
displacements and stack-guard accounting.

**DP and register allocation.** Use the tracker's explicit instruction effects,
then define the allocator's lifetime and scratch reservations. Observing a value
in a register does not reserve that register against later selector use. Allocate
only approved DP ranges; apparently unused scratch bytes alone are not a contract.
Begin with call-free scalar routines and a conservative operation whitelist.
Keep values spanning ordinary or helper calls on their invocation's stack and
retain fallback emission for unsupported operations. Mutable parameters and
addressable locals need separate storage and alias proofs before promotion.
Add X/Y residency only when selection can honor addressing, width and clobber
constraints across every edge.

**Preemption.** Preserve the existing domain-owned DP and complete CPU-state
restoration contract. Live scratch may survive asynchronous suspension only
under that contract. Qualify interruptions while resident values and arithmetic
flags are live. Keep IRQ/NMI isolation and interrupt reserves unchanged.

## Validation and completion criteria

For each slice:

1. Preserve the previous snapshot and declare expected instruction and traffic
   changes before implementation. Retain the 14-pair / 66-vector corpus as the
   comparison core and add general regression cases, never sample-specific rules.
2. Check exact selection, rejection and fallback cases. Include boundary words,
   frame displacements, both branch arms, backedges, cyclic copies and malformed
   plans as relevant. Keep independent ca65 encoding checks for new sequences.
   Use immutable state traces as claims to verify against executing code, never
   as the VM's input or the expected-result oracle.
3. Execute serialized raw and optimized machine code through the
   [qualification runner](../tools/native65816-runtime-tests/README.md) in debug
   and release hosts, with both incoming interrupt-mask states. Cover clobbering
   direct/indirect/helper calls, aliasing and volatile traffic, IRQ/NMI suspension,
   relocated o65 code, recursion and stack-boundary failures as affected.
4. Record code bytes, cycles, stack depth and stack/DP traffic. Permit only the
   explicitly predicted changes; require unchanged unaffected artifacts and
   behavior. Keep byte-identical refactors on the
   [strict equality gate](../tools/compare65816/check_state_tracker.py). Give
   optimizations their own independently checked deltas. Recheck LF/CRLF corpus
   builds and retain the known external vbcc optimized unlink failure rather
   than exempting it.
5. Run the compiler checks required by the affected contracts. NIR changes also
   require the contributor-mandated snapshots, sweep and full compiler suite.
   Update the relevant contract, save qualification evidence and commit the
   completed slice while preserving unrelated local changes.

Preserve [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md), image v3, the
[o65 profile](MIR65816_O65_PROFILE.md), public argument/result placement and stack
guards. Frame sizes may shrink only with verified accounting. No slice reduces
the platform's interrupt headroom or changes Exec816's compiler pin; adopting
new compiler output in Exec816 remains a separate integration qualification.

The width/control-flow and copy slices target stack-resident code. DP and X/Y
allocation address the larger remaining gap to vbcc. Set further numerical
targets from each new baseline rather than promising cumulative gains before
implementation.
