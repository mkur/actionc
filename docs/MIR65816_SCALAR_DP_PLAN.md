# Scalar DP inventory and measured implementation plan

Status: slices 1–3 implemented. Inventory `3af8799` froze the forecast against
qualified main `e790349`; prerequisite `0298e49` preserved all saved images.
The bounded allocator matches all 28 frozen image transforms and all 264 debug
measurement records. Full cross-profile qualification is the remaining slice. See the
[quality plan](MIR65816_CODE_QUALITY_PLAN.md) and
[planning evidence](benchmarks/65816-scalar-dp-plan/baseline.json).

## Objective and boundaries

Use the existing call-clobbered, per-domain DP scratch for private 16-bit MIR
values in a bounded class of call-free scalar routines, including loops. Reduce
stack traffic and actual frame reservations while preserving native word
selection, existing forwarding and parallel-copy semantics.

Keep physical ABI v1, image v3's tagged location schema, o65 profile v1, public
argument/result placement, stack checks, interrupt reserves and Exec816's pin.
The proposed scratch partition is an internal selector contract within the
existing 64 bytes, not a new callee-preserved ABI resource. No NIR or semantic
changes, source-variable promotion, X/Y residency, cross-call allocation,
spilling around calls, general graph coloring or new coalescing are included.

## Evidence collected for this plan

The existing read-only movement exporter recompiled all 28 counted Action
builds and matched their complete saved images. Its 56 additional LF/CRLF
compilations also matched. Qualified evidence was rechecked: 432 current
compiler/fixture inputs, 626 native artifacts in each host profile, all 224
comparison artifacts and the identical 264 debug/release records. The known
optimized vbcc `unlink` failure remains in those records.

The preliminary filter finds **14 candidate routine instances in 14 builds**
among 30 counted routine instances. They contain 84 static private-word memory
instruction sites and need only one to four existing word-home classes each.
Candidates are identity, add, subtract, constant-chain and maximum in both
modes; optimized rotation and sum-loop; and the call-free helper inside both
direct-call builds. The calling worker remains stack allocated. Raw rotation
and sum-loop contain casts and are outside the initial whitelist. Other
rejections include calls, wider values, pointer operations and indirect memory.

These are preliminary candidates, not completed typed admission proofs. The
current exporter does not expose every type, cast, addressability or scratch
fact required below. The inventory slice must export those facts and account
for all routines, including uncounted startup/wrapper code.

For each candidate, the planning calculation maps each distinct existing word
stack home to an aligned DP word, preserving all home equalities. It counts
actual memory instructions using the saved per-PC execution counts; forwarded
loads that do not exist are not counted. On the qualified VM, with the ABI's
aligned D, the admitted A16 stack memory instructions cost five cycles versus
four for their DP equivalents. Both encodings occupy two bytes. The forecast
preserves all other selected instructions except existing zero-frame teardown
elimination, and retains all guards.

| Representative case | Code bytes, measured → forecast | Cycles, measured → forecast | Observed stack peak, measured → forecast |
| --- | ---: | ---: | ---: |
| Identity, either mode, 13 | 59 → 51 | 63 → 49 | 4 → 0 |
| Add/subtract, either mode, 13/41 | 70 → 62 | 90 → 72 | 8 → 0 |
| Constant chain, optimized, 13 | 65 → 57 | 73 → 58 | 6 → 0 |
| Constant chain, raw, 13 | 155 → 147 | 223 → 193 | 6 → 0 |
| Maximum, either mode, 13/41 | 90 → 90 | 94 → 90 | 6 → 2 |
| Rotation, optimized, 13 | 130 → 130 | 896 → 793 | 16 → 8 |
| Sum loop, optimized, 13 | 120 → 120 | 1,212 → 1,092 | 12 → 6 |
| Direct calls, either mode, 13/41 | 319 → 311 | 466 → 436 | 20 → 14 |

These are **conditional forecasts**, not executions of modified code. They
depend on retaining every current forwarding decision and instruction order.
For zero-frame routines, the existing `release(0)` behavior omits six teardown
instructions, eight bytes and 13 cycles per return. Direct-call totals include
two executions of the smaller helper; its own frame changes from six to zero.
The other 14 counted builds are expected to retain their code. Whole-image
checking must still account for shifted routines and relocated references.

The sum-loop forecast has an especially simple accounting oracle:

- Lift S+$06, S+$08 and S+$0A to D+$20, D+$22 and D+$24 respectively.
- Eleven static word instructions execute 120 times at input 13: 40 reads and
  80 writes. Expected stack-byte traffic is 165→85 reads and 188→28 writes;
  scratch DP traffic becomes 80 reads and 160 writes.
- Keep the mutable parameter at S+$02, its entry copy, and the Boolean home at
  S+$05, even though the fused comparison does not write that Boolean. The exact
  fixed extent therefore becomes six bytes; incoming displacement is recomputed.
- Preserve the distinct arithmetic input/output homes and the backedge copy.
  This slice saves one cycle per changed instruction, not the instruction itself.

Rotation similarly moves 51 word reads and 52 word writes at input 13: stack
bytes 123→21 reads and 136→32 writes. Its four DP classes preserve the two
existing coalesced identities. The cyclic backedge retains one stack staging
word, repacked from S+$0E to S+$06, giving an eight-byte frame.

## Slice 1: read-only inventory and exact forecast

Add a dedicated test-only typed exporter and comparison report, proposed as
`tests/mir65816_scalar_dp_inventory.rs` and
`tools/compare65816/inventory_scalar_dp.py`. Reuse the existing verified compile,
image-equality, MIR-span and LF/CRLF paths. Do not infer eligibility from names,
source text, formatted IR or a disassembly opcode alone.

Export stable routine/block/temp/object/parameter IDs; exact scalar types and
widths; operation and comparison kinds; call/helper requirements; volatile,
indexed and addressable facts; entry/result contracts; CFG edges and typed
arguments; closed-operation live points/interference; current stack/DP homes;
fixed objects, incoming homes, staging requirements and frame accounting.
Include unreachable blocks and unused block parameters. Export structured
target effects for each admitted operation and prologue/epilogue, including
scratch reads/writes, width requirements, register/flag effects and barriers.

Classify every routine with explicit acceptance/rejection reasons. Link every
candidate access to typed ownership and final instruction bytes. Count dynamic
reads/writes by joining saved instruction-site counts; distinguish parameter,
object, private-temp, staging, selector scratch and guard metadata traffic.
Report static bytes, cycles by vector, DP capacity, retained forwarding counts
and exact frame/peak accounting separately. Never add incompatible alternatives
or count an elided load as a DP saving.

Dry-run the proposed home-class mapping against an independent reconstruction
of the unchanged closed-operation graph. Check its sources, all destinations,
successor live-ins, dead Boolean homes, byte extents and staged cycles. A new
typed candidate validator may run in tests, but no altered frame goes to the
production selector in this slice.

Save a fresh inventory under `docs/benchmarks/65816-scalar-dp-inventory/`, keeping
the planning evidence and historical snapshots immutable. Freeze a complete
expected image transform: DP opcodes/operands, frame immediates, incoming and
staging displacements, optional zero-frame teardown removal, maps, labels,
branches, fixups and moved routines. Derive all-vector counter deltas from the
saved execution sites. Extend the independent decoder for address-space-aware
copies; the old movement report predates coalesced self-copy omission and must
not be run unchanged against this baseline.

Completion: all 28 original Action images and 264 records remain equal; every
candidate and rejection has typed evidence; forecasts cover complete images
and every vector, and the report does not claim any unexecuted improvement as
a measured result. Commit inventory/tool tests and the frozen implementation
forecast before changing production selection.

## Slice 2: typed word homes and DP state tracking

Prepare the consumers before enabling scalar allocation. Use an explicit word
home that distinguishes stack from DP; source operands additionally admit
immediates. A numeric offset alone cannot identify storage. Stack displacements
include temporary S movement; DP offsets are relative to the unchanged D.
Validate both bytes before emitting anything.

Update `emit/copies.rs`, `select.rs`, `accumulator.rs` and `parameter.rs` so the
existing native ADD/SUB, compare/fused-branch, load/store, return and edge paths
accept checked DP word homes. Generalize destinations as well as sources.
Dependency, overlap, self-copy and staging checks compare address space plus
byte range. A stack offset equal to a DP offset is neither an alias nor a
self-copy. Preserve direct/selective/complete scheduling, capture order and
the final logical A/N/Z repair. Staging remains on the invocation's stack.
Make copy costs use the actual operand/home kind rather than a fixed five-cycle
memory cost.

Extend the tracker with address-space-tagged private-home identity, generations
and exact overlap invalidation. Register scalar DP homes explicitly; D must be
the current aligned domain and remain fixed. A checked resident DP store can
record A/N/Z without destroying an unrelated frame witness. An unmodelled write,
call or domain change remains a conservative barrier. Keep the existing
adjacency/cursor rules; labels still discard value permissions. Allocation
across a loop is justified by CFG liveness, not by carrying tracker permissions
through the join.

Apply tagged homes to adjacent-temp and incoming-parameter capture witnesses;
incoming reads and frame-object witnesses remain stack based. Preserve the
current frame, parameter and temp forwarding decisions for admitted routines.
Extend proof snapshots and the native oracle to read DP at `D + offset` and
stack homes at the frame anchor; never interpret all snapshots as stack homes.
Leave pointer-leaf tracking/selection behavior unchanged.

Completion: scalar allocation is still disabled; existing machine bytes, maps,
counter values and trace-on/off output are identical. Focused tests establish
DP word encoding, flag effects, partial-overlap invalidation, same-offset
different-space handling and mixed stack/DP copies. Commit this independently
qualified prerequisite.

## Slice 3: bounded scalar DP allocation and accurate frames

Keep the existing pointer-leaf path first. For the general path, build and
verify the ordinary stack allocation and existing coalescing result, then
attempt one deterministic scalar-DP transaction. Admission applies to the
whole routine:

| Admit initially | Keep on existing path |
| --- | --- |
| Private CARD/INT word temps and word block parameters | Address, pointer, wide or arbitrary byte residents |
| Nonvolatile, unindexed word loads/stores of proven non-addressable frame objects and parameter homes | Indirect, external, static, indexed or volatile memory; address formation/escape |
| Native word ADD/SUB and currently native-supported word comparisons | Casts, unary/bitwise/shift operations, signed ordering and unmodelled selectors |
| Compare-produced byte Booleans with stack homes | Byte promotion or removal of unused Boolean homes |
| Verified branches/gotos/fallthrough, word edges, word/void returns | Calls of every kind, helper calls, machine blocks and unsupported exits |

Initially restrict declared scalar parameters and fixed objects to supported
word forms as well. A leaf helper called by another routine is eligible; a
routine containing a call is not. Do not treat the absence of an explicit MIR
Call as sufficient: selected operations, runtime paths and adapters must also
be accounted for. The nonreturning stack-fault transfer is retained and cannot
resume with live DP values. Reject unknown effects conservatively.

Reserve **D+$20..D+$3F** for up to 16 aligned resident words. Assert this entire
range lies inside the ABI scratch extent and outside selector workspaces,
pointer-leaf slots and domain metadata. D+$1F is deliberately unused. Ordinary
callees may still destroy the entire scratch region; this is no preservation
promise. Admission must fail if a future selector requires overlapping scratch.

Promote complete two-byte home classes from the verified, coalesced stack map,
in ascending original-offset order. All TempIds sharing a word home move
together, preserving existing affinities, identities and edge dependencies.
This reuses the established allocation instead of adding a second coloring
heuristic. Keep closed-operation input/output interference unchanged. If the
classes do not fit, preserve the complete original stack allocation; partial
promotion, pressure ranking and spills are deferred. Require at least one
useful eligible class and a nonregressing checked selection plan.

Leave fixed objects, parameters and retained byte-temp offsets intact. Repack
only required stack staging after the highest retained stack byte. Recompute
the even frame extent, actual stack spill extent, incoming displacements and
local peak. Do not retain fictitious stack slots for DP temps or globally relax
the exact-accounting verifier. A dedicated mixed-location verifier must check
all types, IDs, byte extents, DP ownership, interference, effects, edge plans,
staging disjointness and argument bounds before selection. Keep `verify_stack`
strict for the all-stack fallback; malformed input must not be hidden by a
fallback attempt.

Preserve the same stack-check algorithm with updated reservation operands,
including zero-frame entry checks. Use the existing zero-frame teardown behavior.
Do not alter I, interrupt headroom, checked call transfers or fault ordering.

Extend image validation explicitly: retain the existing size-three pointer-slot
case and add only aligned size-two homes fully inside the scalar pool, without
calls. Reject gaps, odd offsets, end overruns, metadata overlap, invalid widths
and mixed allocation families not admitted by the allocator. The existing v3
tagged schema already represents these locations; no transport schema or ABI
change is planned. Older validators reject the new maps and require a matching
compiler update; this plan does not imply Exec816 compatibility or adoption.
Final maps establish geometry, not liveness. o65 keeps D-relative operands
literal and allocates no application scratch in its zero segment.

Completion: the selected routines meet the frozen exact transform and predicted
traffic/frame deltas; all rejected routines keep their strategy, allowing only
necessary address relocation when a neighboring routine shrinks. Commit the
production slice with its targeted regressions and contract updates.

## Slice 4: independent execution and saved qualification

Use raw and optimized emitted serialized images, plus independently verified
target probes for shapes the frontend removes. Cover at least:

- Word limits, signed ADD/SUB bit patterns, all admitted compare predicates,
  both branch arms, repeated iterations, unused parameters and successor live-ins.
- Direct, reordered, self/all-self and cyclic edges; mixed stack/DP sources;
  immediate tails and final A/N/Z repair; distinct physical spaces at equal
  offsets. Preserve closed-operation conflicts and all logical edge counters.
- One and 16 resident classes, 17-class fallback, deterministic home reuse,
  malformed pool geometry, scratch collisions and frame-boundary displacements.
- Rejection of calls/helpers, recursion, volatile/aliased/escaped storage,
  casts and unsupported widths; unchanged pointer-leaf artifacts. Exercise
  stack callers retaining values across a helper that clobbers all 64 bytes.
- Independent ca65 encodings and VM checks for DP word instructions; tagged
  home generations and A/N/Z; trace-on/off bytes, labels and fixups identical.
- Two simultaneously live task domains with different resident values, and
  scalar code in IRQ dispatch on its separate domain. Inject IRQ/NMI at every
  retained instruction while values and arithmetic flags are live, including
  edges and staged cycles. Compare full CPU and live stack/DP bytes with an
  uninterrupted step; include seeded schedules and both incoming I states.
- Flat images and two o65 placements, zero-frame and reduced-frame guard faults,
  argument/result ABI, immutable metadata, and unchanged interrupt reserves.
  Repeat newline-sensitive fixture paths in an isolated CRLF checkout.

The exact comparison gate must check all complete images and 264 records,
including uncounted code and unchanged vbcc artifacts. Retain existing
forwarding/coalescing counters with their logical meanings, allowing declared
PC relocation; report resident DP traffic separately from selector scratch and
guard metadata. Require no unexpected new bytewise fallback, load, mode switch
or memory access. Keep the known vbcc failure visible.

Run affected emitter/proof and image/ABI/CLI/o65 integrations, focused inventory
tool tests, the corpus generator check, and full native qualification in debug
and release. Inventory-only and byte-identical prerequisite commits need their
affected checks, not every suite. Final compiler qualification uses the existing
native runner and saved before/after corpus workflow. NIR snapshots, sweep and
full root tests become required only if the scope crosses those contracts.

Save results under a new scalar-DP benchmark directory and qualification JSON;
update the allocation/emission/state-tracker contracts and quality-plan baseline.
Commit qualification separately. Preserve every historical snapshot and unrelated
local path; do not update Exec816's pin in this work.

## Reproduction of the planning baseline

From the repository root, with the preserved edge-coalescing corpus available:

```sh
A816_COMPARISON_MANIFEST="$PWD/target/edge-coalescing-after/manifest.json" \
A816_MOVEMENT_FACTS="$PWD/target/scalar-dp-plan-facts.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --test mir65816_movement_inventory -- --ignored
```

The committed planning JSON records hashes, candidate home classes, exact
instruction substitutions, representative measurements and aggregate execution
counts across vectors. Its conditional projections require the stronger typed
inventory and final-image checker in slice 1 before implementation. If those
checks change the admitted subset or invalidate a forecast, revise and commit
the forecast explicitly rather than weakening its acceptance gate.
