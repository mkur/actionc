# Call predicates

[Implementation plan](../call-predicate-implementation-plan.md).

Stage 0 freezes compiler `a15f9f36` and the executable SHA in
[stage0.json](stage0.json). The generic `Forward` still needs five spill bytes.
Both intact roots match the address-value qualification images, including
classic. Actual LF/CRLF compilations produce identical images in both runtimes
for the fixtures and in cartridge mode for both roots.

The new `call_predicates` VM target checks original snapshots, effect order,
normalization and intact-root `SetTagged` status, bitmap guards and counts.
Together with `tn_codegen_reproducers`, six tests pass and one new regression
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

Stage 4 completes qualification. Explicit address-reference facts distinguish
address exposure from content ranges, including word high lanes and combined
address/content operands. Snapshot admission excludes loops and ambiguous
incoming joins. A later call that mutates only the earlier word's high byte also
preserves the original snapshot. The expanded snapshot fixture includes this
new routine, so its whole-image size is not comparable with Stage 0.

The integrated result is recorded in [stage4.json](stage4.json):

| Cartridge measure | Baseline | Final |
| --- | ---: | ---: |
| `Forward` code, including ABI payload | 70 | 39 |
| `Forward` spill / parameter bytes | 5 / 5 | 0 / 5 |
| TN load-file bytes | 16,802 | 16,766 |
| TNDBG load-file bytes | 20,539 | 20,503 |
| Valid `SetTagged`, setting 0, cycles including call wrapper | 842 | 802 |
| Valid `SetTagged`, settings 1/2/255, cycles including call wrapper | 930 | 890 |

Enabled/disabled library configurations execute the generic wrapper, ordinary
local sources, a non-tail WORD result and other ABI placements in both runtimes
with actual LF/CRLF files. The exclusive `Forward` cycles are 80/82 with both
selectors disabled, 48/50 with capture removal, and 40/42 with both enabled.
Executed direct spill operands fall from 10 to 2 to zero. Parameter operands
remain five. The independent local wrapper also saves 40 cycles and removes
all ten spill operands; its ordinary local accesses remain eight.

These traffic/cycle counts include only instructions whose PC is inside the
named wrapper. They exclude callee/resident-helper execution, stack, indirect
pointer fetches and fixed ABI/runtime scratch. Canonical parameter captures
performed by SArgs remain in place and are outside this exclusive traffic
census. Whole-call cycles are recorded separately. Cycle values are observations,
not correctness assertions.

Qualification passed 1,048 MIR6502 library tests (two existing ignored), all 96
MIR6502 integration tests across eleven targets, and the full VM crate's 392
tests. This includes nested calls, both runtimes, TN dispatch, directory/MyDOS
models, sort/tag/publication vectors and the completed address-value consumers.
Final roots match Stage 3 byte for byte, classic hashes are unchanged, and
full-root LF/CRLF images match. No SemIR/NIR contracts or snapshots changed.

Calls, hardware/absolute/indirect sources, incomplete or extra uses, loops,
ambiguous joins, physical conflicts and unknown/non-profitable cost keep their
captures. Boolean register homes retain their existing carrier; `OtherHome`
demonstrates that fallback. `PredicateFirst` retains its one-byte source local.
The two optimized selectors have independent configuration flags, with ABI
carrier selection requiring capture selection and its final cost gate.

The final compiler, listings, images, raw measurements and logs are retained in
ignored `target/tn-call-predicates/final`. The temporary build cache is removed
after qualification; historical frozen compilers and user work are preserved.
