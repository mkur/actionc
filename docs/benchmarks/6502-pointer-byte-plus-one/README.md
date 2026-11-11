# Pointer byte increment fusion: measured results

Measured 2026-11-11 following the
[four-slice plan](../../MIR6502_POINTER_BYTE_CONSTANT_ARITHMETIC_PLAN.md).
[baseline.json](baseline.json) freezes the preceding compiler and eight image
controls; [results.json](results.json) records the new source/image hashes,
routine/image sizes, storage, site counts, timings and execution observations.
All eight feature-off controls reproduce their frozen hashes exactly.

The optimized configuration now enables `enable_pointer_byte_plus_one_fusion`.
The conservative/default configuration keeps it disabled. Selection recognizes
two adjacent, materialized operations computing `p + BYTE[p+offset] + 1` and
replaces their 26-byte implementation with a typed 21-byte target operation.
It requires initialized, disjoint fixed zero-page pairs, an immediate byte Y
offset and dead A/X/arithmetic flags. The shared rewrite driver validates the
ordered entry-value read and removed scratch definitions. Exact final costing
can restore individual routines or the complete baseline.

SemIR, NIR, calls, parameter homes and argument captures retain their existing
contracts. The selector also preserves published callee exit facts used by
preceding optimizations. See the
[pseudo-machine contract](../../MIR6502_PSEUDO_MACHINE_CONTRACT.md#pointer-byte-increment-fusion).

## Sizes and execution

Only the new feature flag changes within each pair. Both settings use modern
MIR6502, optimized NIR and entry-reachable linking. Sizes include XEX framing;
neutral origins are `$3000` and `$41C7`, and TN/TNDBG use `$2C00`.

| Program | Runtime | Before | After | Saved |
| --- | --- | ---: | ---: | ---: |
| Neutral, either origin | cartridge | 668 | 663 | 5 |
| Neutral, either origin | standalone | 750 | 745 | 5 |
| TN | cartridge | 9,873 | 9,868 | 5 |
| TN | standalone | 9,998 | 9,993 | 5 |
| TNDBG | cartridge | 12,969 | 12,964 | 5 |
| TNDBG | standalone | 13,094 | 13,089 | 5 |

Neutral `Forward` shrinks from 95 to 90 bytes. A sampled call with `p=$7000`,
loaded byte 1 and `q=$7200` takes 356→350 cycles, including the wrapper,
argument-observing callee and return. Both runtimes and origins agree.
The `+0`, `+2`, subtraction and plain-arithmetic controls retain their sizes
and cycles. The ordinary result-producing `Address` routine remains 33 bytes:
its lowering does not expose this adjacent producer pair.

TN and TNDBG each select one site in `Strcat`, reducing its body from 125 to
120 bytes. A call with `s=$7000`, `t=$7200` and lengths 3/8 takes 370→364
cycles in either runtime. The 20-byte post-call pointer reconstruction and
outgoing argument staging remain. Both parameter-home alias counterexamples
still execute correctly. Relative to the historical 51-byte C control, the
remaining whole-body difference is 69 bytes. The
[frozen 125-byte audit](../6502-argument-staging/STRCAT_BYTE_BREAKDOWN.md)
continues to describe its original images.

The source-pair read trace remains `$AE,$AE,$AF,$AF`, with each indirect source
byte observed once in its original order. At callee entry the neutral VM
checks identical arguments, flags, stack depth and outgoing ABI write traces.
The fused schedule removes one destination-low scratch read and one write;
high-byte scratch access counts are unchanged. Frames, storage capacities
and declared/allocated/mapped zero-page bytes are equal. Code-relative data
and reserved ranges may move down five bytes with shorter code.

## Applicability and compiler cost

| Input | Exact adjacent pairs | Applied | Whole-image saving |
| --- | ---: | ---: | ---: |
| Neutral, either origin | 1 | 1 | 5 |
| TN | 1 | 1 | 5 |
| TNDBG | 1 | 1 | 5 |
| Existing word arithmetic | 0 | 0 | 0 |
| Indexed byte arguments | 0 | 0 | 0 |
| Scalar/predicate captures | 0 | 0 | 0 |
| Predicate snapshots | 0 | 0 | 0 |
| Array-parameter string loop | 0 | 0 | 0 |
| Graphics (`fedora`) | 0 | 0 | 0 |
| Resident string calls | 0 | 0 | 0 |

Counts apply separately to each runtime. Across 22 runtime/origin pairs,
eight images save five bytes each and the other 14 are byte-identical.
The latter have no exact adjacent producer pair; their zero counts do not
represent alias or profitability failures. Subtraction, other constants,
nonadjacent operations, scaled-Y consumers, wrapping/overlapping pairs,
missing incoming definitions and live A/X/flags remain on existing paths.
Focused tests also reject stale proofs and changes to published callee exits.

The selector has no callee-signature requirement and can serve any producer
of this MIR pair. Observed savings are confined to the neutral call and
`Strcat`; this corpus establishes narrow applicability. There is no new
scheduling window or pointer retention across calls.

A cheap shape scan avoids analysis and costing when no pair exists. Each
accepted transaction removes one operation and rebuilds its snapshot; the
initial operation count bounds iterations. An accepted batch uses at most
five extra emissions for routine/home and final XEX costing, independent of
site count, and no additional whole-program materialization. Literal fixed
pairs permit physical proof replay without another emission map.

Timing samples include normal materialization/reporting under concurrent
qualification load. Neutral cartridge at `$3000` takes 67.8→71.4 ms, TN
cartridge 8.26→8.37 s and TNDBG cartridge 51.84→25.39 s. These single noisy
observations do not establish a compiler-speed change. The zero-page census
excludes implicit runtime-only helper scratch; frame and image checks provide
separate storage guards.

## Validation and reproduction

Fourteen focused proof/emission/selection tests pass. The arithmetic oracle
executes 131,684 emitted fragments, including every pointer-low/addend pair,
high-byte and word wrap, offsets 0/1/255, scratch/ABI/source aliases and repair
branches straddling a page boundary. The neutral VM runs 1,920 calls across
raw/optimized NIR, both runtimes/origins and actual LF/CRLF source parsing,
plus 104 entry-alias calls. Measured TN images execute 80 string cases,
including same-buffer and overlapping copies, plus 16 parameter-alias calls.
Source lengths are positive; existing zero-length `MovePage` assembly behavior
is outside that counted-string oracle.

Final qualification passes 1,196 MIR6502 unit tests, 103 integration tests
across 16 targets and all 169 emission/load fixtures. MADS reconstructs and
relocates the new listing byte for byte in eight LF/CRLF/runtime/origin
combinations. Seven existing affected VM targets pass 25 tests; the new VM,
measured TN, TN panel/dispatch and frozen audit targets pass separately.
Three focused runtime-library assertions and the direct-word/indirect-field
selection assertions (six and two) pass. The broad fixture-script Cargo
wrappers exhausted space with generated debug artifacts; equivalent scoped
invocations completed after package cleanup. The previously documented
indexed-byte selection-count discrepancy remains, while its execution
assertion passes. No shared NIR or semantic contract changed.

The opt-in measurement writes images, MIR, entry/parameter tables and full
frame/storage facts under ignored `build/pointer-byte-plus-one/measured/`:

```sh
cargo test --release --locked --lib pointer_byte_plus_one_measure_corpus -- --ignored --nocapture
```

The exporter and baseline-sensitive helpers pin later pointer load sharing and
argument X forwarding off. Set `ACTIONC_POINTER_MEASURED_IMAGES` to that
directory's absolute path when running `pointer_byte_plus_one` to require exact agreement with
these historical neutral images. Current default TN execution uses the
[argument X forwarding measurements](../6502-argument-x-forwarding/README.md)
and `ACTIONC_ARGUMENT_X_MEASURED_IMAGES` for `tn_dispatch`. The measured TN
oracle below consumes the historical files directly and resolves parameter
homes from their owner-qualified maps:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test pointer_byte_plus_one_measurements -- --ignored --nocapture
cargo test --locked --test mir6502_pointer_byte_plus_one -- --ignored --nocapture
```

The last command requires MADS on PATH or `ACTIONC_MADS`. Historical comparison
and argument-staging exporters explicitly disable this new feature, preserving
their frozen controls and byte-role tables. The plan lists the full scoped
qualification commands.
