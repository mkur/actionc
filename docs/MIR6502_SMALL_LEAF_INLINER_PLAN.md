# Costed Small Leaf-Routine Inliner

Status: implemented, 2026-09-05. See the rollout details below for the
conservative subset and validation results.

Baseline: `e091990` (`nir: promote bounded scalar relays for indexed consumers`).
The standalone Atari AES benchmark at this baseline is 4039 XEX bytes and
1729 PAL ticks, with `SUM: 0`.

## Implemented rollout

`analysis/leaf_routines.rs` owns the pre-interface-resolution census;
`materialize/inlining.rs` owns bounded caller/callee transactions and cloning;
`materialize/inlining_cost.rs` costs actual emission after the existing backend
has assigned homes and relaxed branches. The shared small-loop temp remapper
also handles block parameters; fresh clone IDs are disjoint from both inputs.
No NIR or SemIR boundary changed, and no high-bit selector extension was needed.

Arguments enter the clone through fresh byte block parameters. This is required
even for an already evaluated logical temp: a direct cross-block substitution
can expose an argument producer to block-local cleanup before routine-wide
liveness is available. Entry-edge arguments keep that producer explicit until
normal block-argument lowering/coalescing. Regression execution covers computed
arguments, branching returns, repeated calls, and all 256 byte inputs, including
the sequence of public `$A0` stores.

The initial cost proof uses a deliberately stronger, narrower condition than
full condition-correlated path matching. Stable block provenance pairs caller
regions and exits, with identical intervening opaque-call sequences. Within
each changed region, **every** candidate path must beat **every** original path
by at least four estimated cycles per expanded call. This implies the required
saving for corresponding paths without inferring branch correlations. Changed
unrelated regions must also be non-regressing; identical unaffected regions
may cancel identical estimates. Lost region correspondence, internal cycles,
unsupported instructions, or changed materialized routines outside the caller
reject the trial. Calls to the leaf include its emitted prologue/body/return;
tail calls and final branch encodings are included. Indexed reads use their
page-crossing maximum on both sides; branch page crossings use final addresses.
These are conservative model estimates, not input-specific timing guarantees.

Selection ranks the frozen original leaves by a cheap call/capture saving hint
and stable IDs, then uses at most sixteen trials and eight sites per group.
Growth limits are 32 bytes/site, 128/caller and 256/program, cumulatively charged
without deleting the original callee. Speculative materialization and emission
reports are suppressed even when reporting is enabled through the environment.
`Default` stays off and `optimized()` enables the cost-gated pass; the CLI's
materialized dump now follows the same profile setting as normal generation.

Atari800 PAL AES validation (same source, origin `$2000`, standalone runtime):

| Build | XEX bytes | PAL ticks | Result |
| --- | ---: | ---: | --- |
| Inliner off | 4039 | 1729 | `SUM: 0` |
| Costed inliner on | 4074 | 1627 | `SUM: 0` |

All four `XTime` calls in `MixColumns` are expanded: 102 fewer ticks (5.9%) for
35 additional image bytes. The cost report charges 36 caller code bytes and
estimates at least 88 saved cycles across the four sites per loop-body execution;
it does not multiply by a guessed trip count. CRC8, CRC16 and sieve produce
byte-identical XEX images with the inliner on and off.

The TN standalone image is also byte-identical. The full test suite and the
167-fixture MIR6502 sweep pass. Focused coverage includes emitted execution for
all 256 byte inputs, argument-call evaluation exactly once, public return-store
traces, pointer/table composition, tail-call costs, page-crossing branches,
unknown/cyclic costs, omitted arguments, observable/escaping storage, budgets,
profile-consistent CLI dumps and environment-requested report suppression.
The pointer/table composition test deliberately checks expansion separately:
its cost-gated version retains the call when the initial region proof cannot
establish profitability. This rollout does not relax limits to select it.

## Objective and first useful case

Inline small, private scalar leaf routines when the resulting caller is
cheaper under an explicit 6502 cost policy. Preserve Action! storage and ABI
observability, and reuse the existing optimization and allocation passes.

AES supplies a useful acceptance case, but never a recognition rule:

```action
BYTE FUNC XTime(BYTE value)
  IF (value AND $80)#0 THEN
    RETURN((value LSH 1) XOR $1B)
  FI
RETURN(value LSH 1)
```

Current optimized NIR has one byte parameter, no locals, three reachable
blocks, and two returns. MIR6502 retains that structure and adds a store to
the public byte return slot at each return. The emitted body is 23 code bytes
plus one parameter byte; `MixColumns` contains four direct calls. Existing
known-callee result forwarding already lets the caller consume the result in
A, so the benefit must not count an imaginary result reload.

The first complete implementation must cover this small branching body.
Straight-line leaves are the first implementation slice, not the final scope.
No timing target is a legality or profitability predicate.

## Ownership and pass placement

Put the initial inliner in MIR6502, at `MirPhase::PreMaterialization`, before
`prepend_action_abi_param_prologue`, scratch reservation, storage layout,
block-argument lowering, and register-home selection.

At this boundary:

- `MirOp::Call` still identifies its callee, typed argument values, ABI homes,
  result definition, and effects;
- callee parameter loads still identify `MirMem::Param` storage;
- `lower_return_value_ops` has made the public return-slot stores explicit;
- blocks, logical temps, and block arguments can be cloned and verified; and
- ordinary MIR cleanup, allocation, and instruction selection can optimize the
  enlarged caller and expose its real storage pressure.

Keep discovery, cloning, and costing separate. NIR continues to supply typed
meaning and storage facts. MIR must not inspect source text, SemIR, routine
names, or benchmark constants to decide eligibility.

Add a small program-level preparation step shared by output generation and
materialized-MIR inspection. Runtime interface resolution must happen exactly
once per prepared input. Candidate trials use a non-recursive continuation
through the existing materializer; they do not run the inliner again.

Plain lowered MIR remains an inspection of lowering. Document that inlining
appears in prepared/materialized MIR. Ensure the modern configuration used by
`actionc-emit --emit-materialized-mir6502` matches normal modern code generation;
the inspection path currently passes `Mir6502Config::default()`.

## Existing mechanisms to reuse

| Concern | Existing code | Required extension |
| --- | --- | --- |
| Routine/address references | `standalone.rs` reference visitors; `linker::LinkGraph` | Extract reusable structured reference scanning, including parameter/local relocations; no second source linker. |
| CFG, use/definition identity, liveness | `analysis/cfg.rs`, `analysis/prehome.rs`, `analysis/use_def.rs` | Rebuild snapshots after each accepted expansion. |
| Cloned temps and operands | `materialize/small_loops.rs` remapping; `materialize/temp_rewrite.rs` | Extract a simultaneous ID remapper for the supported operations, block parameters, and edges. |
| Merged results | `materialize/block_args.rs` | Reuse block parameters, edge arguments, and parallel-copy lowering. |
| Parameter and ABI observability | `materialize/abi.rs`, `lower.rs`, `analysis/known_callees.rs` | Add a program-wide direct-call/escape census; existing local parameter elision alone is insufficient. |
| Bytes and cycles | `rewrite/posthome.rs`, `materialize/cfg.rs`, existing emission | Reuse estimates for ranking; account for complete calls, returns, paths, layout, and storage when selecting. |
| Reporting | `materialize/stats.rs` | Add candidate, rejection, selection, growth, and cost observations. |
| High-bit shift/XOR | `materialize/small_loops.rs` | Audit whether it fires after expansion; generalize its source/result proof only if needed. |

The operation-splice `MirRewritePlan` cannot represent a multi-block call
expansion. Use a small program/routine transaction with the same generation,
verification, and invalidation discipline. Do not broaden the existing effect
delta contracts merely to bypass their checks.

## Initial eligibility contract

Use a closed operation whitelist and named resource limits. Initial limits to
validate against fixtures and the candidate census:

- at most four reachable blocks and twelve MIR operations, including public
  return stores, with an acyclic CFG and only normal returns;
- zero to two byte scalar parameters, with direct, offset-zero reads only;
- a byte scalar result, or a procedure with no result;
- no remaining local storage, spills, allocated zero page, or static initializers;
- pure byte moves, arithmetic/bitwise operations, comparisons, and supported
  constant shifts; no multiplication, division, variable shifts, or operations
  that may lower to a runtime call;
- no callee calls, runtime helpers, foreign code, machine blocks, opaque
  barriers, volatile accesses, pointer accesses, global reads/writes, or
  absolute accesses other than the canonical ABI result stores;
- no physical register/flag dependencies already selected into the body;
- an ordinary `MirRoutineAbi::Action` entry, not a program, observable, fixed,
  current-location, runtime-bound, or external interface entry; and
- every return of a function supplies the declared result width through the
  canonical return store. Mixed, missing, or unrecognized result paths reject.

Use all reachable blocks for leaf classification. Freeze the original leaf
set for one invocation of the pass: expanding a call must not cause a newly
call-free parent to become another candidate in the same run.

Larger bodies, word/pointer parameters, residual local storage, memory-reading
leaves, and nested inlining are later extensions of these proofs. They are not
required for the first rollout.

### Persistent parameter storage and callable identity

Ordinary Action! parameters have routine-static storage. An inline call must
not silently stop updating storage that a later omitted-argument call or an
escaped address can observe.

Before parameter substitution, prove across the program that:

1. All calls to the candidate are known direct calls with the full argument
   list and exact supported widths. Reject the entire candidate if any call
   omits arguments, even if the proposed inline site supplies all of them.
2. Neither the routine address nor its parameter/local storage escapes through
   a value, initializer, descriptor, machine relocation, or runtime binding.
3. Parameters are never assigned or address-formed in the callee. No persistent
   local state survives into the candidate body.
4. No relevant opaque boundary or unresolved reference can observe the removed
   captures. Reuse existing visibility rules; uncertainty rejects the candidate.

`MirRoutineAbi::Action` alone is not a proof that a routine's address has not
escaped. Scan structured references by stable IDs, not printed names. Reuse
the existing NIR storage facts during lowering if a needed proof is not yet
represented in MIR, adding only the missing structured fact.

Partially inlining a candidate is safe only under this whole-candidate proof.
Retain the original routine and its parameter frame in the first release,
including after its last call is expanded. Do not credit dead-routine removal
in the cost model. A later removal pass needs its own complete root proof;
the source linker has already run and will not automatically remove it.

## Expansion and ABI contract

Create a plan against immutable caller and callee snapshots. Apply it to a
candidate copy, verify it, and publish it only after costing succeeds.

1. Split the caller block at the call into a prefix and continuation. The
   continuation inherits all following operations and the original terminator.
2. Capture each actual argument exactly once at the original call point.
   Already evaluated logical temps and constants may be substituted directly;
   reject unsupported lazy/address-carrier values. Do not copy or move the
   operations that originally evaluated argument expressions.
3. Clone callee blocks and all logical definitions using checked fresh IDs.
   Rebase definitions, byte-lane references, conditions, block parameters, and
   edge arguments with one simultaneous mapping. Callee IDs may overlap the
   caller's old and newly allocated IDs without cascading substitutions.
4. Replace supported parameter loads with uses of the captured actual values.
   The candidate proof permits omitting the private parameter captures. Do
   not add caller-local parameter cells merely to make cloning convenient.
5. Keep the canonical public result-slot store on every returning path.
   For byte functions this is the store to `$A0`. At each return, pass the
   stored logical value to a continuation block parameter that defines the
   original call result. A single return can simplify to a direct value
   substitution. A discarded result still preserves its public store.
6. Replace each callee return with a jump to the continuation, and the original
   call with entry into the cloned graph. A procedure needs no result parameter.
7. Rebuild temp inventories, CFG/use-def/liveness, routine effects, required
   fixed-zero-page metadata, and known-callee summaries. Preserve caller source
   attribution and retain readable inline-origin metadata for diagnostics.

Use the normal block-argument coalescer and subsequent passes to remove merge
copies. The inliner itself must not remove public return stores, infer that
they are private spills, or substitute host arithmetic for 8-bit operations.

The public return slot can be observed through raw code or pointer aliasing.
Preserving it also ensures that a later result-forwarding optimization has a
real definition to use. Machine-code callers or explicit consumers of physical
registers/flags require a boundary-state proof; reject them in this first slice.
Do not interpret a call's clobber set as permission to change memory effects.

## Profitability and compile-time bounds

Use two stages. A cheap body/argument census filters oversized or unsupported
candidates; a bounded trial through the existing backend measures the changed
caller after normal cleanup, allocation, and layout. An operation-count limit
alone must never select an inline expansion.

Cost the current accepted program and each candidate with identical origin,
runtime, and optimization settings. For the first implementation, a bounded
whole-program materialization trial is acceptable and avoids inventing a
second allocator or an isolated-caller layout approximation. Cache the current
baseline; invalidate affected facts after accepting a candidate.

Select sites individually or as a bounded `(caller, callee)` group. Grouping
allows the four `XTime` sites to be evaluated together without an exponential
search over subsets. Order candidates deterministically by estimated benefit
and stable IDs. Start with at most sixteen trials and eight sites per group.
When the budget is exhausted, retain the remaining calls and report it.

Measure or conservatively bound:

- total image/code growth, including new spills, changed branch encodings,
  parameter storage, and any helper dependencies;
- argument preparation, callee captures, `JSR`, executed callee path, `RTS`,
  and result handling in the original program;
- corresponding expanded paths, merge copies, spills/reloads, and changed
  surrounding pointer setup and loop edges in the candidate; and
- setup costs separately from costs repeated at the call site.

`estimated_6502_cost` currently treats a call as `(3 bytes, 6 cycles)` and some
unsupported operations as zero. It is a ranking aid, not a complete inline
cost model. Reuse its supported instruction costs and the CFG layout helpers;
extend the target-owned evaluator with explicit unknown costs, branch/return
costs, and page-crossing bounds. Never count an unsupported operation as free.
Use final emission in memory to check byte growth; no files need be written.

Start with the following conservative policy, expressed as named constants:

- require an estimated cycle saving of at least four cycles per invocation
  after uncertainty margins, with no supported path becoming slower;
- allow at most 32 added bytes per selected site, 128 per caller, and 256 per
  program, accounting for cumulative growth against the original baseline;
- account for the original routine remaining emitted;
- use no guessed loop frequency or branch probability;
- reject when costs cannot be compared safely or when surrounding changes
  cannot be bounded. Existing exact loop-trip facts may justify a setup cost,
  but syntactic loop nesting is not a measured execution count.

Compare corresponding execution paths, not independently chosen minimum and
maximum paths. Track stable clone/site provenance to relate original call
regions to expanded regions. If materialization changes an unrelated loop or
adds costs outside the modeled region, include those costs or reject the
candidate. This prevents a local call saving from hiding a caller regression.

These limits are initial rollout bounds, to be calibrated on the corpus rather
than to force AES through. Record rejection reasons and actual trial counts.
Trial reports must not leak into the normal optimization report, including
when reporting was requested through the environment. Baseline failures remain
compiler errors; unsupported candidate shapes are ordinary rejections, while
invalid cloned IR is an implementation error and must not be silently hidden.

## Composition with current optimizations

First measure what the existing passes select after expansion: parameter/value
forwarding, block-argument coalescing, accumulator retention, dead spill cleanup,
pointer reuse, and counted-loop selection. Compare with the no-inline baseline.

The existing high-bit shift/XOR selector is narrower than `XTime`: it expects
both arms to load and store the same ordinary home and jump to a common join.
`XTime` reads its parameter but writes `$A0`; after substitution it may use a
logical temp. Do not assume this optimization will fire automatically.

If this remains a measured blocker, generalize that selector's proof to a
common input value and a separately proven common output. Preserve output
stores and outgoing A/C/Z/N/V state, use existing reaching-definition and
liveness facts, and keep alias-sensitive memory inputs conservative. Use one
shared selector for recurrence and leaf-result forms. Its profitability must
also be tested without inlining, so independently useful improvements remain
available to retained calls.

Implement that extension as a separate slice only if existing materialization
does not already produce the useful form. Do not introduce an `XTime`, AES,
or polynomial-specific intrinsic.

## Implementation slices and commits

1. **Candidate analysis and reporting.** Add `analysis/leaf_routines.rs` and
   reusable reference scanning where justified. Record shape, visibility,
   argument, and growth blockers without changing code generation.
   Commit: `mir6502: analyze private small leaf inline candidates`.
2. **Straight-line expansion behind a disabled switch.** Add
   `materialize/inlining.rs`, extract the minimal remapper, preserve result
   stores, verify replacement definitions, and test one-return leaves.
   Commit: `mir6502: inline verified straight-line scalar leaves`.
3. **Small branching leaves.** Add block cloning and continuation/result
   merging, including `XTime`'s three-block/two-return shape, repeated sites,
   and caller-loop integration. Reuse block-argument lowering.
   Commit: `mir6502: merge return paths from small leaf expansions`.
4. **Costs and bounded selection.** Add the trial continuation, path/cost
   accounting, growth budgets, deterministic selection, and report details.
   Test rejection as carefully as acceptance; leave the switch off by default.
   Commit: `mir6502: cost leaf inlining after caller materialization`.
5. **Composition audit and any necessary generalization.** Inspect actual
   pass reports/listings. Extend the existing high-bit selector only if the
   evidence requires it, with independent positive and negative fixtures.
   Suggested commit: `mir6502: generalize shift xor result consumers`.
6. **Runtime validation and rollout.** Enable `enable_small_leaf_inlining` in
   `Mir6502Config::optimized()` only after correctness and cost checks pass.
   Keep it false in `Default`; preserve an explicit configuration override for
   A/B tests. Align materialized-MIR inspection with the chosen profile.
   Commit: `mir6502: enable costed small leaf inlining in optimized mode`.

If a slice exposes a missing safety proof, land its analysis/tests and retain
the call. Do not relax the proof or increase a budget just to improve AES.

## Validation and acceptance

Focused tests must cover:

- straight-line leaves, conditional and early returns, constants, repeated
  parameter reads, result reuse, discarded results, and procedure calls;
- all 256 byte inputs to the shift/XOR leaf, comparing inlining on/off;
- two inline sites in one block, multiple callers, callers with loops and block
  arguments, edge-only result uses, and overlapping old/new ID ranges;
- exact argument evaluation order, side-effecting argument producers retained
  once, omitted arguments elsewhere, parameter assignment, persistent locals,
  address escapes, initializers, raw-code references, and runtime bindings;
- public `$A0` values after calls, including discarded results and aliasing
  observations; caller-owned zero page and live values across the expansion;
- unsupported calls/helpers, cyclic bodies, pointer/volatile accesses,
  fixed/current-location entries, and observable machine state;
- cost thresholds, cumulative budgets, uncertain frequencies, tail-call
  baselines, page crossings, branch relaxation, new spills, and repeatable
  candidate ordering. A second preparation pass must not expand already
  expanded sites or recurse through newly exposed leaves.

Use source-to-MIR and executable regressions, not only hand-built MIR. Assert
the absence of selected calls and preservation of memory/results; do not lock
tests to the entire AES instruction listing. Run the relevant MIR fixture
sweep and full `cargo test` after behavior changes. If a slice changes NIR
facts or lowering, also run the required NIR fixtures and sweep.

Rebuild AES, CRC8, CRC16, sieve, and representative maintained/TN samples with
inlining on and off from the same compiler. Record checksums/output, XEX and
routine sizes, spill traffic, PAL ticks, compile time, trial counts, selected
sites, and blockers. Keep compatibility and modern/classic results as controls;
the new switch belongs to MIR6502.

For AES, the workload makes 106496 dynamic `XTime` calls
(`512 blocks * 13 MixColumns rounds * 4 columns * 4 calls`). Removing only
`JSR`/`RTS` represents 1277952 raw CPU cycles before accounting for new inline
jumps, captures, spills, and secondary optimizations. This is an explanatory
upper-level estimate, not a predicted PAL timing improvement or a cost credit.

Acceptance requires verified equivalent results, measured benefit for selected
sites within the stated growth bounds, transparent rejection when uncertain,
and a useful `XTime` composition through general mechanisms. Record the final
measurements here before enabling the optimized default.
