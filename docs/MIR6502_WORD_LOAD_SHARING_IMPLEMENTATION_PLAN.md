# MIR6502 word load sharing implementation plan

Prepared 2026-11-12. Status: slice 1 complete; slices 2–4 pending. Compiler baseline: `7f7778f6`.
Implement in four vertical slices and commit each after its checks pass.

Share an ordinary word's byte loads when two direct destinations receive the
same word in one contiguous block region. Place each destination's existing
low-byte store beside the first low-byte load, then do the same for the high
byte. This removes two redundant loads while preserving both complete copies.

The first acceptance target is the three measured cases in the maintained
[Toolkit ALLOCATE library](../samples/toolkit/modern/ALLOCATE.ACT). The
hand-scheduled reference saves 18 bytes and eight cycles per affected call.
This plan contains the bounded scheduling contract; a separate design note
or new executable IR form is unnecessary for this first implementation.

## Baseline and acceptance target

The analysis used an identical retaining client for both backends. Compiling
the library alone now removes unreferenced routines, so preserve that client
when comparing complete images. Its entry performs these calls:

```action
INCLUDE "../../samples/toolkit/modern/ALLOCATE.ACT"
CARD request=$0600,result=$0602
PROC Main()
  AllocInit(0)
  result=Alloc(request)
  Free(result,request)
  PrintFreeList()
  DO OD
RETURN
```

The include path assumes a client under `build/<analysis-directory>/`.
This client retains routines for inspection; execution probes enter routines
directly after installing a valid sentinel and controlled heap in ordinary RAM.

| Cartridge measurement at origin `$3000` | Baseline | Word sharing reference |
| --- | ---: | ---: |
| `Alloc` instruction bytes | 287 | 281 |
| `Free` instruction bytes | 358 | 352 |
| `AllocInit` instruction bytes | 96 | 90 |
| `PrintFreeList` instruction bytes | 112 | 112 |
| Retaining client XEX bytes | 952 | 934 |
| Empty `Alloc` cycles | 102 | 94 |
| Exact-match `Alloc` cycles | 248 | 240 |
| Split-block `Alloc` cycles | 325 | 317 |
| Inserting `Free` cycles | 414 | 406 |
| `AllocInit` cycles | 170 | 162 |

Cycles include the routine's return and exclude the common 20-cycle call
wrapper. The measured sites are `FreeList` copied to `last` and `$AC/$AD`
in `Alloc` and `Free`, and `EndProg` copied to compiler homes and `p` in
`AllocInit`. These names identify measurements, never selector conditions.
Local evidence is under ignored `build/allocate-analysis-20261112/`, including
`metrics.json`, `cycles.tsv`, `word-sharing.asm`, and `reference-cycles.tsv`.
Slice 1 must freeze reproducible inputs and measurements independently of
those temporary files.

The separate six-byte comparison-layout improvement and the library's missed
two-sided coalescing are outside this plan. The acceptance target is 934,
not the combined hand-scheduled reference's 928 bytes.

## Ownership and initial scope

SemIR and verified NIR keep their existing meaning, storage facts and forms.
MIR6502 owns this byte schedule and its physical alias proof. Emission supplies
placement and final costs and writes the selected existing instructions.

Add a dedicated word-load-sharing proof and selector alongside
[pointer load sharing](MIR6502_POINTER_LOAD_SHARING_PLAN.md). Reuse its
transaction and costing conventions without broadening that earlier rewrite
or changing [home value invalidation](MIR6502_HOME_VALUE_INVALIDATION_IMPLEMENTATION_PLAN.md).
The missing optimization requires scheduling across byte lanes, rather than
retaining one more equality for the current value in A.

Match exactly eight adjacent, byte-wide operations in one block. Each load
reads direct storage into A; every store writes A into direct storage.
Let `S` be the source pair and `D` and `E` be the two destination pairs:

| Original operations | Replacement operations |
| --- | --- |
| `LDA S.lo` | `LDA S.lo` |
| `STA D.lo` | `STA D.lo` |
| `LDA S.hi` | `STA E.lo` |
| `STA D.hi` | `LDA S.hi` |
| `LDA S.lo` | `STA D.hi` |
| `STA E.lo` | `STA E.hi` |
| `LDA S.hi` | |
| `STA E.hi` | |

Construct the replacement by cloning original operations `[0,1,5,2,3,7]`.
No store, home or scratch reservation is removed. Preserve low-before-high
order within each destination. Only the interleaving of the two copies changes.

Initial source support covers complete ordinary scalar parameter/local words
and ordinary RAM globals, using stable IDs and offsets. Parameter/local
eligibility retains the existing exclusions for explicit backing or aliases.
Global support must use MIR global facts and final ownership; an address in
RAM or a different symbol ID alone does not establish eligibility.

Destinations may be complete ordinary direct storage, independently classified
compiler spill/zero-page home bytes, or existing backend pointer scratch pairs
`$AA/$AB`, `$AC/$AD`, and `$AE/$AF`. Require adjacent physical low/high bytes.
Do not treat arbitrary fixed zero-page addresses as private scratch. Outgoing
ABI cells, raw absolute addresses, volatile/hardware storage, indirect/indexed
operands, additional operations and cross-block windows remain unsupported.

## Proof and application contract

Expose a typed query through the existing post-home rewrite context. Its
witness records routine, generation, exact region, typed source/destination
bytes and the three resolved physical footprints. Prove all of the following:

1. Both repeated source reads identify the same respective typed byte.
   The source word is complete, non-wrapping, ordinary and owned as data.
2. All destination bytes have known ordinary data or approved private-scratch
   ownership. Verify both bytes of each footprint, including separately
   allocated compiler home bytes. Reject execution-memory overlap.
3. `S`, `D`, and `E` are pairwise physically disjoint. Resolve named aliases
   through actual placement; symbolic inequality is insufficient. Unknown
   placement, partial overlap and same-destination copies reject initially.
4. Moving `E.lo` before `D.hi` and before the high-byte read cannot change a
   source or another destination. No call, barrier, machine block, observer
   or ordered memory access occurs inside the exact region.
5. At exit, memory is equal, A holds the same source-high byte, and N/Z match.
   X, Y, SP and every other status bit are unchanged. This is complete exit
   equality and requires neither a dead-register assumption nor a following
   instruction to repair machine state.
6. All four stores retain their values and each destination's lane order.
   The two removed reads are redundant ordinary reads. Later indirect reads
   may alias any completed destination; do not assume otherwise.

Use a new `MirPostHomeWitness::WordLoadSharing` variant. Validate its exact
original and replacement against the current generation and supplied physical
map at application. The shared driver must reject an attempted rewrite of
this family with a missing, altered or stale witness. Neither a rule name,
permission Boolean nor matching aggregate effects can authorize store motion.
Keep existing dependency checks and their unknown-memory fallback intact.

## Integration and cost limits

| Code home | Planned responsibility |
| --- | --- |
| New `analysis/word_load_sharing.rs` | Exact shape, complete ordinary words, physical overlap and exit-state proof |
| [record_bases.rs](../src/mir6502/analysis/record_bases.rs), [call_predicates.rs](../src/mir6502/analysis/call_predicates.rs) | Reuse placement/ownership and existing source classifications; add narrowly scoped queries if needed |
| [context.rs](../src/mir6502/rewrite/context.rs), [plan.rs](../src/mir6502/rewrite/plan.rs), [driver.rs](../src/mir6502/rewrite/driver.rs) | Typed witness, structural transaction and application-time revalidation |
| New `materialize/word_load_sharing.rs` | Candidate discovery, bounded selection, cost checks and placement replay |
| [mod.rs](../src/mir6502/mod.rs), [passes.rs](../src/mir6502/passes.rs) | Final pipeline placement and independent `enable_word_load_sharing` flag |
| [pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md) | Accepted schedule, proof obligations and rejected forms |

Run after the complete existing final selector chain, including argument X
forwarding. Convert the current tail return into a baseline binding as needed;
when disabled, preceding decisions and emitted bytes must remain identical.
Gate the selector on both `enable_peepholes` and the new flag. Initially keep
the flag false in conservative/default and optimized configurations; enable
the optimized configuration only after qualification.

Use a cheap shape scan before emission or analysis. Apply one transaction per
snapshot through the existing driver, rebuild facts after each change, and
bound convergence by the original operation count plus one. Each application
removes two operations. Keep candidate identities stable as neighboring
operation indices shift.

Follow the existing costed batch model: baseline, proposed batch, and at most
one batch with rejected routines restored. Require unchanged frames, home
capacities, reservations and occupied zero page, no affected-routine growth,
and a strict complete-XEX saving. Preserve published callee facts consumed
by earlier selectors, including relevant register/flag entry requirements,
register preservation and observable accumulator results.

Re-emit the combined proposal's physical map and replay the accepted
transactions from the unchanged baseline. Accept only an identical proposal
that passes PreEmission verification and final cost checks. Lost ownership,
changed aliases, replay mismatch, weakened callee facts or unknown costs
restore the baseline. As in the existing selector, bound this to at most
seven additional emissions per selected batch. Perform no whole-program
materialization per candidate.

Report `word-load-sharing-candidate`, `selected`, `applied`, and specific
storage, footprint, overlap, witness, callee-contract and final-cost blockers.
Count applications after final acceptance, and keep counts and output stable
with reporting enabled or disabled.

## Slice 1 Freeze the baseline and prove the exact schedule

1. Freeze the retaining client, relevant compiler/source/dependency hashes,
   images, listings, materialized MIR, routine sizes, frames and zero page.
   Record the current working-tree inputs as well as the revision. Preserve
   all existing optimizations in every comparison.
2. Add a neutral `fixtures/runtime/word_load_sharing.act` and integration
   target `mir6502_word_load_sharing`. Keep both copies observable and confirm
   the actual eight-operation miss after the complete optimized pipeline.
   Include ordinary-global sources and both measured destination arrangements.
3. Add the dormant proof module and witness validation. Unit tests cover the
   exact replacement, ordinary global/local/parameter inputs, complete words,
   separately named aliases, stale generations and changed physical maps.
4. Document the schedule in the pseudo-machine contract. Keep compiler output
   byte-identical while the selector is absent or disabled.

Acceptance: the three real-library misses and a neutral miss are reproducible;
legal schedules prove complete exit equality, and unsafe shapes reject.

Suggested commit: `Prove direct word load sharing across two destinations`.

## Slice 2 Select and cost word load sharing behind a flag

1. Add the disabled configuration flag and final selector. Apply the six
   existing operations through the generation-qualified structural driver.
2. Add bounded rollback, final placement replay, verifier, callee-contract
   and routine/image cost checks. Keep all prior flags identical in off/on
   comparisons and retain the complete prior result on qualification failure.
3. Test missing/forged witnesses, stale plans, adjacent candidates, idempotence,
   mixed accepted/rejected routines, changed placement and reporting invariance.
   Confirm unchanged frame/home capacity and no new zero-page demand.
4. With the flag explicitly enabled, measure the real three sites and the
   neutral fixture. Record actual load counts, routine bytes and XEX bytes.

Acceptance: the neutral case loses exactly two source loads while retaining
all stores and machine state. The real retaining client targets 934 bytes;
any different result needs an instruction-level explanation before proceeding.

Suggested commit: `Select costed word load sharing behind an independent flag`.

## Slice 3 Qualify aliases and execution boundaries

Add VM target `word_load_sharing`, using emitted fragments with an independent
host oracle as well as the neutral Action fixture. Batch distinct boundaries:

| Area | Required coverage |
| --- | --- |
| Values | `$0000`, `$0001`, `$00FF`, `$0100`, `$7FFF`, `$8000`, `$FFFF`; repeated calls with changed values |
| Ownership | Every supported source class and destination arrangement; incomplete words, object ends, missing placement, execution overlap, absolute/hardware/volatile storage |
| Aliases | Source/destination and destination/destination overlap, both overlap directions, shared high/low bytes, separate IDs resolving to the same bytes, and wrapping footprints |
| Effects | Interposed call/helper/barrier/machine block or indirect/indexed access must prevent selection |
| Machine state | Arbitrary incoming A/X/Y and carry/overflow values, exit N/Z/C/V and unaffected status bits, SP, live successor inputs and full observable memory |
| Read and write traces | Two source reads instead of four; equal destination store counts/values and per-destination lane order; later indirect consumers may observe the completed copies |
| Pipeline | Raw and optimized NIR, feature off/on, both Atari runtimes, origins `$3000` and `$41C7`, actual LF and CRLF source files |
| Placement and reporting | Independent final-map replay, mixed batches, repeated application, bounded emission counts and identical images with reporting off/on |

Unsafe cases must retain the baseline path and execute against their own
independent expected memory/state results. For physical-alias negative cases,
derive expected values from their constrained entry memory; do not seed
mutually inconsistent aliases or overwrite executing code to construct a test.
Normalize host text before newline-sensitive instrumentation and test actual
LF/CRLF parsing paths. Preserve exact XEX and ATASCII bytes.

Run the existing `allocate_executes_through_the_vm_library` oracle and focused
pointer-load-sharing, home-value, word-move-store, argument-forwarding and
checked-pointer tests. Execute controlled empty/exact/split/boundary allocation,
insertion/merge and initialization cases. Compare feature off/on on identical
inputs, including the existing two-sided-coalescing behavior without changing
the library algorithm.

Acceptance: allowed schedules meet the complete contract, fallback behavior is
preserved at every rejected boundary, and both runtime/link modes remain valid.

Suggested commit: `Qualify word load sharing aliases and machine state`.

## Slice 4 Measure applicability and enable the optimized default

1. Add a reproducible exporter/check command for this feature, following the
   existing MIR6502 measurement conventions. Freeze compact results under
   `docs/benchmarks/6502-word-load-sharing/` and keep bulk MIR/listings/images
   under ignored `build/word-load-sharing/`. Preserve historical scorecards.
2. Toggle only the new flag. Cover the neutral fixture, full retaining client,
   TN/TNDBG, existing pointer/argument/copy fixtures, a loop, a graphics sample
   and runtime-linked code. Record positive selections and explicit controls
   with no candidate. Use normal entry-reachable linking.
3. Compare instruction/routine/XEX sizes, source-read counts, VM cycles, frames,
   home capacity, zero page, selection/rejection counts and actual compiler
   work. Confirm the three ALLOCATE sites and explain every corpus change.
4. Enable the optimized default after the proof/cost/VM gates pass. Keep the
   conservative/default configuration unchanged. Check final feature-on and
   feature-off defaults, and test that disabling peepholes disables this pass.
5. Run final MIR6502 qualification: backend unit tests, all root
   `mir6502_*` integration targets, emission/load fixtures and MADS listing
   targets; then the Atari VM runtime suite, including ABI/runtime checks.
   Include LF/CRLF source and assembly roundtrips and a nondefault origin.
   Reassemble the measured outputs to identical objects and execute those
   exact newly measured images.

TN controls at this baseline are 10,290/10,415 bytes for cartridge/standalone
and 13,385/13,510 for TNDBG. Require no unexplained growth and preserve
allocator bounds proofs, dispatch behavior and pointer reconstruction after
calls. Additional valid matches may reduce these sizes; do not require a
particular TN saving or change its source to manufacture a match.

Acceptance: the optimized retaining client reaches the measured 18-byte
opportunity, admitted corpus cases improve without weaker proofs or storage
growth, unsafe cases preserve fallback, and all affected backend checks pass.

Suggested commit: `Enable qualified word load sharing and record corpus savings`.

## Validation and disk usage

During development use filtered `cargo test --lib word_load_sharing`, the new
integration/VM targets, and affected existing targets. At final qualification
use `cargo test --lib mir6502::`, explicit backend integration targets, and
`cargo test --release --locked --manifest-path tools/vm-runtime-tests/Cargo.toml`.
Run the new exporter's check command when its result artifacts change.

Reuse `build/checked-arena/shared-target` for root Cargo commands with
`CARGO_INCREMENTAL=0` and the existing dev/test profiles with debug information
disabled and optimization level 1. Keep VM commands in their existing release
target directory. Use
per-command target-directory settings so exporting a root target does not
create duplicate VM artifacts. Freeze compact evidence once; avoid copying
Cargo targets or creating build trees for each slice.

This work changes MIR6502 scheduling, not NIR forms, lowering, verification or
printing. If implementation expands into a shared compiler contract, run the
additional AGENTS.md checks: `cargo test nir_fixtures_match_snapshots`,
`cargo run --bin actionc-nir-sweep -- fixtures/nir`, and `cargo test`, and explain
any fixture change. Broaden beyond 6502 only when actual consumers require it.
