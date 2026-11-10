# MIR6502 pointer retention through calls and loops

Status: initial bounded subsets implemented and qualified on 2026-11-10.
See the [implementation plan](MIR6502_POINTER_RETENTION_IMPLEMENTATION_PLAN.md)
for concrete slices and qualification results.
Baseline: `66f42d1a`, after qualified register-copy forwarding. Commit each
implementation slice separately and record its measurements here.

Keep useful addresses in existing zero-page pairs through operations that
provably preserve them. Begin with redundant setup across direct calls, then
loop-invariant setup. Treat promotion of changing pointer variables as a
separate transformation with stronger observation and exit-state obligations.
TN supplies application evidence; neutral programs define general behavior.

## Evidence and existing foundations

The [external review](TN_6502_CODEGEN_REVIEW_2026-11-10.md#3-keep-pointers-live-across-calls-and-loop-iterations)
identifies `SetWin`, `Fnamecmp`, `Print`, `PrintFname`, `Instr` and `Convert`.
The frozen baseline cartridge images are TN 9,959 XEX bytes and TNDBG 13,055.
Baseline emitted routine sizes are:

| Routine | TN bytes | TNDBG bytes | Main opportunity |
| --- | ---: | ---: | --- |
| `SetWin` | 1,093 | 1,096 | Retain captured bases through conversion calls and loops |
| `Fnamecmp` | 185 | 185 | Advance two resident pointers instead of repeatedly copying homes |
| `Print` | 58 | 58 | Preserve an invariant base through the output call chain |
| `PrintFname` | 58 | 58 | Same call-chain obligation |
| `Instr` | 66 | 66 | Reuse an invariant base across loop edges |
| `Convert` | 131 | 131 | Multiple bases, conversion calls and destination writes |

These sizes come from the existing qualified listings under
`build/register-copy/final/`; the compiler code is unchanged by the intervening
documentation commits. Freeze fresh artifacts and hashes before implementation.
The `SetWin` eight-character conversion region is still 57 bytes, currently
`$4198..$41D1`. The review's 19-byte loop is a target to prove and measure,
not an admission rule or a promised whole-program saving.

Reuse these components:

- [Known callees](../src/mir6502/analysis/known_callees.rs) already summarize
  direct/transitive writes, X/Y preservation, bounded machine routines and
  exact A result locations. There is already a `preserves_fixed_pair` query.
  The current `Internal` body ends with `STA $A0; RTS`, which the existing
  machinery can describe as A equal to `$A0`; do not add an `Internal` special case.
- [Machine values](../src/mir6502/analysis/machine_values.rs) already carry
  register and fixed-zero-page byte facts across CFG edges and known calls.
  Unknown indirect stores deliberately kill resident-pair facts.
- [Record bases](../src/mir6502/analysis/record_bases.rs) prove complete
  setup equality with final physical aliases, but their proof scans one block
  and unconditionally clears state at calls and barriers.
- [Bounded pointers](../src/mir6502/analysis/bounded_pointers.rs) supply
  owner-qualified objects and extents. Their current audit has same-block,
  singleton-offset and restricted-call limits; it is not a general loop proof.
- [Loop motion](../src/mir6502/analysis/loop_motion.rs), CFG, dominance,
  liveness, reaching definitions and immutable post-home snapshots provide
  the existing control-flow and rewrite infrastructure.

The earlier [record-base plan](../surveys/tn/record-base-reuse-plan.md) and
[register-copy plan](MIR6502_REGISTER_COPY_FORWARDING_PLAN.md) deliberately
exclude this extension. Extend shared queries and analyses rather than create
an emitter cache, another machine-code decoder or a source-pattern optimizer.

## Ownership and distinct proofs

This is MIR6502 placement over verified typed NIR. SemIR retains pointer and
call semantics; NIR retains explicit captures, storage and conservative effects.
Emission supplies resolved layout and writes selected operations. No source
names, printed IR, TN addresses or inferred language rules enter selection.

Keep three questions distinct:

1. **Residence:** does the pair still contain the same captured address?
2. **Reevaluation:** would loading the original pointer cells now produce that
   same address? A callee can preserve the pair while changing those cells.
3. **Promotion:** may updates to source-visible pointer cells be delayed or
   removed while the evolving pointer lives in a pair?

A captured address can survive source-cell mutation, but it cannot justify
replacing a subsequent reevaluation of those cells. Facts must include complete
low/high provenance, source dependencies, owning routine/storage IDs and
generation-scoped definition sites. Partial lanes and unknown values prove
nothing. Extend the existing resident-base analysis/query surface; keep
machine-value and memory-effect interpretation consistent across consumers.

### Calls

A call may preserve a resident address only when its full transitive execution
preserves both pair bytes. Preserving equality with source homes additionally
requires that their bytes are unchanged. Resolve named and absolute aliases in
the final layout, including callee prologues, argument placement, return slots,
allocated scratch, helpers and tail calls. Callee-local IDs must retain their
owning routine; an unqualified `MirMem::Local` or `Param` from a flat summary is
insufficient for an interprocedural physical-disjointness proof.

Use existing exact summaries first. Narrower bounded footprints must be
call-site-specific when they depend on actual arguments; never cache them by
callee ID alone. Unknown/indirect/recursive calls, opaque machine code and
unsupported effects retain fallback. Inspectable machine routines use the
existing bounded summary rules. Arbitrary inline machine blocks remain barriers.
All actual calls and observable accesses retain their order and count.

An unknown read footprint need not destroy an unchanged pair, but it cannot
justify deleting stores that the callee could observe. Remove private captures
only through the existing definition/use proof with physical alias exclusions.
Retaining a pointer does not by itself make its saved spill stores dead.

Returning a byte in A is separate from pointer preservation. Keep an A result
only through operations proved not to clobber it. Remove an ABI-result reload
only when exact equality and the required N/Z effects are preserved or dead.
Do not publish new incidental return-flag contracts for mutable MIR routines.

### CFG edges, loops and writes

Use forward must-agree dataflow with separate unreachable and reachable-unknown
states. At a join, every reachable predecessor must establish the same complete
fact. A backedge cannot establish its own initial value: a retained loop base
needs a dominating definition on every entry and preservation on every cycle.
Bound analysis work and fall back on unsupported graphs or exhausted budgets.

Unknown indirect stores remain barriers. Preserving a base through an indirect
write requires a bounded complete footprint, disjoint from the pair, source
cells and other protected captures in the final layout. Include access width,
index ranges, carry, wrap and all possible object origins. Numeric RAM addresses
and pointer types do not establish object bounds or ordinary data ownership.

This is the material issue in `SetWin`: `Internal` preserves `$AC/$AD`, but the
following `s(i)` store must also be proved unable to overwrite the pair or the
source pointer. Its origin comes through directory buffers and `v`, not merely
from the declaration of `s`. Trace only supported typed definitions; retain
fallback if that origin cannot be proved. A neutral bounded-buffer case must
improve independently of success on the complete TN routine.

Start loop selection with single-entry natural loops and existing pairs. Reuse
an already established base before moving setup. Moving setup requires valid
execution placement for zero-trip loops, preserved capture timing and no new
live A/X/Y/flag clobbers at the guard. Do not speculate hardware/volatile reads.
Keep loop bounds, memory-access order and final index values unchanged.

### Changing-pointer promotion

`Fnamecmp` increments its parameter homes, so invariant-base reuse is insufficient.
Promotion needs complete use/escape and alias auditing, a pair assignment valid
through the region, modulo-65536 pointer updates, and correct source-home state
at every exit. Account for early returns, multiple exits, source-home address
exposure, indirect reads, call observation and routine-static reentry.

Non-escape alone does not prove an incoming pointer disjoint from the callee's
own parameter homes. An externally admitted pointer may point into those homes;
delaying their updates could change the compared bytes. Preserve such observable
updates or decline promotion. A closed-entry assumption requires an actual
verified contract, not a direct-call census. If necessary facts are absent from
NIR, specify that contract as a separate vertical slice before consuming it.

Initially admit only complete word pointers with unit updates and a bounded,
auditable use region. Reject unsupported interference or observation. A shared
Y offset, multiple competing new pairs, general allocation and unbounded scan
provenance are follow-on choices, not prerequisites for invariant retention.

## Implementation slices

0. **Freeze the scorecard.** Save TN/TNDBG and neutral listings, materialized
   MIR, maps, hashes, frame/ZP counts and execution measurements. Add diagnostics
   distinguishing pair clobber, source mutation, unknown call footprint,
   unbounded write, CFG disagreement, live machine state and cost. Include
   renamed/relocated one- and two-pointer cases; telemetry must not change bytes.
1. **Retain captured bases through direct calls.** Connect resident-base proofs
   to existing known-callee summaries and final physical aliases. Start inside
   one block. Preserve the original high-byte load when needed for A/N/Z, then
   use existing exact-result and register-copy cleanup where separately proved.
   Test nested calls, ABI/prologue effects, source mutation and pair clobbers.
2. **Establish CFG and bounded-write facts.** Extend the shared resident-base
   proof to joins/backedges and add the minimum object/range queries needed for
   loop stores. Keep this analysis independently testable before loop selection.
   Test initial-entry dominance, multiple origins, partial lanes, aliasing,
   page/word wrap, unsupported loops and budget exhaustion.
3. **Select invariant retention.** Reuse existing entry pairs, then consider
   safe one-time setup in natural loops. First qualify a neutral bounded
   conversion loop; measure the real `SetWin` region and report its precise
   remaining blockers. Treat `Putchar` separately: its indirect screen store
   currently prevents an exact write summary, even when `CalcAdr` and `Internal`
   are individually inspectable. Do not weaken that barrier to obtain a saving.
4. **Promote changing pointers.** Add a separate proof/selector for private,
   unobserved pointer homes and complete exit reconstruction. Qualify neutral
   one/two-pointer scans before `Fnamecmp`. Document any needed entry-contract
   extension before changing NIR; never silently assume it.
5. **Qualify and update the contract.** Record actual image/routine sizes,
   setup/spill traffic, cycles and compiler analysis cost. Update the
   [pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md) with the admitted
   guarantees and rejected forms. Keep each implementation slice reviewable.

## Selection and rollback

Use independent optimized-only controls for call/CFG retention and changing-home
promotion, under `enable_peepholes`. Feature-off preserves today's pipeline.
Run initial selection after existing placement/inlining; retain frames and
reservations. Use existing structural transactions for local changes and
explicit validated region plans for setup movement or promotion. Rebuild facts
after each applied dependent change; a shorter operation list alone does not
validate a previously discovered plan.

Recompute summaries and replay every consumed call/region proof against the
combined proposal and final emitted layout. A changed callee must not invalidate
an already optimized caller's preservation or A-result contract. Reuse the
baseline-reference physical-map hook when deleted references still need final
addresses. Reject stale, unverifiable, ambiguous or unprofitable proposals by
restoring the full feature-off result, preserving previous optimization gains.

Require a strict final image-size improvement, no affected routine/home growth
and no additional occupied zero page in the initial slices. Check complete
path costs, including entry setup and exit writeback. Unknown or zero trip
counts cannot inherit the review's eight-iteration speedup. Separate measured
cycles from nominal instruction estimates and candidate-pattern budgets.

## Validation and acceptance

Generic tests must cover preserved/clobbered pair halves, changed source cells,
captured-address use after source mutation, transitive calls, callee scratch and
return placement, hardware/volatile reads, unknown writes, physical aliases and
cross-lane overlap. Loop tests cover zero/one/many iterations, byte boundaries
0/127/128/254/255, page crossings, full word wrap, joins, multiple exits, final
indices/pointers, reentry and parameters pointing into their own homes.

Execute independent feature-off/on data/guard/access-trace oracles in cartridge
and standalone runtimes, raw/optimized NIR, relocated code/data and actual
LF/CRLF parsing. Use renamed sources and different pair assignments; no fixture
name or absolute address may admit a compiler rewrite. Report real TN/TNDBG
results even when a particular routine remains unchanged.

Run focused checks during each slice. Final MIR6502 qualification includes its
full library filter, tracked-emitter tests, all `mir6502_*` integration targets,
the emission/load fixture sweep, MADS plain/source and re-origining oracles,
and affected pointer/ABI/readback/TN VM targets. Broaden to the required shared
NIR checks only if a shared contract changes, as specified in
[AGENTS.md](../AGENTS.md).

Do not claim the full finding solved merely because one call-site setup folds.
Record call retention, invariant-loop retention and changing-pointer promotion
separately, with blockers and measured coverage for each.

## Implemented boundary

The [implementation results](MIR6502_POINTER_RETENTION_IMPLEMENTATION_PLAN.md#final-qualification)
record the completed slices, exact artifacts and validation. Direct-call
retention, must-agree CFG residence, owned indexed-store retention, safe entered
loop setup and one private changing-pointer scan are enabled independently.
Frames remain unchanged and every accepted family passes final-layout replay
and strict emitted-size checks. Exact ABI-result reuse composes through the
existing accumulator/flag proofs.

The admitted promotion subset deliberately excludes incoming parameter pointers
and competing changing pointers. TN's `Fnamecmp`, the `Putchar` chain and
unbounded destination-write loops retain their conservative code. Their next
proof obligations are documented with the results; no shared NIR or source
entry contract was added by this implementation.
