# MIR6502 pointer retention implementation

Starting revision: `97a60fa6`. Implements the contracts in the
[design note](MIR6502_POINTER_RETENTION_PLAN.md). Status: slices 0–1 complete.

Each numbered slice is a separate commit with its focused checks completed.
All selection is MIR6502-only, optimized-only and independently switchable.
Keep the previous accepted program on proof, layout, verifier or cost failure.

0. Freeze TN/TNDBG and neutral call/loop/scan artifacts, hashes and frame counts.
   Add a maintained neutral fixture and baseline compilation/execution checks.
1. Build owner-qualified physical write footprints from existing exact machine
   summaries and complete direct MIR callees. Extend resident-base analysis
   across proven calls within a block. Select redundant complete setups through
   structural plans, preserving the original high load for A/N/Z. Replay against
   proposed layout/summaries and require a strictly smaller final image.
2. Extend the same resident-base state to forward CFG must-agree facts using the
   shared solver. Keep unreachable distinct from unknown entry. Admit indexed
   writes only when the entire possible byte-index footprint lies in owned RAM;
   invalidate physical overlaps. Test joins, backedges and unsupported writes
   before enabling CFG selection.
3. Enable invariant CFG retention and safe setup placement in natural loops.
   Start with existing dominating pair definitions; move setup only when source
   invariance, ordinary reads, entry state and complete path costs are proved.
   Qualify the bounded conversion fixture and measure actual TN blockers.
4. Add an independent changing-pointer selector. Initially require an ordinary,
   unexposed local word initialized to owned storage, a bounded counted region,
   unit updates, exclusive pair use and an auditable footprint. Reconstruct
   pointer homes at every exit and preserve live A/flags. Incoming/self-aliasing
   parameter scans and unsupported regions retain their existing code.
5. Qualify the complete MIR6502 backend, listing/MADS and affected VM targets.
   Record sizes, setup traffic, cycles, occupancy, compiler cost and remaining
   TN blockers separately for calls, invariant loops and pointer promotion.

Implementation homes:

- Extend `analysis/record_bases` with shared retention/CFG queries; reuse its
  byte identities and final physical map. Do not add an emitter state tracker.
- Keep physical call footprints owner-qualified and bounded. Unknown recursion,
  calls, indirect writes and opaque effects reject narrower footprints.
- Add final `materialize/pointer_retention` transactions after existing placement
  choices. Keep frame allocations and reservations unchanged. Use the existing
  baseline-reference emission hook to resolve removed references during replay.
- Revalidate published callee A/preservation facts and all consumed call proofs
  against the combined proposal. Rebuild snapshots after dependent rewrites.

Focused checks accompany each behavior: exact call/source/pair aliases and ABI
effects; must-agree joins and initial loop entries; owned indexed-write bounds;
private-pointer observation and exit state; final-cost rollback. Execution uses
feature-off/on, raw/optimized NIR, both runtimes, relocation, repeated boundary
inputs, guards and actual LF/CRLF parsing. Final checks follow the design note
and repository backend scope. No shared NIR change is planned; if required,
specify and qualify that contract before using it.

## Results

Slice 0 freezes [baseline.json](benchmarks/6502-pointer-retention/baseline.json)
and complete local artifacts under `build/pointer-retention/before/`.
Neutral/TN/TNDBG XEX sizes are 245/9,959/13,055 bytes. The neutral call,
conversion and scan routines are 40/94/63 bytes. Baseline VM data, complete
buffer guards and final-pointer checks pass with both runtimes, raw/optimized
NIR, two origins and actual LF/CRLF input. Setup traffic and cycles are frozen
in the scorecard. Proof-specific blocker diagnostics arrive with their first
consumer in slice 1 rather than adding a temporary second analysis in slice 0.

Slice 1 connects complete-base equality to owner-qualified physical callee
writes, including transitive bodies, ABI argument placement and existing bounded
machine summaries. Exact result/register facts remain in their existing shared
analysis. Selection applies one validated pair rewrite per snapshot and retains
the high-byte load for exact A/N/Z. Final baseline-reference layout replay,
callee A contracts, identical frames and strict final emission cost are checked.
The independently controlled call pass runs after earlier placement choices.
Blocker counters describe attempts, not additive savings.

The neutral `ReadAcrossCall` executes with two rather than four pair writes and
takes 82 rather than 92 cycles in both runtimes. Its image/routine shrink by
seven bytes. Focused checks pass: four new proof/transaction tests, nine existing
resident-base tests, two integration cases, and the pointer/readback VM targets
(three tests). Feature-off/on execution covers raw/optimized NIR, both runtimes,
two origins and actual LF/CRLF input. Full TN measurements follow separately.
