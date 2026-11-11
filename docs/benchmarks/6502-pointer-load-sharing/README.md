# Pointer load sharing measured results

Measured 2026-11-11 following the
[four-slice plan](../../MIR6502_POINTER_LOAD_SHARING_PLAN.md).
[baseline.json](baseline.json) freezes the preceding compiler and eight image
controls. [results.json](results.json) records source/image hashes, final
sizes, storage, site counts, timings and execution checks. All eight
feature-off controls match their frozen hashes exactly.

`Mir6502Config::optimized()` now enables `enable_pointer_load_sharing`.
Conservative/default configuration keeps it disabled. The selector replaces
an exact 11-operation window with nine existing MIR operations: store the
original pointer bytes beside their first loads during `q+1`, then reuse that
scratch pair for the following `q^`. Both capture stores and both pointer
stores remain. Two redundant absolute loads disappear, saving six bytes and
eight cycles at each admitted site.

The proof requires an ordinary direct parameter/local word, disjoint physical
source/capture/pointer ranges and the exact offset-zero indirect consumer.
Including that consumer restores A/N/Z after arithmetic; C/V, X/Y and stack
state also agree. The shared driver revalidates the typed witness against its
supplied physical map. Final costing replays the accepted changes against the
proposed layout and preserves published callee facts. SemIR, NIR, emission
operations, parameter homes and the public ABI keep their existing contracts.

## Sizes and execution

Only this feature changes within each pair. Both settings use modern MIR6502,
optimized NIR and entry-reachable linking. Sizes include XEX framing. The new
neutral fixture uses `$3000` and `$41C7`; TN/TNDBG use `$2C00`.

| Program | Runtime | Before | After | Saved |
| --- | --- | ---: | ---: | ---: |
| New neutral, either origin | cartridge | 716 | 710 | 6 |
| New neutral, either origin | standalone | 798 | 792 | 6 |
| TN | cartridge | 9,868 | 9,862 | 6 |
| TN | standalone | 9,993 | 9,987 | 6 |
| TNDBG | cartridge | 12,964 | 12,958 | 6 |
| TNDBG | standalone | 13,089 | 13,083 | 6 |
| Existing pointer-increment fixture, `$3000` | cartridge | 663 | 639 | 24 |
| Existing pointer-increment fixture, `$3000` | standalone | 745 | 721 | 24 |

New neutral `Forward` is 90→84 bytes. A sampled call with `p=$7000`, loaded
byte 1 and `q=$7200` takes 350→342 cycles, including the wrapper, real
argument-observing callee and return. Both runtimes and origins agree. Controls
with second expressions `q+0`, `q+2`, `q-1`, an indexed read and an existing
shorter argument path retain their sizes and cycles.

TN/TNDBG `Strcat` is 120→114 bytes. A call with `s=$7000`, `t=$7200` and
lengths 3/8 takes 364→356 cycles in both variants/runtimes. The two pre-call
source-home reads per lane become one; pointer stores and outgoing ABI writes
retain their values/counts. Each indirect byte is still read once in its
original order. The 20-byte post-call reconstruction remains and both
parameter-home overwrite probes still follow the updated pointers correctly.
The historical C control remains 51 bytes, leaving a 63-byte whole-body
difference with distinct ABI and alias contracts. That difference is not a
claim of removable bytes.

Frames, source-storage capacities, reserved lengths and occupied zero page
are unchanged. Code-relative data and reserved ranges may relocate with
shorter code. The zero-page census unions declared, allocated and mapped
cells; it excludes implicit runtime-only helper scratch.

## Applicability and compiler cost

| Input | Exact windows | Applied | Image saving |
| --- | ---: | ---: | ---: |
| New neutral, either origin | 1 | 1 | 6 |
| TN | 1 | 1 | 6 |
| TNDBG | 1 | 1 | 6 |
| Existing pointer-increment fixture | 4 | 4 | 24 |
| Direct word arithmetic | 0 | 0 | 0 |
| Indexed byte arguments | 0 | 0 | 0 |
| Scalar/predicate captures | 0 | 0 | 0 |
| Predicate snapshots | 0 | 0 | 0 |
| Array-parameter string loop | 0 | 0 | 0 |
| Graphics (`fedora`) | 0 | 0 | 0 |
| Resident string calls | 0 | 0 | 0 |

Counts apply separately to each runtime. Ten of 24 runtime/origin pairs
improve; the other 14 are byte-identical. There are 16 applications across
those images. All surviving exact-window counts fall to zero. Each application
removes two direct load operations and no stores. The existing pointer fixture
qualifies four different first-argument computations through the same local
second-argument setup.

Discovery has no callee-name, signature or preservation requirement. Its
observed applicability is limited to this contiguous arithmetic/setup family.
Other constants, source classes, consumer offsets and scaled/live-Y forms are
excluded by the early shape check. The measured corpus has no late proof/cost
rejections; focused tests cover unsafe physical aliases and missing facts.

A cheap shape scan skips analysis/maps for absent templates. Every transaction
removes two operations and rebuilds its snapshot, with iterations bounded by
the original operation count. An accepted batch uses seven additional
top-level emissions for two physical maps, three routine/home costs and two
XEX costs. There are no additional whole-program materializations or per-site
emission trials. Unknown costs, failed replay or weakened exit facts retain
the preceding accepted program.

Single release-profile materialization samples under concurrent qualification
load are 99.3→89.7 ms for neutral cartridge, 8.07→8.28 s for TN cartridge and
24.15→25.27 s for TNDBG cartridge. These observations do not establish a
compiler-speed change.

## Validation and reproduction

Ten focused tests pass. Their emitted-sequence oracle executes 13,104 cases
covering pointer-low values 0–255, high/word wrap, both incoming carry values,
six byte boundaries and source/capture/pointer/ABI aliases at two origins.
It checks A/X/Y/C/N/Z/V/SP against an independent host oracle. Three VM tests
execute 2,304 calls across raw/optimized NIR, both runtimes/origins and actual
LF/CRLF parsing, plus 104 entry-alias calls and eight independent-feature
composition calls. They check arguments, guards, read order/count, pointer/ABI
write traces and entry flags/stack. Both feature settings match the exact
measured neutral images; the default CLI also reproduces the measured image.

The exact TN images pass 80 string cases and 16 parameter-home alias calls,
including same-buffer and overlapping copies. Positive source lengths preserve
the counted-string oracle's scope; existing zero-length `MovePage` assembly
behavior is unchanged. Current default TN/TNDBG images match the measured
files and pass panel/dispatch execution.

Final backend checks pass 1,206 MIR6502 unit tests, 104 integration tests across
17 targets and 169 emission/load fixtures. Eight MADS assemblies reproduce
and relocate the new listing with both LF and CRLF text. Nine affected VM
targets pass 30 tests, with two TN dispatch tests separately, plus the
measured-image and frozen audit targets. Three focused arithmetic/indexed/
indirect-field runtime assertions and direct-word/indirect-field selection
assertions (six and two) pass. The previously documented indexed-byte count
discrepancy remains separate from its passing execution assertion. No shared
NIR or semantic contract changed.

The opt-in exporter writes images, MIR, entry/parameter maps and full frame/
storage facts under ignored `build/pointer-load-sharing/measured/`:

```sh
cargo test --release --locked --lib pointer_load_sharing_measure_corpus -- --ignored --nocapture
```

Set `ACTIONC_POINTER_LOAD_MEASURED_IMAGES` to that directory's absolute path
when running `pointer_load_sharing` and `tn_dispatch` to require equality
between freshly compiled and measured images. The measured TN oracle consumes
the images directly and resolves current owner-qualified parameter homes:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test pointer_load_sharing_measurements -- --ignored --nocapture
cargo test --locked --test mir6502_pointer_load_sharing -- --ignored --nocapture
```

The last command requires MADS on PATH or `ACTIONC_MADS`. Historical exporters
and baseline-sensitive helpers explicitly disable this later optimization,
preserving earlier manifests and byte-role tables. The plan lists the full
scoped qualification commands.
