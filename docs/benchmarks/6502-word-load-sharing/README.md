# MIR6502 word load sharing measurements

The frozen baseline is compiler revision `7f7778f6`, with source, dependency,
compiler executable and image hashes in [baseline.json](baseline.json).
The [implementation plan](../../MIR6502_WORD_LOAD_SHARING_IMPLEMENTATION_PLAN.md)
specifies four slices and an 18-byte ALLOCATE target.

The retaining client is
[`allocate_word_load_sharing.act`](../../../fixtures/runtime/allocate_word_load_sharing.act).
It keeps all four routines of the maintained library reachable. VM probes
initialize a valid sentinel and heap before entering each routine directly;
the inspection client's entry is not the execution oracle.

| Cartridge baseline at `$3000` | Bytes |
| --- | ---: |
| `Alloc` | 287 |
| `Free` | 358 |
| `AllocInit` | 96 |
| `PrintFreeList` | 112 |
| Complete retaining client XEX | 952 |

The neutral fixture is 313 bytes in both runtimes at `$3000` and `$41C7`.
Baseline images, listings, materialized MIR and maps are retained under ignored
`build/word-load-sharing/before/`. The future feature-off compiler must
reproduce these images; the committed manifest preserves their identities.

The original hand schedule removes two ordinary absolute loads at each of
three sites, predicting 934 XEX bytes and eight cycles saved per affected call.
Final compiler selection must establish the actual saving with physical replay,
unchanged machine/memory behavior and the same capacity/reservation facts.
The separate comparison-block layout and the source library's coalescing
behavior remain outside this optimization.

Slice 1 validation: five typed-proof unit tests passed. The neutral and
retaining clients reproduce all six frozen XEX files byte-for-byte; their
frame, storage and reserved-range facts are also hashed in the manifest.
The existing pointer-load-sharing integration matrix remains green.

Slice 2 opt-in selection passes nine proof/transaction unit tests and the
frozen-image integration matrix. It also passes the existing pointer-sharing,
argument-X and callee-entry-flag matrices. At that stage both configuration
defaults remained disabled pending VM/corpus qualification.

The general local-source rule finds an additional six-byte copy in `Free`'s
traversal loop. Four sites therefore save 24 bytes: `Alloc` 287→281, `Free`
358→346, `AllocInit` 96→90, and complete cartridge/standalone images
952→928 / 1,427→1,403. The neutral images shrink 313→301 at both origins and
in both runtimes. Frames, globals, reservations, block order and terminators
are unchanged. The accepted batch uses seven additional emissions.

Slice 3 qualification passes 13 proof/transaction tests, including all 65,536
word values under four incoming status patterns, and six VM tests. The VM
checks 4,704 exact emitted fragments across storage classes, raw/optimized NIR,
both runtimes/origins, and actual LF/CRLF files. Partial aliases use an
independent sequential-byte oracle; volatile/absolute reads retain fallback.
Later indirect reads may alias the completed copies. Real allocator scenarios
retain the existing two-sided-coalescing behavior. Existing pointer, home-value,
word-store, argument-X, entry-flag and checked-pointer VM targets also pass.

The cycle probe now stops after the wrapper's final JMP. Earlier analysis
executed a seven-cycle BRK at the destination; subtracting only the 20-cycle
wrapper overstated absolute times by seven. Savings are unchanged. Corrected
cartridge routine measurements (return included, wrapper excluded) are:

| Case | Disabled | Enabled | Saved cycles |
| --- | ---: | ---: | ---: |
| Empty Alloc | 95 | 87 | 8 |
| Exact Alloc | 241 | 233 | 8 |
| Split Alloc | 318 | 310 | 8 |
| Boundary/skip Alloc | 407 | 399 | 8 |
| Insert Free | 407 | 391 | 16 |
| Left-merge Free | 345 | 329 | 16 |
| Right-merge Free | 474 | 466 | 8 |
| Two-neighbor Free | 521 | 505 | 16 |
| AllocInit | 163 | 155 | 8 |

The extra eight-cycle saving in traversing Free cases comes from the fourth
match in its loop body, and repeats for each visited node.

## Qualified applicability

The optimized configuration enables word sharing; `Mir6502Config::default()`
keeps it disabled. The master `enable_peepholes` flag also disables selection.
The complete corpus has ten shrinking cases and eighteen unchanged controls,
with no routine or image growth. Only this feature differs between variants.

| Source | Cartridge XEX | Standalone XEX | Selected windows per image |
| --- | ---: | ---: | ---: |
| Neutral, either origin | 313→301 | 313→301 | 2 |
| Parameter/local source fixture | 513→501 | 513→501 | 2 |
| ALLOCATE retaining client | 952→928 | 1,427→1,403 | 4 |
| TN | 10,290→10,290 | 10,415→10,415 | 0 |
| TNDBG | 13,385→13,373 | 13,510→13,498 | 2 |

TN's seven windows reject: five fail supported-storage classification and two
fail the footprint proof. TNDBG admits two additional ordinary global words:
`dbgPathName` in `Path` and `dbgFrameName` in `DrawWinFrame`, each copied to
`s` and `$AC/$AD`. These save six instruction bytes and eight cycles apiece.
Its ten uninitialized buffer reservations relocate twelve bytes earlier in
each runtime; their capacities and zero-page allocations stay equal, and the
combined proposal passes final-map replay.

The eight remaining sources have no exact candidate and use no selector
emissions: pointer-load-sharing, word-move-store, word-argument arithmetic,
indexed-byte arguments, captured predicate arguments, the string-length loop,
Fedora graphics and the runtime-linked strings fixture. The source-class
fixture also retains one blocked absolute-source candidate, while its volatile
source preserves all ordered reads. The committed scorecard gives every
before/after routine and image measurement, including unchanged controls.

## Corpus reproduction

Run the normal linked compiler with only `enable_word_load_sharing` toggled:

```sh
python3 tools/compare6502/word_load_sharing.py --check --mads
```

The exporter performs one outer materialization per variant and compares 28
source/runtime/origin cases (56 images). It checks PreEmission verification,
per-routine and whole-image size, exact source-load reductions, equal frames
and reservation capacities, and equal block order, branch conditions and CFG
edges. Uninitialized buffers following code may relocate as it shrinks; the
scorecard records their before/after addresses, and the selector replays its
proofs at final placement. Zero-page allocations stay equal.
Fused-compare indices may rebase when earlier loads disappear. Some baseline
indices already retain earlier-lowering provenance, so the corpus gate checks
bounded rebasing and equal branch flags/edges rather than dereferencing every
historical index. Emission consumes the flag test; the exact-region proof
requires equal exit flags. Tightening those existing producer indices is a
separate MIR hardening task.
The compact [current scorecard](current.json) records image hashes, changed
routines, storage-fact hashes, selection/blocker counters and selector emissions.
Historical baseline and opt-in scorecards remain unchanged.

Bulk MIR, MADS listings, XEX files, the compiler report and per-variant wall
times stay under ignored `build/word-load-sharing/measured/`. Wall times are
observations rather than deterministic acceptance thresholds. An admitted batch
uses at most seven additional emissions; an entirely blocked batch uses one
physical-map emission, and a source with no exact window uses none. No
candidate causes another outer materialization.

`--mads` independently reconstructs all 56 images with LF and CRLF assembly,
requiring 112 byte-identical roundtrips. The integration test also relocates
the neutral listing from `$3000` to `$41C7`. To repeat assembly or comparison
without recompiling, add `--reuse` after producing artifacts for the current
compiler. To intentionally refresh the scorecard, omit `--check` and review
every changed row.

Execute the exact measured neutral, retaining-client and TN cartridge images
through their existing independent VM oracles:

```sh
ACTIONC_WORD_LOAD_MEASURED_IMAGES="$PWD/build/word-load-sharing/measured" \
  cargo test --release --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test word_load_sharing --test tn_dispatch
```

The word-sharing VM target checks image identity before running the complete
neutral program and controlled real-library allocation/merge scenarios. TN
dispatch checks identity before its command, panel, directory, copy and menu
oracles. The checked-arena and real Alloc image controls separately cover
allocator boundaries and failure behavior.

## Final backend checks

The enabled default passes 1,279 MIR6502 unit tests and 838 emission/codegen
unit tests. All 29 selected root integration targets pass (128 tests), followed
by all nine opt-in MADS tests. These include the frozen feature-off images,
frame/storage facts, configuration and master-switch checks, NIR/MIR fixture
consumers, ABI, checked-arena controls, listing reconstruction and relocation.

The full Atari VM package passes 458 tests with the enabled default; eleven
existing artifact-dependent measurement tests remain ignored. The run supplies
`ACTIONC_WORD_LOAD_MEASURED_IMAGES` and `ACTIONC_HOME_VALUE_TN_IMAGES`, so the
neutral, retaining-client and TN dispatch oracles check measured-image identity,
and the Alloc oracle exercises all four measured TN runtime/program images.
Real arena checkpoints parse actual LF/CRLF sources in both supported build
modes and runtimes. All six word-sharing VM tests pass, including 4,704 emitted
fragment executions, 448 partial-alias executions and the controlled
allocation/merge cycle cases.

Root checks reuse `build/checked-arena/shared-target` with incremental builds
disabled, dev/test debug information disabled and optimization level 1. VM
checks reuse `tools/vm-runtime-tests/target/release`. No shared frontend or NIR
contract changes are part of this optimization.
