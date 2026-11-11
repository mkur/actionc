# Callee entry flag summaries measured results

The [four-slice implementation](../../MIR6502_CALLEE_ENTRY_FLAG_SUMMARIES_PLAN.md)
refines incoming flag facts for generalized argument A to X forwarding.
`enable_precise_callee_entry_flags` is enabled by
`Mir6502Config::optimized()` and disabled by the conservative default.
[baseline.json](baseline.json) freezes eight controls at `82ada38f`;
[census.json](census.json) classifies the ten historical flag blockers.
[results.json](results.json) records final images, storage, masks, counts,
timings and qualification. Measurement date: 2026-11-11.

The earlier selector required all incoming C/N/Z/V to be unused when a tail
could not restore flags. The new query asks only about N/Z. It can replace
an ordinary X reload with an earlier TAX while C/V remain potential inputs.
The coarse admission path retains its existing policy and the new admissions
use an explicit per-flag proof. Stores, parameter homes, ABI layout and source
access order remain intact.

## Sizes and runtime behavior

Only the new flag changes within each pair. Generalized X forwarding and all
preceding optimized strategies stay enabled, with modern MIR6502, optimized
NIR and entry-reachable linking. Sizes include XEX framing.

| Program | Runtime | Coarse control | Precise entry proof | Saved |
| --- | --- | ---: | ---: | ---: |
| New neutral, either origin | cartridge | 69 | 67 | 2 |
| New neutral, either origin | standalone | 69 | 67 | 2 |
| TN | cartridge | 9,858 | 9,858 | 0 |
| TN | standalone | 9,983 | 9,983 | 0 |
| TNDBG | cartridge | 12,954 | 12,953 | 1 |
| TNDBG | standalone | 13,079 | 13,078 | 1 |

Neutral `Forward` shrinks 11→9 bytes. Its callee defines N/Z while masking a
BYTE argument, then calls a machine observer that captures status. Incoming
C/V remain observable and equal under both schedules. `UnsafeForward` passes
N/Z directly to that observer and keeps its reload. A sampled call with tag 1
and C/N/Z/V clear through the status-setting harness takes 91→89 cycles;
the unsafe control stays 79. Both runtimes use no embedded runtime for this
fixture, so their image sizes coincide. The neutral origins are `$3000` and `$41C7`; TN variants use `$2C00`.

TNDBG saves one byte in `Dbg`, at its call to `PrintHexC`. A known incoming
mask contains C/V and excludes N/Z. TN has no additional admission. `Strcat`
remains 113 bytes and its sample with `s=$7000`, `t=$7200`, lengths 3/8 stays
355 cycles in both variants/runtimes. All eight measured string images retain
counted-string results, overlapping-copy behavior and parameter-home overwrite
semantics. The required post-call reconstruction remains in place.

Six of 32 runtime/origin pairs shrink and 26 remain byte-identical. Six
additional applications save ten bytes across those images: four neutral
absolute reloads save two bytes each, and two TNDBG zero-page reloads save one
each. No corpus image grows. All eight frozen feature-off images match, as do
the previous corpus's 28 feature-on images against the new coarse controls.
Frames, storage capacities, reservation lengths and occupied zero page agree.
Code-relative data can relocate with shorter code. Symbol totals can count
aliases; the zero-page census excludes implicit runtime-only helper scratch.

## Proof and applicability

A known entry summary records a conservative mask of flags read before a
certified definition on any reachable path. Unknown differs from a known
empty mask. Joins union input requirements and loops use shared CFG liveness.
May-clobbers cannot discharge incoming dependencies; ADC/SBC/ROR retain their
read-before-write carry inputs. Register self-moves emit no definition, and uncertified address/compare/shift
forms retain dependencies. The entry-proof mode leaves unrelated liveness
transfer policies unchanged.

Each nested call/helper/barrier observes all flags conservatively. A prior
N/Z definition can discharge N/Z without discharging C/V. Scalar signatures,
effect declarations and machine-block eligibility retain their gates. Explicit
flag inputs/preservation and opaque call contracts still reject, and mutable
routines do not publish new incidental output flag values.

| Historical caller set | Flag blockers | Newly admitted | Remaining reason |
| --- | ---: | ---: | --- |
| TN `PopUp`, `Inv`, `Copy` | 3 | 0 | Unavailable entry facts |
| TNDBG `Dbg` | 1 | 1 | N/Z independently unused in `PrintHexC` |
| TNDBG `PopUp`, `Path`, `Inv`, `DrawWinFrame`, two in `Copy` | 6 | 0 | Unavailable entry facts |

The nine remaining cases involve unsupported scalar/machine/prologue facts.
The separate setup-operation blockers in `Sort`/`Tag` remain. Other measured
argument, pointer, predicate, string-loop, resident-string, arithmetic, indexed
and graphics controls have no additional size change. The improvement is
local; it does not establish that unknown machine entries are flag-free.

Typed witnesses carry the target ID, requested mask and immutable callee body.
A different body with the same mask invalidates an old witness. Final replay
uses proposed callee bodies and preserves consumed input bits as well as
published outputs. Tests cover simultaneous caller/callee rewrites and reject
weakened input contracts. Both original N/Z obligations, physical aliases,
X lifetime and the 64-operation/tail limits remain in force.

The refinement shares the existing selector/costing batch. An accepted batch
uses seven top-level emissions; a wholly rejected shaped batch uses one map,
and absent candidates use none. No full materializations or per-site emission
trials are added. The neutral coarse setting needs one map and its accepted
precise setting uses seven emissions; TN/TNDBG already have accepted coarse
batches. Entry summaries are computed once per distinct candidate target in
each supplied snapshot, with fresh queries for proposed-body replay and final
input validation. Test-only counters in `results.json` record actual entry
queries and CFG evaluations, excluding report-only mask inspection. New queries/CFG
visits are 5/5 for neutral cartridge, 6/0 for TN and 11/3 for TNDBG. Unknown
eligibility cases return before CFG analysis. Materialization samples are
10.6→8.3 ms, 8.34→8.36 s and 24.42→24.28 s respectively. These single
release-profile samples under concurrent qualification load do not establish
a compiler-speed change.

## Qualification and reproduction

Eleven focused tests cover all observed/requested masks, definite versus possible
writes, separate N/Z reads, joins, loops, unreachable paths, contracts and
stale body/target/mask rejection. VM execution covers 22,144 calls, including
all 256 forwarded values and all 16 input flag combinations, with both
runtimes/origins, raw/optimized NIR and actual LF/CRLF parsing. A further 64
calls cover all eight preceding-feature combinations. Returned A/X/Y/status/SP
and normalized retained parameter/output write traces agree. The unsafe
wrapper remains a negative control. Incoming N/Z can differ at the positive
callee entry; exact state is checked after its retained definition restores it.

The exact TN/TNDBG images pass 80 string and 16 post-call parameter-alias cases.
Fresh default compilation matches the measured images and passes full
panel/dispatch execution. Qualification passes 1,228 MIR6502 unit tests,
106 integration tests in 19 targets, 169 emission/load fixtures and 35 tests
in 11 affected VM targets, with two TN dispatch tests separately. Eight
LF/CRLF MADS reassemblies and relocations pass, together with scoped arithmetic/
indexed/indirect-field execution and selection checks and frozen static/VM
Strcat audits.
Exact target counts and hashes are in `results.json`. No shared NIR or semantic
contract changed. The existing indexed-byte selection-count discrepancy
remains separate from its passing runtime assertion.

The exporter writes assembly, XEX, MIR, maps and complete facts to ignored
`build/callee-entry-flags/measured/`:

```sh
CARGO_INCREMENTAL=0 cargo test --release --locked --lib callee_entry_flags_measure_corpus -- --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test callee_entry_flags_measurements -- --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test --locked --test mir6502_callee_entry_flags -- --ignored --nocapture
```

MADS must be on PATH or selected by `ACTIONC_MADS`. Set
`ACTIONC_CALLEE_ENTRY_FLAGS_MEASURED_IMAGES` to the measured directory's absolute
path for fresh `callee_entry_flags` and `tn_dispatch` image equality checks.
`ACTIONC_CALLEE_ENTRY_FLAGS_BASELINE` selects frozen neutral controls.
`ACTIONC_CALLEE_ENTRY_FLAGS_METRICS=1` reports sample neutral cycles. Historical
argument-X exporters and helpers explicitly keep the refinement off; their
artifacts and tests remain reproducible. The plan lists the full backend
qualification commands.
