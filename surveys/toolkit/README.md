# ACTION! Toolkit Survey

This directory tracks current `actionc` support for the official ACTION!
Toolkit sources extracted under `corpora/toolkit/original/extracted`.

Run:

```sh
surveys/toolkit/survey.sh
```

The script builds `actionc`, checks every Toolkit `.ACT`, `.DEM`, `.DM1`, and
`.DM2` file through tokenization, parse + semantic analysis, and code
generation, then writes `TOOLKIT_SURVEY.md`. Each source is checked with the
supported profile/backend combinations: `legacy` + `classic`, `modern` +
`classic`, and `modern` + `mir6502`.

The report is the compatibility baseline for the extracted original sources.
Some original files still rely on old Toolkit idioms that are tracked as
blocked in `TOOLKIT_SURVEY.md`; modernized copies for those cases live under
`samples/toolkit/modern`. Use the report and comparison notes to track
regressions across the classic and MIR6502 paths.

Batch object compilation uses:

```sh
surveys/toolkit/compile-toolkit-batch.sh --preset all
```

The batch compiles demo/program files where a Toolkit library has demos, and
uses a generated `EndProg` harness for `ALLOCATE.ACT` when the selected source
does not already declare it. Presets are available for `legacy-classic`,
`modern-classic`, and `modern-mir6502`. The older `compat-legacy` and
`modern-legacy` preset names remain accepted as aliases.

`legacy-classic` normally compiles the extracted original sources, but entries
marked as modernized use the maintained copy under `samples/toolkit/modern`
even there.
`MUSIC.DEM` is one such case: the original source depends on implicit PMG
pointer and address-byte idioms that `actionc` intentionally requires to be
spelled explicitly.

Four demo entries are expected to be rejected when `legacy-classic` validates
the extracted originals: `KALSCOPE`, `PMGDM1`, `PMGDM2`, and `PRINTF1`. Those
sources rely on loose pointer conversions that actionc deliberately rejects.
After verifying each diagnostic, the preset compiles its maintained replacement
with the legacy profile and classic backend. This keeps the legacy ATR complete
without weakening validation of the originals. An unexpected acceptance, an
undocumented rejection, or a replacement that fails under legacy classic fails
the gate. The modern presets compile the same replacements under their normal
profiles and backends.

The current object-size comparison across the original compiler,
`legacy-classic`, `modern-classic`, and `modern-mir6502` is tracked in
`TOOLKIT_SIZE_ANALYSIS.md`.

Toolkit object ATRs are generated with:

```sh
surveys/toolkit/pack-toolkit-atrs.sh
```

The packer rebuilds the three batch presets, then writes one MYDOS ATR per
compiler setting under `outputs/atr`, with every successfully compiled `.COM`
object from that setting packed onto the same disk. Runtime dependencies are
packed alongside their programs: when `MUSICDEM.COM` is present, the byte-exact
3,600-byte `corpora/toolkit/original/extracted/MUSIC.SCR` screen image is added
as `MUSIC.SCR`.

For WARP-specific runtime/control-flow notes, especially the main loop and its
absolute-address timer/collision aliases, see `WARP_MAIN_LOOP.md`.

For SemIR-native coverage under the modern profile, see
`SEMIR_NATIVE_MODERN.md`. That note tracks modern-specific source copies and
the remaining comparison work exposed by the Toolkit sweep.

Original-compiler object captures for Toolkit demo/program files are generated
with:

```sh
surveys/toolkit/capture-original-demos.sh
```

The script compiles the raw `*.atascii` sidecars through the original Action!
compiler VM, prefixes temporary sources with `$3000` code-origin setup, and
writes `.COM`, symbol JSON, load-info, and compare reports under `outputs/vm`.
Sources can still override that setup themselves; `KALSCOPE.DEM` deliberately
resets the original compiler origin to `$5000`.
