# MIR6502 Home-Elision Plan

Snapshot date: 2026-07-19.

## Problem

MIR6502 currently performs useful value propagation before temp
materialization, but each residual temp lane is still converted into a spill by
`materialize_temp_ops`. Later dead-store removal, spill coloring, and zero-page
promotion can find a better physical home or remove an unused one, but they
cannot recover all of the code spent storing and reloading a transient value.

Home elision and home allocation are separate concerns:

- home elision avoids creating storage for values that can remain in a
  register, reach their consumer directly, or be rematerialized cheaply;
- home allocation assigns the remaining unavoidable homes to shared RAM or
  zero-page storage using liveness and interference.

The intended pipeline is:

```text
target-specific MIR expansion
  -> CFG temp liveness
  -> home planning and elision
  -> materialize residual temps as the conservative fallback
  -> spill cleanup, coloring, and RAM/ZP allocation
```

Home planning belongs after target-specific MIR expansion and liveness, but
before `materialize_temp_ops`. It is a MIR6502 target-strategy decision and must
not move into NIR.

## Goals

- Make a temp home opt-in rather than the default for every residual temp.
- Remove spill stores, reloads, and storage bytes together when possible.
- Retain short-lived byte values in A/X/Y when clobber and ABI constraints
  permit it.
- Rematerialize cheap constants and addresses instead of preserving them in
  memory.
- Preserve existing profitable word-store, carry/borrow, pointer, and indexed
  addressing combines.
- Leave existing temp materialization as a correct conservative fallback.
- Produce separate metrics for homes avoided and physical homes allocated.

## Non-goals for the first phase

- General register allocation across arbitrary joins and loops.
- Phi construction or register-state merging at CFG joins.
- Register or memory-value preservation across calls without structured
  effects.
- Elision of address-taken, absolute, hardware, externally visible, or
  ABI-observable source storage.
- RAM/ZP interference coloring; that follows home elision and operates on the
  smaller set of unavoidable homes.

## Slice 1: Home-demand census

Status: implemented on 2026-07-19.

Add byte-lane-granular observability without changing generated code. For every
temp lane that reaches the home boundary, record:

- definitions, uses, and whether it is single-use or multi-use;
- same-block, cross-block, terminator, join, and backedge uses;
- whether it is live across a call or machine block;
- whether its producer naturally leaves the value in A, X, or Y;
- the first relevant register clobber between definition and use;
- unsupported consumer, word-lane, carry/borrow, and profitability blockers;
- estimated spill stores, reloads, and storage bytes attributable to the home.

Report, per routine and in aggregate:

- virtual temp lanes reaching the boundary;
- homes requested and allocated;
- home stores and reloads;
- each reason a home was retained;
- candidates for register retention, forwarding, and rematerialization.

This slice is instrumentation only and should be committed independently.

The accepted TN modern/MIR6502 census is taken after existing copy propagation
and immediately before `materialize_temp_ops`:

- 501 residual temp byte lanes have 501 definitions and 529 uses;
- 73 lanes are conservative same-block, single-use A-residency candidates;
- the mutually exclusive primary retention reasons are 228 coupled lanes, 109
  terminator uses, 32 multi-use lanes, 27 unsupported consumers, 15 accumulator
  clobbers, 12 values live across calls, four unused lanes, and one cross-block
  lane;
- the gross boundary traffic is 501 stores plus 529 reloads, with a nominal
  3,090-byte absolute-addressing ceiling that deliberately ignores later
  folding and is not an achievable saving estimate;
- final materialization and coloring leave 171 logical temp homes: 119 virtual
  ZP cells and 52 ordinary spill cells.

The census also reports overlapping CFG exposure counters for joins, backedges,
calls, machine blocks, and explicit barriers. The primary retention counters
partition every residual lane exactly once, while the exposure counters show
all applicable constraints.

TN remains byte-identical: both the 13,348-byte load file and the generated
listing match the accepted pre-census artifacts.

## Slice 2: Explicit home planning

Status: implemented on 2026-07-19.

Introduce an internal analysis keyed by temp ID and byte lane. Its conceptual
result is one of:

```text
ElideInRegister(register)
Rematerialize(value)
ForwardToConsumer
MustMaterialize(reason)
```

Initial `MustMaterialize` reasons include:

- call or machine-block barrier;
- join or backedge;
- register clobber;
- address escape or observable storage;
- incompatible consumer;
- word-lane or carry dependency;
- rewrite would displace a more profitable combine;
- rewrite is not smaller than materialization.

The plan is internal MIR6502 analysis, not a new NIR form. Rewriting consumes
accepted decisions; any residual `VTemp` is passed unchanged to
`materialize_temp_ops` and receives the existing conservative spill treatment.
Commit the analysis and reason reporting before enabling broad transformations.

The implemented `HomePlan` contains exactly one decision for every residual
temp lane. Its stable decision vocabulary includes register elision,
rematerialization, direct consumer forwarding, and mandatory materialization
with a typed reason. Slice 2 deliberately populates only the decisions already
proved by the Slice 1 census; rematerialization and forwarding remain reserved
for their profitability slice.

For TN, the plan contains 501 decisions:

- 73 `ElideInRegister(A)` candidates;
- 428 `MustMaterialize` decisions, partitioned into 228 coupled lanes, 109
  terminator uses, 32 multi-use lanes, 27 unsupported consumers, 15 accumulator
  clobbers, 12 values live across calls, four unused lanes, and one cross-block
  lane.

The planner records aggregate and per-routine `home-plan-*` counters. It does
not yet rewrite MIR, but returns the plan at the temp-materialization boundary
for Slice 3 to consume. The 13,348-byte TN load file and listing remain
byte-identical to Slice 1.

## Slice 3: Same-block single-use accumulator residency

Implement the first behavior-changing rule for a byte value when:

- it has one definition and one use in the same block;
- the definition precedes and dominates the use;
- the producer naturally leaves the value in A;
- no intervening operation clobbers A;
- the consumer accepts A directly;
- the value is not live into the terminator.

Rewrite the producer and consumer to use `MirDef::Reg(A)` and omit the spill
store and reload. Initially exclude:

- calls and machine blocks;
- joins, loops, and cross-block uses;
- word operations and coupled byte lanes;
- carry/borrow chains;
- indirect-store shapes protected by structural combines.

Add focused positive tests for load, move, arithmetic, compare, call-argument,
and store consumers as they become supported. Add negative tests for every
barrier and clobber class. Commit this slice independently and measure TN.

## Slice 4: Profitable rematerialization and forwarding

Avoid a home when reproducing the value at the consumer is cheaper than
preserving it. Start with:

- constants;
- static addresses;
- byte lanes extracted from known constants;
- immutable direct values only when alias barriers prove them unchanged.

Use an explicit cost rule comparing rematerialization with spill stores,
reloads, and storage cost. A rewrite must not disable an existing more
profitable combine. In particular, preserve:

- `word-array-store-value-staging`;
- `binary-word-store-producer-forward`;
- compact carry/borrow word updates;
- scaled indexed-address patterns.

Calls, runtime helpers, machine blocks, and possibly aliasing stores invalidate
rematerializable memory values. Constants and static addresses do not require
memory facts and can survive ordinary stores. Commit rematerialization as a
separate slice.

## Slice 5: Multi-use block-local retention and lazy materialization

Extend register residency within one block:

- calculate the last use of each candidate;
- retain a value while its assigned register remains valid;
- allow multiple compatible consumers;
- materialize lazily immediately before the first consumer that cannot use the
  register-resident or rematerialized value;
- omit the home entirely if no consumer requires it.

Start with A. Add X and Y only for constrained, profitable roles such as byte
indexes, and give ABI and pointer-index requirements priority. Keep word values
out of the initial slice; add word pairs only after byte residency is stable.

## Slice 6: Straight-line CFG propagation

Carry a register-resident or rematerializable value across a block edge only
when:

- the successor has exactly one predecessor;
- the predecessor has exactly one successor;
- the edge is not a backedge;
- the definition dominates every use;
- no call, machine block, or register clobber intervenes;
- required flags and registers agree at the boundary.

Do not initially merge register states at joins or synthesize phi-like copies.
Commit cross-block support separately from block-local scheduling.

## Slice 7: Internal source-storage promotion

After compiler-temp home elision is stable, apply the same principle to
storage-backed internal scalars:

- unused internal parameter homes;
- direct-ABI parameter copies;
- non-address-taken scalar locals;
- non-escaping private scratch storage.

Initially exclude absolute and hardware storage, address-taken or
pointer-escaped variables, arrays, records, initialized persistent storage, and
externally or ABI-observable homes. Begin with single-block parameters and
locals, then extend only when dominance and CFG liveness prove the promoted
value. Commit parameter and local promotion separately.

## Interaction with home allocation

Home elision answers whether a value requires storage. It must run before
physical home selection. The remaining homes are then inputs to a separate
allocator that can:

- color non-interfering homes onto shared ordinary RAM;
- select hot eligible homes for a shared caller-clobbered ZP pool;
- preserve contiguous pairs for words and pointers;
- keep values live across calls out of caller-clobbered pools.

Ordinary RAM pooling saves storage but normally not load/store instructions.
ZP placement can additionally save a code byte and cycles per access. Neither
substitutes for avoiding the home and its traffic in the first place.

## Validation and commit discipline

For every behavior-changing slice:

1. Add focused positive and negative unit tests.
2. Verify that protected word-store and carry/borrow combines remain enabled.
3. Generate and measure the TN modern/MIR6502 listing and executable.
4. Compare homes, stores, reloads, spill cells, spill accesses, and code bytes.
5. Reject or narrow a slice if TN grows without a justified broader win.
6. Run:

   ```sh
   cargo test
   cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
   ```

7. Commit each coherent major slice separately.

The primary success criterion is fewer homes together with fewer memory
accesses. Physical pool size and ZP-weighted access cost are allocator metrics
and should be reported separately.
