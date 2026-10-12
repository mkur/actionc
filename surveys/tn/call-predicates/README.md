# Call predicates

[Implementation plan](../call-predicate-implementation-plan.md).

Stage 0 freezes compiler `a15f9f36` and the executable SHA in
[stage0.json](stage0.json). The generic `Forward` still needs five spill bytes.
Both intact roots match the address-value qualification images, including
classic. Actual LF/CRLF compilations produce identical images in both runtimes
for the fixtures and in cartridge mode for both roots.

The new `call_predicates` VM target checks original snapshots, effect order,
normalization and intact-root `SetTagged` status, bitmap guards and counts.
Together with `tn_codegen_reproducers`, seven tests pass and one new regression
is temporarily ignored: MIR6502 reloads an earlier argument after a later call
mutates its parameter through a passed address. Classic preserves the snapshot.
This correctness defect must be repaired before enabling capture removal.

The repair freezes typed address exposure before temp cleanup. An internal
callee summary cannot authorize moving a home load across a call once that
home's address can escape. All four `call_predicates` tests now pass, including
the unignored alias regression, in both runtimes with actual LF/CRLF inputs.

Direct TN/TNDBG MIR `SetTagged` at ordinal zero costs 842 cycles for setting 0
and 930 for settings 1/2/255. Out-of-range ordinal 20 costs 516/518 cycles.
These vectors execute the wrapper; other directory benchmark cycles cannot be
attributed to it. Cycle observations are not correctness assertions.

Large images, listings, raw MIR and logs live in ignored
`target/tn-call-predicates/stage0`. One debug-free, nonincremental cache is shared
by compiler and VM checks.

Stage 1 adds generation-scoped snapshot facts and context queries with complete
word-lane, exclusive-use, dominance, pure-region and physical-disjointness
requirements. Two focused proof tests pass. Production selection is disabled;
all four root images remain byte-identical to Stage 0 with actual LF/CRLF
compilation. The snapshot fixture grows by 36 bytes to preserve the escaped
parameter correctly; this is a correctness repair, not an optimization result.

Stage 2 removes the four earlier word spill bytes while retaining the Boolean
carrier. `Forward` uses 46 code bytes (including inline ABI payload), one spill
byte and five parameter bytes, versus 70/5/5 at Stage 0. Both MIR roots shrink
by 28 load-file bytes to 16,774 and 20,511; classic hashes are unchanged.
Four focused proof/transaction/cost tests, 79 targeted integration tests and
seven VM tests pass. Both runtimes and actual LF/CRLF sources are covered;
full-root LF/CRLF images match. Direct valid `SetTagged` vectors save 32 cycles.
[Stage 2 measurements](stage2.json) separate code, homes and complete images.

Stage 3 removes the Boolean carrier through one atomic verified diamond.
`Forward` now uses 39 code bytes, zero spill bytes and five parameter bytes.
Both MIR roots shrink by 36 bytes relative to Stage 0: TN 16,766, TNDBG 20,503.
Classic hashes are unchanged. Seven focused units, 79 integrations and seven
VM tests pass, covering both runtimes and actual LF/CRLF compilation. Direct
valid `SetTagged` vectors save 40 cycles (802/890 versus 842/930).
[Stage 3 measurements](stage3.json) pin the qualified executable and images.
