# LET code-generation follow-up

Baseline: [LET code-generation audit](LET_CODEGEN_AUDIT.md), committed as
`93e03b8`. The baseline CSV is historical; these are general NIR/MIR changes,
not LET-specific rules. No source-name or immutability matching is involved.

## Slice 1: static sources in indexed byte copies

The existing transactional `indexed-byte-copy` candidate now reuses ordinary
static byte indexing for its source after preparing the destination pointer.
It emits `LDA table,Y` instead of constructing a second indirect pointer.
Destination preparation, captured index dependencies, memory access order,
candidate proof checks and allocator liveness remain in the existing pipeline.

Eligibility requires a resolved static source base, byte elements and an index
accepted by existing byte-index selection. Delayed byte arithmetic is evaluated
with byte wrapping before indexing; it is not folded into the table base.
Word indexes, scaled elements and explicit physical-register inputs retain
the general path. This also improves the generic static-to-static fallback:
its destination arithmetic remains explicit when the adjacent-copy range proof
is unavailable. Two unit expectations intentionally reflect this new shape;
no NIR contract or snapshot changed.

Measured global-pointer mutable relay: **85 bytes / 76 cycles → 73 / 59**.
The fresh-local and LET forms still measure 83 / 69 pending slice 2. The
parameter-indexed mutable relay remains 76 / 65; other audit probes are unchanged.
These are XEX sizes and VM CPU cycles, not PAL frame timings.

Validation: 2,848 compiler tests, NIR snapshots and 38-fixture sweep, 167-fixture
MIR sweep, all 1,296 audit executions, and a new 1,008-execution VM regression.
The regression compares raw and optimized lowering with both runtimes, two
origins, byte wrapping, word indexes, page crossings and writes overlapping
the captured source index, destination index and pointer cells.

## Remaining slices

2. Extend the bounded byte-relay profitability tier to connected single-pair
   homes using the existing storage analysis, def-use facts and SSA promotion.
   Retain the cold-home guard for isolated, long-lived or barrier-crossing values.
3. Keep volatile-boundary dead-store elimination separate. Atari locals have
   static backing; immutability does not prove that storage is unobservable.
   Do not weaken the barrier without a reusable alias/effect proof.
