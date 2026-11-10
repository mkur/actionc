# MIR6502 pointer retention implementation

Starting revision: `97a60fa6`. Implements the contracts in the
[design note](MIR6502_POINTER_RETENTION_PLAN.md). Status: slices 0–5 complete, including exact-result composition.

Each numbered slice is a separate commit with its focused checks completed:
`dbce199d` (0), `90cf8dad` (1), `a46685d8` (2), `b44f84bc` (3),
`d69682ed` (4), `b4e55b66` (qualification), and `0a9c3fe1` (result composition).
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

Fresh slice-1 TN/TNDBG images are 9,945/13,041 bytes, each 14 bytes smaller
than baseline; only `SetWin` changes (1,079/1,082 routine bytes).

Slice 2 adds immutable-source CFG facts through the shared forward solver.
Reachable unknown entry is distinct from unreachable, and every join/backedge
must agree on both bytes. There is no assumed loop-entry residence. Constant
owned bases can survive an indexed store only when its full possible Y range
is contained in an owned ordinary allocation; unknown, foreign-RAM and wrapping
footprints remain barriers. Zero-page reads still execute and do not become
removable reads merely because other resident facts survive them. Hardware and
opaque barriers retain conservative handling.

Seven focused tests pass, including first-entry/backedge and disagreeing-edge
guards, bounded/foreign/wrapping indexed writes, 256-block/4,096-operation input
limits, and all four call transaction regressions. The new CFG query is not yet
an optimization consumer; the slice-1 selector retains its block-local policy.

Slice 3 selects CFG retention and prepares a pair once before a proven-entered
constant byte loop when entry state, pair deadness and source invariance permit.
It handles existing A/X/Y setup placement (two stores with zero, one or two
ordinary accumulator producers). Matching pair values authorize removed stores;
live A/N/Z retain the original producers. Unknown/zero-trip setup movement is
declined; existing dominating definitions can still be reused.

Indexed-store retention has a separate exact-window proof: the original pair
held `base+index`, whereas the selected pair holds `base` and Y holds the index.
The proof checks the original complete base, equivalent modulo-65536 effective
address, unchanged result reload, dead changed pair/Y/flags, generation and
original window. It rejects base/result reads aliased with the pair. This is
an explicit representation-changing region plan, not a relaxation of the shared
driver's removed-definition dependency check. Combined final-layout replay and
published callee contracts remain mandatory.

Eleven focused proof/transaction tests and four integration cases pass. Two VM
tests qualify both fixtures across the complete raw/optimized, runtime, origin
and LF/CRLF matrix. The fixed-loop case preserves all 64 indirect reads in order,
decreases pair stores from 128 to 2 and takes 1,696 rather than 2,578 cycles.
The bounded conversion case saves 18 cycles and two pair stores per iteration;
its zero-count path stays at 60 cycles. The one-call case now takes 78 cycles
versus 92 at baseline. Changing-home promotion remains independently disabled.

Slice 4 enables a separate costed changing-home transaction after accepted
call/loop retention. It admits one ordinary unexposed local word in an entered
constant-bound scan, with complete owned-buffer bounds, unit updates, exclusive
existing-pair use and one audited exit. Initial home stores become pair stores
at their original point; no observer may intervene. Loop setup copies disappear,
word updates target the pair, and the original home is reconstructed before
exit consumers. Entry/exit A/N/Z, pair liveness, physical aliases, frame exposure
and source-memory observations are checked; frames remain allocated.

This initial subset declines incoming parameter pointers, unknown/zero-trip
regions and competing changing pointers that need joint placement. In particular
it does not infer a closed-entry contract for `Fnamecmp` or silently defer its
observable parameter updates. Broader admission needs separately proved bounds,
entry/observation contracts or joint placement, as described in the design note.

Fourteen focused proof/transaction tests pass, plus the 13 pointer/ABI integration
cases. Five tests pass across the pointer-retention and absolute-readback VM
targets. Feature-off/calls/loops/promotion execution covers raw/optimized NIR,
both runtimes, three origins, actual LF/CRLF parsing and a promoted low-byte
carry. Incoming scans also execute a full word wrap and a pointer into their
own parameter home; their feature-off/on images remain identical. `ScanFixed` takes 1,156 cycles versus 1,629 at baseline and replaces 64 repeated direct
setup stores with two initial setup stores; required pointer increments remain.
All buffer bytes and published final pointers are checked independently.

The result-composition slice finishes the planned exact-result reuse. An indexed
store can load its index directly into Y, preserving the call result in A.
After a successful pointer rewrite, the shared accumulator equality query and
structural exit-state proof can remove a redundant typed ABI-result reload.
Raw absolute readbacks and live N/Z remain barriers. Variable-length indexed
plans preserve fused-compare producer sites. Sixteen focused unit tests and
three pointer VM tests pass, including the flag/readback negatives. The neutral
conversion now saves 23 cycles per iteration; its zero-count path is unchanged.

## Final qualification

Code revision: `0a9c3fe1`; frozen baseline: `97a60fa6`. The
[scorecard](benchmarks/6502-pointer-retention/results.json) contains entry-source,
compiler-binary and image hashes, routine sizes, occupied private zero-page
lanes, feature-off/on VM measurements and compiler cost samples. Full local
assembly, XEX, map and materialized MIR artifacts are under
`build/pointer-retention/final/`; compiler-generated TN listings are also copied
to `build/tn-mir6502/TN.asm` and `TNDBG.asm`.

| Artifact | Baseline bytes | Final bytes | Saved |
| --- | ---: | ---: | ---: |
| TN XEX | 9,959 | 9,891 | 68 |
| TNDBG XEX | 13,055 | 12,987 | 68 |
| TN `SetWin` | 1,093 | 1,025 | 68 |
| TNDBG `SetWin` | 1,096 | 1,028 | 68 |
| Neutral XEX | 245 | 214 | 31 |
| `ReadAcrossCall` | 40 | 30 | 10 |
| `ConvertSpan` | 94 | 77 | 17 |
| `ScanFixed` | 63 | 59 | 4 |
| Invariant fixture XEX | 87 | 77 | 10 |
| `ReadFixed` | 61 | 51 | 10 |

Private zero-page occupancy stays at 33 lanes for TN, 37 for TNDBG and zero
for both neutral fixtures. Frames and reservations are identical. `Strcpy`
stays at 41 bytes. `Fnamecmp`/`Print`/`PrintFname`/`Instr`/`Convert` stay at
185/58/58/66/131 bytes in both TN variants. Saved pointer spill stores remain
unless a separate observation proof permits their removal.

| Executed region | Baseline cycles | Final cycles | Direct pair setup stores |
| --- | ---: | ---: | ---: |
| `ReadAcrossCall` | 92 | 78 | 4 → 2 |
| `ConvertSpan`, zero elements | 60 | 60 | 0 → 0 |
| `ConvertSpan`, one element | 176 | 153 | 4 → 2 |
| `ConvertSpan`, 255 elements | 29,640 | 23,775 | 1,020 → 510 |
| `ReadFixed`, 32 iterations/two reads | 2,578 | 1,696 | 128 → 2 |
| Promoted `ScanFixed`, 32 iterations | 1,629 | 1,156 | 64 → 2 |
| Actual TN/TNDBG eight-character `SetWin` loop | 963 | 827 | 32 → 16 |

These are VM-reported cycles including executed callees. Direct setup counts
exclude required RMW pointer increments. The actual TN region shrinks from 57
to 45 bytes. Saved-image execution checks both variants, origins `$7000` and
`$70FE`, six byte patterns, all buffer guards, final index and stack state.
The neutral loop retains all 64 pointee reads in their original order. The
review's proposed 19-byte loop remains a future target.

Single sequential debug CLI samples (including source listings) take TN
14.994 → 16.511 seconds and TNDBG 46.883 → 49.808 seconds, increases of about
10.1% and 6.2%. These samples use the pinned baseline/final compiler binaries
without concurrent compiler jobs; they are measurements rather than a timing
threshold. Maximum resident bytes and binary hashes are recorded in the
scorecard. The additional analysis cost is a tradeoff of the final proof/layout
transactions.

Final checks pass:

- 1,163 MIR6502 library tests (two existing ignored) and 15 tracked-emitter tests.
- All 13 `mir6502_*` integration targets: 100 passing tests.
- All 169 MIR6502 emission/load fixtures.
- MADS oracle: 59 compiler cases and 228 assembled listings, including both new
  fixtures, both runtimes, plain/source listings and re-origining.
- Nine affected VM targets: 18 passing tests, covering pointer retention,
  absolute readbacks, register forwarding, record-base reuse, indirect bitwise
  access, static pointer copies, value copies and TN behavior/reproducers.
- Complete TN/TNDBG listings reassemble byte-identically; changing only TN's
  origin from `$2C00` to `$3C00` matches direct compilation at the new origin.

The pointer VM matrix independently compares feature-off/calls/loops/promotion,
raw/optimized NIR, cartridge/standalone runtimes, three code origins, repeated
boundary inputs and actual LF/CRLF parsing. Incoming-pointer fallback executes
both full word wrap and a pointer into its own parameter home. Hardware/bank
reads and writes, unknown/unowned writes, partial/clobbered lanes, source
mutation, CFG disagreement, stale plans, live flags/Y/pair, escapes, competing
pointers and unsupported bounds have focused negative coverage. No shared
frontend/NIR contract changed, so qualification is scoped to MIR6502.

## Remaining TN proof obligations

- `SetWin`: the destination base comes through directory-buffer and `v`/`j`
  flow. A complete owned bound for every indexed write is still unavailable.
  The write can therefore alias the pair or source cells on the backedge;
  call retention succeeds, while full loop hoisting declines.
- `Fnamecmp`: two changing incoming parameter pointers need joint placement and
  an entry/observation contract. A pointer can address its own parameter homes;
  non-escape and a direct-call census cannot justify delaying their updates.
- `Print`/`PrintFname`: `Putchar` writes indirectly through the screen pointer.
  The call chain needs a bounded transitive write footprint proving disjointness
  from the retained pair and source cells.
- `Instr`: dynamic length, partial-lane setup and absolute-backed index writes
  remain outside complete-pair and owned-write proofs.
- `Convert`: destination writes need narrower proven index/range footprints to
  retain an unknown incoming base through them.

The next vertical slice should add the smallest typed range/origin proof needed
for one of these boundaries, with a neutral positive case and physical-alias
negatives. Broader incoming-pointer promotion or a new NIR entry contract needs
its own design and shared-contract qualification. The current implementation
makes no closed-entry assumption and adds no TN-specific selectors.
