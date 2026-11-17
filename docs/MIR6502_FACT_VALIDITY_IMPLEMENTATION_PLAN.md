# MIR6502 fact validity implementation plan

Status: implemented and qualified in three slices, 2026-11-17.

Implements issue **8** from the [repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md),
following [summary validity across transformations](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#summary-validity-across-transformations).
Commit the design/plan and each implementation slice. Keep frontend/NIR, emitted
instruction recipes, ABI eligibility and private-alias policy unchanged.

## Slice 1 — Shared compatibility checks

- Extract the terminal transaction's callee-contract comparison into the
  existing known-callee analysis. Preserve finite read/write containment,
  observable accumulator and published flag provenance, entry-flag obligations,
  register preservation and physical preservation with matching fixed backing.
- Keep immutable versus rewrite publication policy explicit. Check single-byte
  preservation as well as pairs; matching IDs alone must not certify changed
  backing. Do not add full native entry/result analysis to consumers that do not
  publish those facts.
- Migrate the terminal transaction without relaxing its verifier, physical replay,
  cost or rollback guards. Keep stricter transaction-specific comparisons.
- Add focused compatibility regressions for stronger/weaker footprints, changed
  exit state, entry flags, backing and policy. Run affected analysis and terminal
  tests; commit.

## Slice 2 — Bound core reuse and restore dependent decisions

- Put the three existing core summary lifetimes under an explicit checked scope.
  Reuse the analyses already built at the structural/cleanup, ZP-reuse and final
  cleanup boundaries; validate freshly produced facts before they authorize the
  next group. Rebuild once at the end of the final changed group.
- Validate the source interfaces, call dependencies and storage interpretation
  required by each scope. A local generation rebuild cannot refresh those
  dependencies. Keep mandatory representation lowering explicit.
- Retain one checkpoint before the first core summary consumer. If a later
  boundary weakens a published guarantee, restore dependent caller rewrites,
  home-fate bookkeeping and optimization statistics together. Re-run this bounded
  tail with conservative external-callee facts, so required lowering still runs.
  This avoids a new replay registry and does not retry the complete compiler.
- Trace analysis builds, checked boundaries and fallback reasons. No-op boundaries
  reuse their current facts; do not analyze per instruction or query. Keep maps
  local to their attempt; final emitted-address checks remain in their existing
  transactions.
- Pass the same scoped bundle to register-reload folding, removing its internal
  immutable-policy analysis so conservative fallback cannot bypass the owner.
- Add focused scope/consumer tests: callee/descendant mutation, metadata and home
  changes, a compatible change, complete rollback, opposite routine order and
  an unchanged-boundary analysis count. Retain existing entry-stage and physical
  replay controls. Run the affected tests and commit.

## Slice 3 — Qualify and document

- Run focused integration controls for the materialization/terminal boundaries
  and a small VM matrix preserving caller state across calls, with both runtimes
  and LF/CRLF through the actual source loader. Reuse existing controls where
  they cover the behavior; no full suites by default.
- Run the retained 13-control/95-routine audit. Compare against issue 7: facts,
  placement, selected MIR, actual TN image size/hash, analysis counts and compile
  time. Explain any fallback or code-size change; savings are not required.
- Record independently confirmed baseline failures without changing unrelated
  expectations. Save a qualification receipt, update the design/status/plan,
  verify links and diffs, remove temporary audit hooks, and commit.

## Acceptance

An accepted candidate preserves every callee guarantee used by its earlier
rewrites, or those decisions are reconstructed from the checkpoint without the
invalid assumptions. Fresh analysis alone never legitimizes already-applied
dependent changes. Existing entry facts remain stage-scoped; candidate output,
maps and proof state remain paired. No persistent cache or per-query dependency
ledger is introduced.

## Progress

Slice 1 shares the compatibility query with terminal cleanup, checks the
publication policy and fixed backing, and retains individual entry flags and
single-byte preservation. Five new regressions plus the affected analysis and
terminal controls pass: **143 passed, three diagnostic tests ignored**. No full
suite was run.

Slice 2 owns the complete core tail checkpoint and refreshes summaries at the
existing group boundaries and core output. Incompatible metadata or guarantees
cause one conservative retry, restoring code, home records and statistics. The
reload-folding pass now borrows this same policy. Four new regressions cover
scope reuse, metadata/placement invalidation, a real caller carrier rewrite in
both routine orders, and conservative reload folding. **165 focused tests pass**
(four ignored diagnostics, including the separately executed audit).

Slice 3 qualification is complete: **four integration tests**, **three VM
controls** and the **13-control/95-routine audit** pass. VM coverage includes
both runtimes, default/optimized modes, LF/CRLF, aliasing and live caller state.
Trace and work accounting preserve the generated image. No full suite was run;
previously documented unrelated failures were not rerun or altered.

TN grows **9,811 → 9,926 bytes (+115)**. All six materializations decline `Pull`'s
expanded writes at `post-zp-remapping`: new physical writes to `$E2–$E5` weaken
the earlier contract. The conservative retry retains more reloads, including
in unrelated callers, because this initial scope tracks the whole published
bundle rather than individual queries. Entry proofs and parameter/local home
bindings are unchanged. `Alloc` and `Free` now retain a read of `$A1`; final write
summaries are unchanged. Six independent global-state controls grow by two bytes
each; the other seven controls are unchanged.

The TN run builds **12 core bundles**, checks six changed boundaries and takes
six bounded fallbacks. The old core had four analysis calls per full traversal;
the new TN path builds two per materialization and none during conservative
replay. This does not make compilation faster: the uninstrumented audit rises
**21.11 → 24.07 seconds** in single local debug/opt-level-1 runs. TN's separately
measured MIR-through-output compilation takes **23.46 seconds**. These are
observations, not a controlled benchmark.

See the [qualification receipt](benchmarks/6502-call-facts-audit/fact-validity-qualification.json)
for hashes, routine deltas, trace evidence, commands/scopes and timing limits.
Recovering the 115 bytes is follow-up optimization work: preserve the published
contract or replay with narrower conservative assumptions. Fresh summaries alone
cannot justify keeping the old caller rewrites.
