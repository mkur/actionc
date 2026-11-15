# MIR6502 whole-image cost qualification

Qualified 2026-11-15. Compiler change: `65b223ce`; previous compiler baseline:
`8626beff`. Implements the [design refinement](../../MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-refinement-whole-image-cost-acceptance)
and [two-slice plan](../../MIR6502_WHOLE_IMAGE_COST_IMPLEMENTATION_PLAN.md).

Both final cost guards now accept a verified candidate when its complete packaged
image is strictly smaller. Individual routine growth is reported without vetoing
that saving. Routine-inventory compatibility, preservation, exact rollback and
all existing eligibility checks remain intact. Equal or larger images still fail.

## Independent acceptance change

The existing `storage_observation.act` control, ActionCart `$3000`, keeps scoped
observation explicitly off to retain Walk's extra staging store. Parameter-home,
handoff and cross-call placement remain enabled. The old policy rejected the
229-byte trial because Walk grew; the new policy accepts it against the 261-byte
selected fallback: **32 bytes saved**.

| Routine | Before: code + ordinary homes | After: code + ordinary homes | ZP homes before → after |
| --- | ---: | ---: | ---: |
| FindRecord | 80 + 5 | 51 + 0 | 0 → 5 |
| Walk | 116 + 4 | 118 + 4 | 0 → 0 |

These are the only changed routines in the complete five-routine inventory.
FindRecord saves 34 code/storage bytes; Walk's two-byte increase leaves a net
32-byte saving. Its reported cost grows from 120 to 122 because that cost includes
four ordinary-home bytes. The selected image is 229 bytes with small-leaf inlining
either disabled or enabled. Both probe cases match their untraced artifacts.

The integration regression also compares cross-call placement off/on in the new
compiler, checks actual Walk code growth, resident FindRecord homes, the accepted
size/delta diagnostics and traced/untraced equality. This is an isolation control;
normal optimized builds retain scoped observation.

## TN outcome and compiler work

Normal optimized TN, ActionCart `$3000`, remains **9,791 bytes**. Image, complete
map and relocations match the retained baseline exactly. All 101 routine code and
ordinary/ZP-home footprints are unchanged; there are no growing routines.

| Work | Independent control, before / after | TN, before / after |
| --- | ---: | ---: |
| Core materializations | 2 / 2 | 6 / 6 |
| Emission invocations | 28 / 28 | 163 / 163 |
| Inner emission probes | 112 / 112 | 978 / 978 |
| Preservation replays | 2 / 2 | 24 / 24 |
| Rewrite analyses | 864 / 864 | 50,800 / 50,800 |

No comparable work counter changes. The historical control receipt predates the
`source_storage_inventory_builds` counter added in `8626beff`; the current value
of 16 describes previously uncounted work, not 16 newly introduced constructions.
TN already recorded that counter and remains at 60.

The one TN probe observed 23.87 seconds traced, 21.02 seconds untraced and a peak
RSS of 700,071,936 bytes for the process retaining both outputs and tracing.
The VM target was building concurrently. These are observations, not evidence of
a performance regression or improvement; no repeated timing benchmark was run.

## Baseline and receipts

Normalized source and compiler-configuration hashes match the retained references.
Old image/map/relocation/config artifacts were verified against their receipts;
the cached TN compiler library matched its historical hash before rebuilding.
The current TN probe source is identical to the historical one.

The older control probe matches `e97dd17c` exactly. Its only differences from the
current probe are a larger diagnostic trace limit and an assertion message;
measurement/configuration logic is unchanged. The compiler changes between the
historical control and `8626beff` were enabling scoped observation by default
(explicitly off in this control) and instrumenting the existing inventory count.
No old compiler was rebuilt, and historical receipts remain unchanged.

- [Baseline verification](baseline-verification.json)
- [Control measurements](control.json)
- [TN measurements](tn.json)
- [Complete routine/artifact/work comparison](comparison.json)

Large images, maps, listings and logs remain under ignored
`build/whole-image-cost/`. Receipts retain source, configuration, compiler-library,
probe and artifact hashes plus selected decisions and work counts.

## Minimal validation

- Three filtered library cost tests: the two revised boundaries and the existing
  pointer-retention control where data cancels an instruction saving.
- The existing exact selected-artifact rollback and callee-exit-contract tests.
- One new integration regression for accepted net savings with real local growth.
- The existing storage-observation VM test: five scenarios covering repeated
  invocations, canonical arguments, results, stack balance, page crossing, byte
  wrap, source-memory sentinels, actual LF/CRLF loading, a standalone smoke case
  and the protected ZP window. The scoped-observation-off arm now executes the
  formerly rejected placement.

All seven selected tests passed. [Slice 1 checks](slice1-checks.json),
[slice 2 checks](slice2-checks.json). Tests used the shared target, opt-level 1,
debug information disabled, incremental off and two build jobs. The full backend,
NIR sweep, MADS, raw-NIR/origin matrices and full TN runtime scenarios were not run
under the user's minimal-testing direction. No existing behavioral assertion was
weakened, and no runtime fixture changed.

## Reproduce the measurements

After building the library with the plan's shared-target/profile settings, use
the freshly built rlib (its filename may vary):

```sh
python3 tools/compare6502/parameter_homes.py \
  --api-rlib build/checked-arena/shared-target/debug/deps/libactionc-bf811c1ef8b2df31.rlib \
  --output build/whole-image-cost/control \
  --enabled --handoff --cross-call --suite storage-observation --skip-tn
python3 tools/compare6502/parameter_homes.py \
  --api-rlib build/checked-arena/shared-target/debug/deps/libactionc-bf811c1ef8b2df31.rlib \
  --output build/whole-image-cost/tn \
  --enabled --handoff --cross-call --scoped-observation \
  --suite storage-observation --tn-only
```

Multiple-leaf-call selection remains suspended pending a batching decision.
Earlier candidate-level per-routine filters remain a separate review.
