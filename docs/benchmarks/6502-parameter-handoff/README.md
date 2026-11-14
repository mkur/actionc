# Parameter-to-argument handoff measurements

The [implementation plan](../../MIR6502_PARAMETER_ARGUMENT_HANDOFF_IMPLEMENTATION_PLAN.md)
starts from compiler `471ef1c8`, with qualified leaf parameter placement enabled.
[baseline.json](baseline.json) freezes twelve builds: TN and independent wrapper
controls with inlining disabled/enabled, both runtimes, origins `$3000`/`$41C7`.
All four TN image hashes and compiler work counts exactly match the prior
[leaf qualification](../6502-parameter-homes/qualification-on.json).

| Subject | Code bytes | Ordinary parameter bytes | Bound ZP bytes |
| --- | ---: | ---: | ---: |
| TN Fnamecmp | 105 | 0 | 4 |
| TN Strcpy | 41 | 4 | 0 |
| TN Strcat | 113 | 4 | 0 |
| Independent ForwardWords | 26 | 4 | 0 |
| Independent CopyCounted | 41 | 4 | 0 |

TN images are 9,878 bytes (cart) and 9,985 (standalone); wrapper controls are
572 bytes in all configurations. Every measured traced output equals its full
untraced artifact. The receipt records source/library/image hashes, configuration,
routine sizes, upstream selections and downstream work counts. The single
sequential probe took 108.30 seconds, maximum RSS 321,830,912 bytes; it includes
traced and untraced compilation and excludes rebuilding the compiler/probe.
This is provenance, not a statistical performance claim.

Reproduce after sourcing `build/mir6502-layout-census/build-env.sh` and building
the current library:

```sh
python3 tools/compare6502/parameter_homes.py \
  --api-rlib PATH/TO/libactionc-HASH.rlib --enabled --suite handoff \
  --output build/mir6502-parameter-handoff/baseline
cargo test --test mir6502_parameter_argument_handoff
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test parameter_argument_handoff
```

Baseline checks: two root tests and one VM test pass. VM coverage uses an
independent canonical-argument/ordered-memory oracle, both runtimes/origins,
actual LF/CRLF loading, all 256 length bytes, page boundaries, identical and
partially overlapping buffers, and stack balance. Additional controls cover
byte widening, parameter mutation/exposure, extra calls, post-call uses, source
ZP ownership and an extra callee entry read. Source offsets differ across
line endings; executable bytes, relocations and storage placement do not.

Slice 1's [gate-off receipt](slice1-off.json) matches all twelve baseline image
hashes, routine sizes, compiler work counts and upstream selections exactly.
`--handoff` independently enables the new extension in the measurement harness;
`--enabled` continues to control the existing leaf placement feature.

Slice 2's [gate-off receipt](slice2-off.json) again matches all baseline image
hashes, routine sizes, upstream selections and compiler work counts. Forced
mixed bindings are covered separately by execution tests: `[ZP($A0), X]` emits
one real memory fragment, retains the high byte in X, and returns 256 distinct
words correctly across both runtimes/origins and LF/CRLF input. This is a
representation test; automatic wrapper placement is still disabled.

Slice 4's [opt-in receipt](slice4-on.json) selects direct wrappers: ForwardWords
is 11 code + 0 ordinary bytes and ForwardBytes 29 + 0, shrinking the control
image from 572 to 549 bytes in all eight control configurations. TN is unchanged
with identical hashes/work counts. No upstream search is replayed; productive
control cases add one core continuation. Indirect producers are still excluded.

Slice 5's [opt-in receipt](slice5-on.json) reaches compiler-generated Strcpy and
CopyCounted at **20 code + 0 ordinary bytes**, each with three resident ZP bytes.
TN becomes **9,853 cart / 9,960 standalone**, saving 25 bytes beyond the existing
leaf result; Fnamecmp remains 105, Strcat 113 + 4, Key 37 and Next 33 code bytes.
The original controls shrink from 572 to 524 bytes. The additional direct
controls retain slice 4's 180-byte image after restricting the resident consumer
to actual pointer-read uses. All sixteen traced/untraced pairs agree.

The receipt also hashes maps and relocations. TN adds one core continuation
(4 to 5), with 123 emission invocations, 738 inner probes and 18 preservation
replays in the resulting pipeline. Upstream selections and attempts are unchanged.
Slice 5 timing overlaps focused tests/builds and is not a performance comparison;
slice 6 measures the isolated off/on pair. The gate remains off by default here.

## Final qualification

Terminal handoff is enabled in `Mir6502Config::optimized()` and remains off in
the conservative configuration. It also requires the peephole master gate and
parameter-home selection. This policy prioritizes final executable size; the
independent handoff switch remains available to avoid its compilation cost.

| Subject | Leaf-qualified baseline | Handoff enabled |
| --- | ---: | ---: |
| TN cart image | 9,878 | 9,853 |
| TN standalone image | 9,985 | 9,960 |
| Strcpy code + ordinary homes | 41 + 4 | 20 + 0 |
| CopyCounted code + ordinary homes | 41 + 4 | 20 + 0 |
| Original control image, inlining off/on | 572 | 524 |
| Repeated/computed/permuted control image | 213 | 180 |

The [qualification record](qualification.json) contains exact commands, compiler
provenance, per-target outcomes and log hashes. It consolidates 1,381 passing
unit tests, 149 integration tests in 33 targets, 59 VM tests in 24 targets, and
11 explicit MADS checks. The first register-copy runs exposed an obsolete
incremental-saving assumption: handoff already removes that traffic. The strict
pass-only oracle remains, and rebuilt combined-mode checks cover values, guards,
size and cycles. No compiler correction followed the optimized-default change.

The [off](qualification-off.json) / [on](qualification-on.json) receipts record
sixteen full handoff builds each; the [leaf off](qualification-leaf-off.json) /
[leaf on](qualification-leaf-on.json) receipts add 24 independent cases each,
omitting the already-measured TN matrix. All 80 traced/untraced artifact pairs
agree. All 24 leaf image/map/relocation hashes, parameter fragments and work
counts are identical with handoff toggled. The twelve original baseline image
hashes are preserved with handoff off. Every comparison preserves upstream
search attempts and choices, with at most one additional core continuation.

TN retains Fnamecmp at 105 + 0, Strcat at 113 + 4, Key at 37 and Next at 33
code bytes. Strcpy's parameter fragments occupy `$A0`, `$A2`, `$A3`; X retains
the remaining byte. The callee still receives the canonical arguments, including
the produced fifth byte in `$A4`. Final-cost acceptance rejects routine growth
in code plus ordinary homes and requires a strictly smaller packaged image.

TN's downstream work increases from 83 to 123 emission invocations, 498 to 738
inner probes, and 12 to 18 preservation replays; core materializations rise from
4 to 5. The extra continuation is bounded but substantial. Unknown callee-entry
contracts, unsupported pointer reconstruction, permutation cycles and post-call
uses continue through the existing fallback.

Sequential off/on measurements ran after all builds and tests finished, using
the recorded opt-level-1, no-debug compiler build. These are single matrix runs,
including traced and untraced compilations, not statistical performance claims:

| Compile-cost measure | Handoff off | Handoff on |
| --- | ---: | ---: |
| Full 16-case probe elapsed | 99.61 s | 129.32 s |
| Median untraced TN compilation, four cases | 11.67 s | 15.39 s |
| Probe maximum RSS | 301.84 MiB | 345.05 MiB |

The observed TN compilation increase is about 32%; the whole probe takes about
30% longer. That cost is accepted for the optimized preset's size priority and
is not described as negligible. The leaf-only probes overlap final tests and are
used for artifact/work nonregression, not for this timing comparison.
