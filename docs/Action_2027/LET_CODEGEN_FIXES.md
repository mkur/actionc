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

## Slice 2: connected single-definition byte relays

The existing NIR bounded-relay selector now admits connected single-pair homes.
It shares interval classification with reused-home relays and follows existing
single-use def-use facts into the next store's source. The original SSA renamer
performs promotion; no new optimizer pass or IR operation is involved.
See the [extension contract](../NIR_BOUNDED_SCALAR_RELAY_PROMOTION_PLAN.md#connected-home-extension-contract)
for the exact bounds, barriers and eligibility rules.

Both table probes now eliminate all staging homes, loads and stores for reused
mutable locals, fresh mutable locals, and LET. Their MIR6502 output agrees in
both runtimes:

| Probe | Baseline LET bytes / cycles | After both fixes, all forms |
| --- | --- | --- |
| Global-pointer table relay | 83 / 69 | 73 / 59 |
| Parameter-indexed table relay | 86 / 75 | 76 / 65 |

The six other nonvolatile probes and every classic-backend size/timing are
unchanged. Full post-fix measurements are in
[LET_CODEGEN_AFTER_FIXES.csv](LET_CODEGEN_AFTER_FIXES.csv); the audit command and
interpretation of XEX bytes/VM cycles are unchanged.

Ten new NIR tests cover multi-home chains, table indexing, fresh/LET parity,
idempotence, isolated and unrelated homes, fan-out, cross-block uses, escaped,
initialized and absolute storage, wider types, gap limits and ordering/fault
barriers. A new VM test covers 24,576 executions: three source forms, global and
parameter pointers, raw/optimized lowering, both runtimes, two origins, all 256
byte inputs and two destination indexes including a page-crossing store.

Final validation passes: 2,858 compiler tests, unchanged NIR snapshots, all 38
NIR and 167 MIR fixtures, and 139 VM harness tests (the full 138-test suite plus
the newly added connected-relay regression run separately). This includes the
1,296-execution audit and slice 1's 1,008-execution pointer-copy regression.

The existing standalone AES source in `atari/c-bench-64/benchmarks/action/aes256.act`
produces **byte-identical 4,211-byte XEX files before and after slice 2**.
The pre-promotion-extension image checks `SUM: 0` and 1,626 PAL ticks in Atari800
at origin $2000 (elapsed bytes `$5A $06`). This is a current before/after check
of the promotion extension, not a comparison with the historical inliner build
or with the pre-slice-1 compiler. The benchmark repository is not modified.

## Deferred: volatile-boundary elimination

The volatile snapshot retains its four-byte/four-cycle LET overhead. Atari
locals have static backing; immutability does not prove that storage is
unobservable. Keep this work separate until a reusable alias/effect proof can
justify weakening the barrier. No source semantics or volatile-access ordering
rules have changed.
