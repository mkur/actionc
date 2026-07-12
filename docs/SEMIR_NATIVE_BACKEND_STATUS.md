# SemIR Native Backend Status

Snapshot date: 2026-05-30.

SemIR-native is the active modern native backend path. It is past pure
bring-up: it has a typed SemIR input, a classifier, reusable materializers, a
tracked emission facade, source listings, map/proof hooks, and compile/run
script support. It is not yet a proven production replacement for the AST
backend. The current work is still semantic stabilization, especially under TN.

## Entry Points

- CLI: `--codegen-source semir-native`.
- Compile/run helper:
  `tools/compile-run-atr.sh --profile modern --codegen-source semir-native`.
- High-level lowering owner: `src/codegen/semir_native.rs`.
- Classification: `src/codegen/semir_native/native_classify.rs`.
- Materialization: `src/codegen/semir_native/native_materialize.rs`.
- Concrete emission helpers: `src/codegen/semir_native/native_emit.rs`.
- Tracked state/emitter facade:
  `src/codegen/native_state.rs` and `src/codegen/native_emitter.rs`.

## Layer State

Typing is stable for current backend needs. The semantic layer provides the
widths, pointer targets, array facts, record fields, call signatures, lvalue
legality, and condition facts that SemIR-native consumes. Old Action source
compatibility is not absolute; newly discovered original-source idioms should
be documented as compatibility work and placed in the owning layer.

Classification is at a healthy plateau. `NativeClassifier` is the read-only
shape facade for values, lvalues, address-like values, calls, byte/word
sources, and compare operands. More classification should be added only when a
new semantic shape appears, not as a shortcut around materialization.

Materialization is at a healthy plateau. It owns most reusable value-to-home,
slot-copy, return-slot, address-of, pointer/index/address, record-field,
indirect array-element, and call-argument staging. Call argument byte homes,
word-to-AX staging, and SARGS byte staging live in materialization. Broader
ABI-home vocabulary is parked until another repeated destination pattern
proves it is needed.

Emission is at a strong plateau. Ordinary instruction writing is routed through
`NativeTrackedEmitter` and concrete helpers in `native_emit.rs`. High-level
SemIR-native lowering is guarded against direct `self.emitter.emit_*` calls.
Future emission work should be targeted: missing helpers, tracked-state
corrections, or raw-data/label guardrails found by another layer.

## TAC Runway Policy

SemIR-native is now a correctness and observability runway for TAC, not the
long-term optimization home. Near-term SemIR-native work should be limited to:

- correctness fixes and missing backend shapes that make real programs compile;
- validation/reporting hooks that make backend behavior easier to compare;
- small layer cleanup that preserves typing, classification, materialization,
  and emission boundaries;
- semantic facts, probes, and proof hooks that TAC can reuse.

Defer broad code-size tuning, instruction scheduling, register allocation,
expression reshaping, and AST-byte mimicry to TAC unless a delta exposes a
semantic bug or a reusable structural hole. Toolkit size deltas should be
tracked as `tac-deferred` by default once the program compiles and no runtime
semantic issue is known.

The TAC/NIR runway hook is `actionc-emit --emit-nir <file.act>`. It lowers SemIR
into the transitional TAC-backed NIR surface for inspection. This output is
structural and observable only: it is not yet an optimizer, allocator, or
replacement backend. The old `--emit-tac` CLI alias has been removed.

## Current Validation

Fixture sweep as of this snapshot:

```sh
cargo run --quiet --bin actionc-semir-sweep -- --candidate native --dashboard fixtures/semir
```

Result:

- total fixtures: 32;
- supported by SemIR-native: 32;
- byte-identical with AST backend: 16;
- byte mismatches: 16;
- unsupported: 0;
- AST failures: 0;
- SemIR-native failures: 0.

This means the backend can lower the current fixture surface, but exact
fixture calibration is not complete. A mismatch is not automatically a semantic
bug, but exact fixtures should be reviewed and classified before treating them
as acceptable modern deltas.

Stress sweep planning is tracked in `SEMIR_NATIVE_STRESS_BACKLOG.md`. That
document maps the current stress failures to reusable SemIR-native backend
shapes and gives the recommended implementation order.

TN no-run build as of this snapshot:

```sh
tools/compile-run-atr.sh --profile modern --codegen-source semir-native --no-run \
  samples/tn/modern/TN.ACT corpora/tn/atr/tn-1.23-stryker.atr
```

Result: builds `TN.COM`, injects it into an ATR, and reports a 51-sector COM
file. Runtime validation remains active and must be done in the emulator.

Focused regression checks currently guarding recent TN blockers:

- `native_set_symbol_to_current_location_uses_deferred_storage_high_water`
  protects `SET symbol=*` after deferred array/string storage.
- `native_nonzero_bitwise_conditions_materialize_before_branch` protects
  conditions like `WHILE skstat&$04 DO`.
- `native_conditions_support_logical_or_and_and` protects real logical
  short-circuit `AND`/`OR`.

## Recent Semantic Lessons

`SET buffer=*` must use the post-layout high-water mark, including deferred
array and string backing storage. Otherwise runtime buffers can overlap program
storage even though the program compiles.

Bitwise expressions in conditions are not logical short-circuit expressions.
For example, `WHILE skstat&$04 DO` must materialize `skstat & $04` and test the
result, not branch on `skstat` before applying the mask.

TN is the main integration pressure source. A TN failure should usually become
a small semantic probe first, then a general backend fix. Avoid routine-specific
patches and avoid optimization work until the copy flow is semantically stable.

## Known Open Risks

- TN runtime behavior is not fully proven. The latest no-run build includes
  fixes for buffer high-water placement and bitwise nonzero conditions, but the
  emulator copy flow still needs confirmation.
- Fixture exactness is incomplete: 16 of 32 fixture programs currently differ
  from the AST backend byte output.
- State tracking has guardrails, but the dedicated state-tracker test suite is
  still thin relative to how much correctness now depends on tracked register,
  flag, and memory facts.
- Old Action source compatibility remains a separate compatibility stream,
  especially around raw pointer decay, routine addresses, `SET *=...` style
  layout, machine blocks, and resident-library idioms.

## Recommended Next Focus

1. Keep SemIR-native sweeps compiling and free of `SEMFAIL` regressions under
   coverage/mixed validation.
2. Emit markdown sweep reports for stress, toolkit, and focused probe runs so
   backend state can be pasted into plans without manual reformatting.
3. Preserve the layer split when closing remaining shape gaps: typing facts
   first, then classification, then materialization, then emission helpers.
4. Start a minimal TAC-facing IR/output path from SemIR so future optimization
   work has a concrete landing zone.
5. Re-run TN copy in the emulator after backend correctness changes and capture
   the next exact stop point if runtime behavior regresses.
6. Keep SemIR-native optimization parked unless it removes a correctness risk,
   clarifies a reusable layer boundary, or creates data TAC can consume.
