# Exec816 call and placement audit after stage 7

The next compiler investment should improve value flow at call boundaries within
the current public ABI: arguments entering calls, results reaching their consumers,
and invocation-owned values surviving calls. Push-based argument construction and
immediate call-to-return forwarding already cover their intended common cases.
Arithmetic helpers contribute a much smaller static footprint.

This is a fresh observation of compiler `af7c504c` using Rust 1.99.0 and the frozen
Exec `57df0d7` workload: 256 source files and 1,156 routines. All three rebuilt
compiler images match the stage-7 images exactly, including their serialized
metadata. No compiler strategy, ABI or acceptance target changes in this audit.
The current live Exec checkout is a different workload.

## Where call code goes

| Profile | Compiler code bytes | Before transfer | Transfer | After transfer | Call guards | Total call bytes excluding guards |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 430,091 | 60,839 | 17,935 | 38,343 | 0 | 117,117 |
| Optimized guarded | 579,765 | 64,159 | 17,935 | 41,721 | 119,178 | 123,815 |
| Raw guarded | 648,963 | 67,321 | 17,983 | 41,653 | 119,502 | 126,957 |

Release call sequences occupy **27.2% of compiler code**. Argument construction
and post-transfer work together occupy 99,182 bytes, or 23.1%. These spans include
ABI work that must remain; their footprints are not savings estimates. Production
and consumption of arguments/results outside the call spans are counted separately.

The optimized profiles contain 4,486 logical calls across 932 routines: 4,443
direct, 40 arithmetic-helper and three indirect calls. Their final shapes are
4,406 push-based calls, five direct reserve/store calls, three indirect calls and
72 terminal forwards. The image records 4,414 ordinary calls; the forwards use
terminal jumps and therefore have no new call frame. Of the 4,486 logical calls,
455 occur in cyclic blocks. This does not establish their execution frequency.

The transfer column measures the actual transfer instruction, including the
three one-byte indirect transfers; it does not assume every transfer is a
four-byte `JSL`. Pre/post columns partition each complete final MIR call span.
Guarded emission has additional non-guard instructions as well as explicit
guards, so subtracting guard bytes does not reconstruct a release package.

## Results and arguments

Optimized release has 2,719 logical result values. Actual stores into their
canonical homes occur for 2,216 values and occupy 6,624 instruction bytes.
The 2,654 reserved stack homes overstate capture frequency: a home can remain
reserved after its capture has been eliminated.

| Sole adjacent result consumer | Values with actual capture stores | Capture store instruction bytes |
| --- | ---: | ---: |
| Compare | 868 | 1,956 |
| Store | 381 | 1,314 |
| Cast | 153 | 524 |
| Another call | 94 | 348 |
| Other consumers | 76 | 254 |
| Return | 1 | 4 |
| **Total** | **1,573** | **4,400** |

These are exact selected store bytes, excluding surrounding loads, mode changes
and cleanup. Their consumers can have effects and lane requirements that prevent
elimination. The table defines useful implementation cohorts, not eligibility
proofs or promised reductions.

Immediate return forwarding is already effective: 498 of 499 sole adjacent
call-to-return values have no capture store. The remaining site is indirect in
`PROCESS.ExecuteImage`. Of the 439 such values with reserved homes, 438 have no
store. Reopening general immediate-return capture work would largely repeat
completed work.

Arguments use 4,499 distinct temps; 1,750 have actual producer capture stores,
occupying 6,626 instruction bytes. There are 2,052 temps with a reserved home and
a sole call use, of which **1,603 still have capture stores**, occupying 6,078
bytes. Within that group, 1,027 have an adjacent producer/call pair and actual
stores, occupying 4,070 bytes. The reserved-home group includes 978 three-byte
and 491 four-byte values.

The argument and result footprints overlap: 209 argument values are also call
results, accounting for 772 store bytes in both totals. Do not add these totals
as independent opportunities. Argument captures also preserve when memory is
read: among the 901 captured `Load` arguments, 617 read indirectly, while 228
read immutable parameters or private frame objects. Any delayed read or reuse
needs the existing effect and storage proofs; an apparent single use alone is
insufficient.

## Values across calls and policy coverage

Logical liveness identifies 660 distinct values live across calls in 258
routines, with 1,959 value/call crossings. They include 307 pointers. Widths are
108 one-byte, 164 two-byte, 311 three-byte and 77 four-byte values. There are
937 calls with at least one live value.

The sealed placement census reports 2,743 mixed homes and 243 loop homes, but
only **one call segment in one routine**, `INPUT.Acquire`, in both optimized
profiles. Raw mode reports no call segments. This demonstrates limited coverage
of the current invocation-backed call-segment policy. It does not prove that
every live value would benefit from a new residence.

The current policy intentionally limits call segments to three-byte values and
requires enough distinct pointer-preparation misses to repay complete reloads.
Invocation stack homes remain authoritative across calls; DP and register caches
do not imply survival across a callee. Broadening this direction should preserve
those ownership rules and choose transfers by measured complete cost.

The audit also groups all typed temps into ordered diagnostic cohorts. Examples
include 660 crossing calls, 2,536 other call results, 1,283 other sole call
arguments and 9,191 other stack-backed values outside current mixed-residence
widths. These are **census categories**, not exact internal allocator refusal
codes. A canonical stack home can coexist with a register or DP cache. Counts
of rejected forwarding requests likewise do not establish profitable missed
optimizations. The full census and selected request outcomes remain available
in [results.json](results.json) and [captures.csv.gz](captures.csv.gz).

## Helper footprint

Nine arithmetic helper bodies occupy 817 release bytes. Their 40 call sequences
occupy another 1,384 bytes excluding guards: 2,201 bytes combined, or 0.51% of
release compiler code. The largest call-site group is unsigned four-byte
multiplication, with 14 sites. [helpers.csv](helpers.csv) separates each body's
size from its caller footprint.

This makes helpers a lower priority for whole-kernel size than general call
value flow. Static frequency does not rank their runtime cost; a separate native
workload measurement is needed before drawing a latency or cycle conclusion.

## Recommended development order

1. **Common value flow into and out of calls.** Use the existing typed logical
   analysis and sealed placement model to cover producer/argument and
   result/consumer relationships consistently. Start with the frequent adjacent
   result consumers and argument-only values above, while retaining capture
   timing, ABI lanes, clobbers and conservative memory effects. Measure complete
   sequences rather than isolated stores.
2. **Invocation-owned placement in call-heavy routines.** Expand common-model
   coverage where measured use patterns repay transfers between calls. Include
   scalar and pointer lifetimes, pressure and joins in the assessment. Keep
   invocation ownership and call barriers explicit.
3. **Runtime helpers selected by execution evidence.** Address a helper when
   native measurements identify a material workload cost; its presence alone is
   not a reason for broad arithmetic work.

Treat this as direction-setting evidence. An implementation plan should define
bounded slices and independent native oracles before changing selection policy.
The [argument/result investigation](argument-result-flow/README.md) traces the
first direction into typed consumer shapes, planner constraints and proposed
implementation slices.
Continue the existing per-routine resource, raw/optimized, guarded/release,
interrupt, aliasing and relocated-image qualification. The release compiler-cost
median was already close to its review limit at stage 7.

The [stage-7 obligations](../65816-record-placement-stage7/README.md) remain:
7,391 release code bytes, 476 representative bytes and 90 measured native private
accesses above their final gates, plus the unsupported unchecked hosted-provider
combination. The 256 KiB complete-application objective, linked platform assembly,
commands and hosted fixture repairs remain separate from this compiler audit.

## Evidence and reproduction

The optional `record_probe --features call-analysis` observer reads prepared typed
MIR, logical uses/liveness, canonical physical homes and sealed placement
summaries. It replays final selected actions with observations enabled and checks
encoding and metadata equality before exporting instruction effects. The report
requires complete nonoverlapping effect coverage, valid instruction boundaries,
home geometry, call spans, transfer census and matching image hashes.

[examples.lst](examples.lst) contains exact release excerpts for `SDFSFILE.Open`,
`COOKEDLINE.Recall`, `SDFSFILE.Measure` and `TASKPOLICY.Switch`.
[calls.csv.gz](calls.csv.gz) retains every call;
[routines.csv.gz](routines.csv.gz) retains routine size/frame/peak and call totals.
[provenance.json](provenance.json) binds compiler/probe sources, source files,
generated layouts, toolchain, commands and input/output hashes.
[evidence-sha256.json](evidence-sha256.json) authenticates the generated report files.

The static access census counts byte accesses once per selected non-transfer
instruction, excluding guards and ABI call/return summaries. It is not executed
private traffic or a replacement for stage-7 native measurements. This audit
does not add VM, hosted, hardware or compiler-speed qualification claims.

Restore the frozen stage-0 directory using its
[input manifest](../65816-record-placement-stage0/inputs.json.gz), including
generated profile sources/layouts. From the repository root:

```sh
python3 -B tools/compare65816/exec_call_audit.py collect
python3 -B tools/compare65816/exec_call_audit.py report
python3 -B tools/compare65816/exec_call_audit.py report --check
python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_call_audit.py'
python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_record_*.py'
```

Collection uses the frozen sources, writes bulky observations under
`target/exec-call-audit` and rejects any compiler image that differs from stage 7.
It leaves the frozen and historical artifacts untouched. Report checking
recomputes all published generated files from authenticated observations.
Recorded absolute paths are host-specific; regenerated provenance on another
host will differ while the pinned compiler images must remain identical.

Validation for this audit: all three profile collections and report checking
pass; the eight audit tests and 27 existing record-tool tests pass. The probe
passes `cargo check --all-features` under Rust 1.99.0. Tests cover forged/missing
observations, exact call partitioning, terminal forwards, actual capture stores
versus reserved homes, stack coordinates and LF/CRLF CSV parsing. Compiler/NIR
contracts are unchanged, so full backend and shared-contract suites were not
rerun for this observation-only change.
