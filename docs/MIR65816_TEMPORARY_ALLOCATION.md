# Native 65816 temporary allocation and stack pressure

## Baseline and reproduction

The inspected main revision is `90bd73e`, rather than Exec816's `c2268b7` pin.
Main already allocates eligible pointer-only leaves in three DP slots. Its
general emitter still reserves a distinct stack home for every MIR temporary,
followed by dedicated parallel-edge staging slots.

The native `stack_allocation` execution target supplies input 13 after linking,
checks independently specified results in both IRQ entry states, and measures
the actual emitted bytes on the qualified VM. Code size sums all routines;
cycles include the independent caller; observed stack use is entry S minus the
lowest S over the complete call chain, including arguments and return addresses.
These are representative compiler probes, not measurements of hosted Exec DOS.

| Probe | Mode | Code bytes | VM cycles | Observed stack bytes | Worker fixed frame |
| --- | --- | ---: | ---: | ---: | ---: |
| Scalar chain | raw | 586 | 891 | 52 | 36 |
| Scalar chain | optimized | 271 | 381 | 22 | 6 |
| Loop rotation | raw | 544 | 2867 | 60 | 44 |
| Loop rotation | optimized | 560 | 2857 | 54 | 38 |
| Recursive sum | raw | 539 | 5950 | 346 | 18 |
| Recursive sum | optimized | 527 | 5596 | 290 | 14 |
| Wide indirect call | raw | 834 | 1687 | 94 | 54 |
| Wide indirect call | optimized | 756 | 1503 | 62 | 26 |

The existing unlink probe confirms main's DP path: raw 191 bytes / 324 cycles /
6 frame bytes; optimized 129 / 189 / 0. Its cycles cover routine entry through
RTL, a different measurement interval from the table.

Reproduce with:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 tools/native65816-runtime-tests/qualify.py \
  --test stack_allocation --test pointer_allocation -- --nocapture
```

## Implemented slice: reuse private stack temporaries

MIR65816 owns allocation from typed def/use facts and CFG edges. Fixed-point
backward liveness includes loops, edge arguments, block parameters, indirect
call targets, address bases/indexes and return values. Deterministic aligned
stack homes are reused only for noninterfering temporaries. An operation's
inputs, outputs and other live values coexist for its entire machine sequence:
bytewise casts, pointer formation and carry chains must not overwrite a dying
input early. This includes dead results and dead block parameters because
emission still writes them. The existing source-saving parallel-copy staging
area stays separate from all temporary homes.

Automatic frame objects, mutable parameters and addressable locals retain their
dedicated homes. Only non-addressable MIR value temporaries share bytes. No
pointee-load forwarding, memory reordering or new alias assumption is needed.
Every value live across a direct, indirect, recursive or helper call stays on
its invocation's stack. The allocation is rechecked before selection, including
physical byte overlap, widths, frame ownership and accounting. Incoming
displacements, spill bytes and local peak derive from the final allocation;
emission preserves last-byte access checks, the 254-byte even-frame strategy, call guards and
platform interrupt reserve. Public ABI v1 and image v3 remain unchanged; image
maps can already give several temporary IDs the same physical location.

Qualification executes actual raw and optimized machine code for loops/parallel copies,
recursion, direct/indirect calls, helpers, mixed widths, aliasing and stack
boundaries, including an assembly callee destroying all scratch and registers
while the caller has a live value. The existing two-task IRQ-at-each-site and
seeded IRQ/NMI suites cover suspension during reused-slot operations.

## Compact edge staging

The [staging-reservation slice](MIR65816_STAGING_RESERVATIONS.md) makes allocation
and emission share the existing word-copy planner. Empty, single-word and
acyclic word edges require no staging. The selective-staging extension captures
only cyclic word sources overwritten by earlier original-order assignments.
Whole-word, disjoint destination geometry is required; partial overlaps keep
complete word staging. Mixed-width and unsupported word forms keep their
complete bytewise path.

The shared plan identifies captured move indices separately from pool slots.
For captured moves [1, 4], slots [0, 1] hold the two saved words. Allocation
reserves each capture ordinal's maximum requested width across all explicit
edges, including both branch arms and unreachable blocks. There are no holes,
source deduplication or scratch reuse within an edge. Full fallbacks capture all
arguments, preserving their argument-index mapping. Multi-byte slots keep even
alignment, and final frame extent stays even.

The planner first uses the minimum frame containing all fixed objects and temp
homes. Immutable incoming parameters are above that extent; adding staging cannot
create an overlap with a destination. Mutable parameters use their fixed object
homes. Final allocation recomputes staging requirements and validates slot count,
exact capacities, logical-to-physical capture mappings, disjointness, incoming
last-byte access, spill bytes and local
peak. This avoids rejecting a legal direct edge because a provisional unused
reservation would exceed the addressing limit. Required reservations and the
254-byte frame limit remain checked.

Temp homes and their closed-operation interference rule do not change. Staging
remains invocation-owned memory, separate from all live values and addressable
objects. Helper/call clobbers, aliasing and task preemption retain their existing
contracts. The public ABI and stack guards are unchanged; guard reservation
immediates and incoming displacements derive from the new final extent.

## Compatible edge homes

A bounded pass follows the verified first-fit allocation. It keeps every block
parameter as a fixed anchor and tries each direct word edge's noninterfering
source-temp affinities together, in stable block/arm order. A trial must pass
whole-routine stack verification, retain exact frame/spill/peak and staging
accounting, and reduce copy cost without increasing bytes or cycles on any edge.
Third-party conflicts, repeated-source ambiguity or accounting changes reject
the whole trial. The closed-operation interference rule is unchanged.

Emission omits direct physical self-copy stores and unnecessary loads while
retaining full A/N/Z of the final logical assignment. A final self-copy needs an
LDA; an all-self edge therefore still has a real instruction boundary. This
changes only private temporary homes and edge traffic, preserving non-edge
captures, addressable objects, ABI parameter placement, guards and preemption.
See the [implementation plan](MIR65816_EDGE_COALESCING_PLAN.md).

## CPU register and DP opportunities

The 64-byte scratch area is per execution domain, but all of it and A/X/Y are
call-clobbered. The general selector currently uses these byte ranges:

| D-relative bytes | Selector use |
| --- | --- |
| 0..2 | Address formation and indirect-call target |
| 3..5 | Aggregate-copy source pointer |
| 8..11 | Shift workspace and scalar return marshalling |
| 16 | Arithmetic/comparison right operand |
| 20..22 | Scaled index |
| 24..26 | Saved aggregate-copy destination |
| 28..30 | Aggregate-copy count |

The bounded pointer-leaf selector has a separate, verified scratch whitelist
and uses 0..2, 3..5 and 6..8. Its third slot overlaps general result scratch.
It cannot simply be enabled around other operations. Bytes 31..63 are not used
by today's selector, but any ordinary callee may destroy them.

Further allocation needs explicit per-operation and helper scratch/register
clobber contracts, with homes split or spilled before calls. Initially allocate
only values whose entire lifetime fits between barriers, preserving the full
operation's input lifetime. A/X/Y retention additionally needs width/flags and
addressing constraints; X/Y and A are already working registers within selection.
Using otherwise unused DP bytes is plausible after that audit, but does not
eliminate the need for invocation-owned homes across calls or recursion.

Preemption safety depends on the existing platform contract: each task and IRQ
domain owns its own DP, the bridge restores the full CPU state, and NMI preserves
the interrupted scratch without calling Action! or switching tasks. Neither DP
allocation nor register retention permits shared scratch between suspended
domains or removal of interrupt headroom. Stack reuse reduces pressure without
extending those contracts. Exec816 adoption still needs image-v3 packaging and
hosted call-chain qualification; this slice does not change its pin or budget.

## Measured result

The same probes, input, caller, VM and entry modes after stack reuse:

| Probe | Mode | Observed stack before → after | Worker frame before → after |
| --- | --- | ---: | ---: |
| Scalar chain | raw | 52 → 22 | 36 → 6 |
| Scalar chain | optimized | 22 → 22 | 6 → 6 |
| Loop rotation | raw | 60 → 34 | 44 → 18 |
| Loop rotation | optimized | 54 → 42 | 38 → 26 |
| Recursive sum | raw | 346 → 206 | 18 → 8 |
| Recursive sum | optimized | 290 → 206 | 14 → 8 |
| Wide indirect call | raw | 94 → 70 | 54 → 30 |
| Wide indirect call | optimized | 62 → 50 | 26 → 14 |

Code size and VM cycles in the baseline table are unchanged: this slice changes
stack displacements and reservation sizes, not instruction selection or memory
traffic. The optimized loop retains dedicated edge staging and is therefore
larger than its raw counterpart despite other optimized code improvements.
The existing pointer-leaf measurements also remain unchanged.

The native target enforces these whole-call-chain stack ceilings and checks an
additional 160-update scalar sequence with more than 160 raw MIR temporaries in
a frame of at most 10 bytes. The previous one-home-per-temporary strategy cannot
represent that sequence. A separate independent assembly callee destroys all 64
scratch bytes and A/X/Y across both direct and indirect calls while wide values
remain live. Allocation regressions reject overlapping live byte ranges,
incorrect accounting, frame-object/staging collisions and incoming bytes beyond
255. No existing NIR snapshots or source fixtures change.

The [qualification record](abi/action65816-stack-allocation-qualification.json)
binds the source inventory, measurements, VM inputs and saved artifact hashes.
All 37 native tests pass in debug and release, including 2,504 raw / 2,352
optimized general IRQ sites and the unchanged pointer-leaf IRQ/NMI corpus.
All eight saved context images disassemble, and the 32 saved artifacts have
identical hashes across host build modes. NIR snapshots and all 51 sweep
fixtures pass; ABI generation and formatting checks pass.

Compiler checks finish with 3,265 passed, 24 ignored and one unrelated sample
failure: the pre-existing untracked `samples/vbxe/shared/lines.act` cannot find
`SHARED.SCREEN` under the generic sample loader's module paths. An isolated
baseline compiler with the same current sample files reproduces that failure.
The test targets after the failing target were run separately and pass. Local
sample edits, untracked files and deletions were left intact.

## Bounded scalar DP residents

After verifying and coalescing the ordinary stack allocation, the emitter may
promote complete word-home classes to D+$20..D+$3F. The pointer-leaf strategy
still runs first. Admission examines every block, including unreachable code:
ordinary 16-bit integer temps, compare-produced stack Booleans, native ADD/SUB,
unsigned ordering or word equality, direct nonvolatile non-addressable word
frame/parameter memory, word edges, and word/void returns. Calls, helpers, casts,
address formation and unknown selectors reject the entire transaction.

At most 16 aligned word classes move, in ascending original stack-offset order.
All existing home equalities and closed-operation interference remain intact.
Capacity failure retains the entire verified stack allocation. Fixed objects,
parameters and Boolean offsets remain; required cyclic staging is repacked on
the stack. Incoming displacements, exact spill extent and local peak reflect
only retained stack storage. The mixed-location verifier independently rechecks
admission, liveness, geometry, staging, bounds and accounting; `verify_stack`
continues to reject any DP home.

This partition belongs to selection, not a new ABI preservation promise.
The whole 64-byte scratch remains call-clobbered and belongs to the current
aligned task/IRQ domain. No scalar resident crosses a call or changes D. The
nonreturning stack-fault path remains checked even for zero-frame routines.
See the [frozen inventory](MIR65816_SCALAR_DP_INVENTORY.md) and
[implementation plan](MIR65816_SCALAR_DP_PLAN.md).
