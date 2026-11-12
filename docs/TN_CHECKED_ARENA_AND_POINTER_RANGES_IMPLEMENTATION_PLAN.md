# Checked arena and pointer ranges implementation plan

Status: implementation in progress; slices 0–5 delivered. Starting revision: `c1dc6a5a`; compiler baseline:
`95e88790`. Implements the [design note](TN_CHECKED_ARENA_AND_POINTER_RANGES_DESIGN.md).
Work in the numbered slices below and commit each after its applicable checks.

There are two independently useful deliveries: a checked arena interface, and
compiler reuse of ranges derived from verified code. Runtime checks remain
enabled regardless of optimization settings. The compiler work must improve a
neutral case without assuming that TN's existing `Strcat` entry is bounded.

## Scope and fixed decisions

- Preserve the public signatures, successful operation order and checkpoint
  semantics of `Alloc`, `Free`, `Push`, `Pull` and `RestorePos`.
- Preserve `Alloc(0)`, including returning the exclusive end. Prove `[B,E]`
  first; use `[B,E-1]` only when a nonzero request is independently established.
- Failure records a reason and reaches a non-returning sink before the failing
  primitive changes its cursor or target bytes. Do not assume that `Error`
  terminates or return a null address to unchecked callers.
- Infer facts from typed checks and complete control flow. No allocator-name
  recognition, new source annotation or executable expression strings.
- Keep unknown/observable entries generic. Consume facts established within a
  routine or supplied by an independently verified entry contract. Facts at
  one caller cannot become unconditional facts at the original callee entry.
- Do not clone routines in this first delivery. Private variants need a
  separate storage-identity design: duplicating ordinary parameter/local homes
  can change observable effects, even when the pointer values are bounded.
- Preserve the current constant-address/owned-object proofs. Add a separate
  purpose-specific physical-range proof for pointer retention; do not weaken
  ownership queries used by unrelated optimizations.

Routine cloning, whole-program callback closure, allocator freshness/lifetime
inference, a general alias solver, `MovePage` zero-count changes and the
independent carry-input arithmetic optimization are outside this plan.

## Code homes

New module names below are proposed; reuse existing query infrastructure.

| Area | Implementation home |
| --- | --- |
| Checked API and failure path | [LIB.ACT](../samples/tn/modern/LIB.ACT), shared by TN and TNDBG |
| Runtime oracles and UI high-water observations | [VM test crate](../tools/vm-runtime-tests/tests), existing TN support modules |
| Numeric range domain and edge transfer | New `src/nir/analysis/ranges/`, using existing CFG, dataflow, dominance, storage and use/def analyses |
| Derived return summaries and narrow query facade | New range-analysis modules and [src/nir.rs](../src/nir.rs) |
| Projection and complete word carriers | [MIR6502 lowering](../src/mir6502/lower.rs), materialization and a new `analysis/address_ranges` module |
| Context-dependent copy effects | [bounded_memory.rs](../src/mir6502/analysis/known_callees/bounded_memory.rs) and [resident-base analysis](../src/mir6502/analysis/record_bases/retention.rs) |
| Validated regional rewrite and costing | [pointer_retention.rs](../src/mir6502/materialize/pointer_retention.rs), existing rewrite context/plan/driver |
| Qualification | Focused integration/VM targets, `docs/benchmarks/6502-checked-arena/`, a maintained measurement/check command |

SemIR retains source meaning. NIR owns generic numeric computation and derived
facts. MIR6502 owns physical address/carrier proofs and target placement.
Emission supplies final maps; it must not recover allocator meaning from source.

## Slice 0 Freeze controls and expose the remaining blockers

1. Freeze TN/TNDBG in cartridge/standalone modes and the classic optimized and
   MIR6502 backends that consume modern LIB.ACT. Record source/compiler hashes,
   image/body sizes, routine maps, frame/storage sizes and zero-page occupancy.
   Reuse qualified artifacts only after matching their source/configuration.
2. Audit every cursor store by resolved storage identity, including `$E8/$E9`
   aliases, initialization and stores outside allocator-named routines.
3. Add a VM harness that calls the real library routines and observes cursor,
   data, position bytes, guard bytes and stack. Freeze successful allocation,
   rewind/scratch and window save/restore behavior.
4. Add neutral checked and unchecked allocator sources, using different names
   and a non-page-aligned arena. Add a concatenation body that obtains its
   pointer from a checked return, plus one with explicit entry guards. These
   provide bounded cases without requiring private routine variants.
5. Record all three TN `Strcat` callers separately: reaching pointer stores,
   initialization, intervening effects, entry eligibility and missing facts.
   Freeze the generic parameter-home overwrite examples, including the case
   with lengths three and two, and the current 113-byte body.

Acceptance: the controls reproduce existing behavior; the harness resolves real
entries/storage and handles LF/CRLF through its actual parsing path. Do not run
unchecked oversized allocations merely to produce a corruption snapshot.

Suggested commit: `Freeze checked arena runtime and pointer range controls`.

## Slice 1 Enforce the complete arena interface

1. Add constants for the exclusive end and stable fault reasons: invalid
   cursor, exhausted capacity, invalid rewind mark and underflow. Implement a
   dedicated reason-recording non-returning path without cartridge dependence.
2. Capture the cursor and request/mark once. Check `B <= c <= E` before
   subtraction. Check `n <= E-c` before allocation, two bytes before push/pop,
   and four bytes before restoring a saved position.
3. Preserve the existing successful ordering: push writes then advances; pull
   and restore decrement before reading. `Free` accepts `B <= mark <= c` and
   leaves released bytes intact. Initialization writes B directly.
4. Keep zero-size requests valid at B, interior cursors and E. Validate that
   the configured positive capacity and exclusive end fit the CARD-based API.
5. Audit `MovePage` metadata against real uses, including restore to `$5A`.
   Correct incomplete memory descriptions conservatively. Do not strengthen
   unconditional pair-preservation or non-zero-page claims from intended use;
   narrower effects remain derived from the body and actual arguments.
6. Exercise normal TN/TNDBG UI flows and record maximum cursor use. An arena
   exhaustion on an intended flow requires investigation, not a silent change
   to the reserved size. Leave the unrelated C port work untouched.

Acceptance: every failing primitive reaches the sink without its cursor/data
update; every successful primitive matches the frozen memory behavior. Test
post-`Free` use in `MkDir` and `CloseWindow`. Cover both backends/runtimes and
actual LF/CRLF inputs. Source ABI parameter capture, ordinary call-stack writes
and the diagnostic byte are distinct from the forbidden failing target writes.

Suggested commit: `Bound TN arena allocation and checkpoint operations`.

## Slice 2 Derive unsigned ranges from verified NIR

1. Add an inert range domain for the initial unsigned BYTE/CARD widths. Keep
   unreachable separate from reachable unknown. Use widened arithmetic for
   endpoints and check representability before claiming no wrap.
2. Track constants, copies, block arguments, supported casts, comparisons and
   checked addition/subtraction. Refine true/false CFG edges through dominating
   comparisons, using stable temp/block IDs and shared use/def/dominance facts.
3. Add a narrow guard certificate for relations such as `n <= E-c`, with the
   already-proved cursor bounds, so the corresponding `c+n` can be certified
   non-wrapping. Match the same captured operands; reloading a mutable cell
   does not inherit the certificate without a reaching-value proof.
4. Prove failure sinks from reachable control flow. Initially handle explicit
   sink loops and verified direct non-returning bodies. Returning handlers,
   opaque/retargeted targets and unresolved control flow supply no such fact.
5. Bound each routine analysis to 512 blocks, 4,096 operations and 4,096 transfer
   evaluations initially. On exhaustion return unknown for the affected query;
   never expose partially converged facts. Loops start from real entry facts.

Acceptance: renamed/relocated neutral routines give equivalent facts; unchecked
lookalikes do not. Cover exact boundaries, casts, signed/unsupported cases,
wrap, joins, backedges, changed definitions and returning failure handlers.
There is no emission consumer yet and existing executable NIR stays unchanged.

Suggested commit: `Derive guarded unsigned ranges over verified NIR`.

## Slice 3 Summarize normal returns and track stored values

1. Derive a return-range summary from every reachable normal return. For the
   checked allocator, require at least `[B,E]`; derive a tighter nonzero bound
   only with its additional proof. An unbounded returning path makes the
   summary unknown. Do not infer an allocation extent or freshness.
2. Instantiate direct call-result facts at the actual call point. Depend on
   the verified body, target identity and analysis generation. Initially reject
   recursive SCCs, unresolved/retargeted calls and depth beyond sixteen.
3. Track complete stores and later loads of ordinary pointer cells through
   reaching definitions and effects. Kill facts at partial writes, possible
   aliases, unknown calls, machine/OS effects and possible reentry. Do not turn
   `linebuf`'s conditional initialization into a global invariant.
4. Distinguish facts after normal return from effects on a path that cannot
   resume the caller. Exclude a failure-only path from a continuation proof
   only with its non-return certificate; retain the failure operation itself.
   Include all effects on every path that can return.
5. Expose a narrow typed query facade. Use derived tables and body/generation
   witnesses, not new metadata operations in executable blocks. If a projection
   contract requires verifier/printer work, add and document it in this slice.

Acceptance: bounded results reach local/global stores and survive only proved
non-interfering effects. Preserve the numerical range after a cursor rewind,
while invalidating any unproved content/lifetime claim. Test initialized/zero
joins, absolute cursor aliases, callbacks, changed bodies and stale summaries.
Caller result facts must not strengthen the original callee's incoming domain.

Suggested commit: `Carry verified return ranges through calls and storage`.

## Slice 4 Project coherent ranges into MIR6502

1. Add an independently controlled, initially default-off range-retention
   pipeline. Preserve the existing bounded constant-address path and its flag.
2. Project verified NIR facts through a narrow interface using stable routine,
   block, temp and storage IDs. MIR must consume the facts without consulting
   SemIR. Raw and optimized NIR paths must both work.
3. Track a complete word range through ABI A/X, spills, globals and fixed
   pointer pairs. Tie low/high lanes to one value and generation. Reject mixed
   origins and unproved partial rewrites instead of combining independent
   byte intervals into an address.
4. Transfer bounds through the existing selected pointer-plus-byte-plus-one
   operation using the typed unsigned-byte range. Preserve the original read
   order; prove the resulting 16-bit effective addresses do not wrap.
5. Refine memory facts after calls only from complete returning effects and
   actual ABI placement. Include parameter prologues, result cells and scratch.
   A target-specific physical proof may preserve a stored range that generic
   NIR effects conservatively discard; the body/layout dependencies must remain
   explicit and replayable.

Acceptance: positive fixtures carry a range from checked return or entry guard
to the real materialized copy inputs. Unknown original entries stay unknown.
Stale facts, changed casts, partial lanes, aliases and unproved effects reject.
With the consumer disabled, emitted images match the checked-library baseline.

Suggested commit: `Project checked address ranges into MIR6502 carriers`.

## Slice 5 Prove physical copy ranges and unchanged source homes

1. Extend the existing typed machine audit to consume coherent word intervals.
   Return context-dependent read/write ranges rather than guessing one address
   from independent bytes. Retain current ISA, incoming-flag, operand and work
   limits unless a demonstrated case receives a separately tested extension.
2. Model all possible Y offsets and the actual zero-count/256-byte behavior.
   Include direct ABI/scratch writes and all indirect destinations. Unsupported
   source reads, wrapping or incomplete effects retain the call barrier.
3. Add a retention-specific disjointness query for complete physical ranges in
   supported RAM. Keep the existing owned-object containment predicate intact.
   The possible tail outside the arena remains in the write footprint.
4. Compare against both bytes of each resident pair and every source home it
   represents, with real routine owners and final placement. Invalidate every
   overlapping cached value, including unrelated storage in the footprint.
5. Replay footprints against the proposed callee bodies and final physical map.
   A changed body, range, owner, signature or placement invalidates the witness.

Acceptance: neutral parameter-home-disjoint copies preserve the captured
address value; copies touching either home lane do not. Exercise relocated
layouts where the same interval overlaps protected data/code and must reject.
Keep generic raw-pointer, hardware, stack and short-string alias controls.

Suggested commit: `Prove frame disjointness from bounded copy footprints`.

## Slice 6 Select bounded post-call pair reuse

1. Add a small same-block region query for two complete post-call pointer
   setups followed by an `IndirectByteCompound` consumer. Require generation-
   scoped available values and the call-footprint/source-home witnesses.
2. Map the consumer's target/source to the retained pairs, preserving its
   effective addresses and ordered post-copy reads. Do not reuse the captured
   pre-copy length. Do not delete the setups while leaving reversed pair roles.
3. Record every changed register, flag and scratch home. Remove reconstruction
   only when the exit changes are proved unobservable; otherwise retain the
   existing code. Preserve published callee facts and raw/absolute observations.
4. Validate an exact typed original/replacement region through the rewrite
   driver. Do not weaken its generic removed-definition checks or introduce a
   rule-name escape hatch. Reconstruct the regional proof during final replay.
5. Use the existing costed transaction with unchanged allocations and strict
   whole-image savings. Count an application only after combined proof and
   cost acceptance. Unsupported pairing or a required expensive restoration
   keeps the baseline; producer rescheduling is not added opportunistically.

Acceptance: a normally lowered neutral case independently saves code and
preserves memory, ordered reads, required exit state and stack. Include live
pair/register/flag negatives, explicit scratch observations, parameter-home
overwrites, repeated application, reporting and failed-cost rollback. Facts
known only at a caller must not optimize the unrestricted `Strcat` body.

Suggested commit: `Reuse retained pairs after range-proven copies`.

## Slice 7 Qualify defaults and measure TN applicability

1. Qualify each enabled family before changing optimized defaults. Conservative
   defaults remain off. Disabling parent pointer-call retention also disables
   this consumer, including when CFG retention remains enabled.
2. Execute the actual checked library and new neutral programs across relevant
   backends, runtimes, raw/optimized NIR, origins and real LF/CRLF files. Test
   failure sinks with a dedicated bounded VM runner; the normal return helper
   is not an oracle for a deliberately non-returning call.
3. Exercise TN/TNDBG strings, generic parameter aliases, window save/restore,
   scratch reuse, dispatch and panels. Measure high-water cursor use and arena
   guards. Reassemble LF/CRLF listings with MADS and verify relocations.
4. Record every TN call's fact/admission result. Distinguish unknown entry,
   uninitialized/mutated global, source-read uncertainty, physical overlap,
   live exit state and unprofitable cost. Report unchanged `Strcat` if those
   obligations remain; do not add guards or private clones to force a saving.
5. Publish three configurations with source/image hashes: unchecked library;
   checked library with range optimization off; checked library with it on.
   Separate runtime-check cost from compiler savings. Include XEX/body bytes,
   frames, reservations, zero page, cycles, analysis evaluations and compiler
   timings; do not quote the twenty-byte region as a net image improvement.
6. Keep historical scorecards frozen. Add new default-image bindings and a new
   generator/check command for this configuration rather than rewriting old
   measurements against changed LIB.ACT.

Acceptance: all applicable checks below pass, a neutral optimization has a
measured benefit, failure behavior matches the design, and TN outcomes and net
cost are recorded honestly. Checked runtime operations can remain delivered
even if an optimizer family stays disabled after qualification.

Suggested commit: `Qualify checked arena ranges and record target results`.

## Required check gates

Add the following focused targets as their slices land. Names are proposed;
record the final commands in the qualification scorecard.

| Target | Scope |
| --- | --- |
| `cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_checked_arena` | Slices 0–1: real library success/fault behavior, both source backends/runtimes, scratch/window uses, LF/CRLF |
| Filtered library tests under `nir::analysis::ranges` | Slices 2–3: guard, return and storage facts, unknown cases and analysis limits |
| `cargo test --test mir6502_checked_pointer_ranges` | Slices 4–6: projection, proof rejection, exact rewrite/costing, layouts and unchanged controls |
| `cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test checked_pointer_ranges` | Slices 4–6: neutral feature-off/on behavior, aliases, byte counts 0–255, exit state and relocations |

The runtime boundary matrix includes zero/exact-fit/one-too-large requests,
invalid cursors below B and above E, `$FFFF` requests, rewind marks at/below/
above their boundaries, and push/pull/restore with exactly enough and one byte
too little space. Observe fault reasons as well as unchanged cursor/target
bytes. Batch the cases in each backend/runtime configuration.

Reuse the existing `mir6502_bounded_call_pointer_retention` integration target
and `bounded_call_pointer_retention`, `pointer_retention`, `strcat_idiomatic`
and `tn_dispatch` VM targets for affected controls. Keep historical-image
bindings separate from fresh checked-library measurements. Include MADS and
fresh-image checks that are ignored in the default test invocation.

Use focused tests while developing each slice. Before committing a slice that
changes compiler NIR or shared lowering/verifier/printer contracts, run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Keep fixtures unchanged for inert facts; explain intentional new source CFG,
IR-contract, printer or bug-fix differences when expectations change. Update
[NIR_TARGET_SHAPE.md](NIR_TARGET_SHAPE.md) for any new projection guarantee,
including rejected/stale facts. Preserve readable printed IDs and names.

Source-library-only changes run the affected TN integration and VM targets in
classic optimized/MIR6502 and cartridge/standalone modes. They do not require
the shared-contract gate unless compiler contracts also change. MIR6502-only
slices run affected unit/integration/VM checks; final qualification includes
its complete backend suite and `actionc-mir6502-sweep --emit-load` coverage.
Include other backends that actually consume changed shared contracts rather
than treating generic NIR work as backend-only work.

Normalize host text before newline-sensitive instrumentation and check both
LF and CRLF through the real path. Preserve binary and ATASCII fixture bytes.
Run the new measurement generator's check command before the final commit.
Repeat passing suites only for further changes, failures or unresolved concerns;
the mandatory shared-contract checks take precedence over local scoping.

## Completion boundary

This plan is complete when the checked API is qualified, derived ranges reach
a validated target consumer, the positive neutral case improves, negative
cases retain their semantics, and all three measured configurations are
documented. It does not promise unrestricted-callee specialization or a TN
size reduction. If unknown entries alone block TN, prepare the separate
private-entry/storage-identity design required for variants before attempting
that extension.
