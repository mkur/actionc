# SemIR-native Removal Plan

Status: implemented on 2026-08-20. SemIR-native was removed directly; the
separate deprecation change was not merged and no warning window was added.

## Decision

Delete the experimental direct SemIR-to-6502 backend and all ways to select it.
The maintained compiler paths remain:

```text
legacy source -> AST/classic
named or bridged source -> SemIR -> AST/classic
all MIR6502 source -> SemIR -> NIR -> MIR6502 -> tracked emission
```

This decision does not remove SemIR, the SemIR-to-classic bridge, NIR, MIR6502,
or standalone runtime selection. It removes only the parallel backend rooted at
`src/codegen/semir_native.rs`.

The historical `00efe36` SemIR-native deprecation commit is intentionally not
part of this work. Removed aliases should fail as invalid CLI values rather than
emit a transitional warning.

## Removed Footprint

The removed backend owned:

- `src/codegen/semir_native.rs` and the three files under
  `src/codegen/semir_native/`, about 11,300 implementation lines in total;
- 134 backend-local unit tests plus SemIR-native cases in codegen, compiler,
  CLI, inline-assembler, sweep, and listing tests;
- the public `generate_semir_native_profile_with_origin` entry point;
- `CodegenSource::SemIrNative` and the `semir-native`, `native`,
  `sem-ir-native`, `native-ir`, and `modern-ir` aliases;
- the native candidate, dashboard, unsupported classification, and
  coverage/mixed policy in `actionc-semir-sweep`;
- helper-script and MADS-listing matrix entries;
- four active SemIR-native architecture/status/validation/backlog documents
  and several current cross-references.

## Boundaries

- Keep the tracked emission layer used by MIR6502. Its retained emitter now
  lives in `src/codegen/tracked_emitter.rs`; `src/codegen/native_state.rs`
  remains a separate naming cleanup.
- Keep `src/codegen/semir.rs`. It is the SemIR-to-AST bridge used by classic
  code generation, including named-module paths.
- Keep `actionc-semir-sweep` as an exact AST-versus-SemIR-bridge fidelity tool.
- Do not move backend-specific classifiers or materializers into NIR or
  MIR6502. Reuse only tests that express an independently owned language, ABI,
  relocation, inline-assembler, or emission contract.
- Do not change the output of `CompileMode::Compatibility`,
  `CompileMode::Optimized`, or `CompileMode::Mir6502` as part of removal.
- The listing-cleanup PR (#13) is merged. Base the implementation on `main` at
  or after `f7fc198` so its `src/codegen.rs` and `tests/actionc_cli.rs` changes
  are preserved.

## Implementation Sequence

### 1. Transfer independently valuable coverage

Audit the backend-local and mixed tests before deleting them:

- Retain semantic and NIR facts in their owning semantic/NIR tests.
- Retarget inline-assembler behavior to AST/classic and MIR6502. Remove only
  duplicate SemIR-native assertions and rename tests that currently say “all
  backends.”
- Retarget the SemIR-native relocation test to the SemIR bridge if that bridge
  still lacks explicit relocation coverage.
- Keep shared tracked-emitter tests where they are; they protect MIR6502.
- Delete classifier/materializer/emitter tests whose only contract is the
  removed backend's internal code shape.

This should be a coverage-transfer commit that is green before deletion.

### 2. Remove selection and implementation

- Remove `CodegenSource::SemIrNative`, its compiler and CLI match arms, imports,
  profile aliases, and codegen-source aliases.
- Make `--codegen-source` accept only `ast` and `semir` (`sem-ir` may remain as
  the existing spelling alias).
- Add CLI regressions proving every removed name exits with configuration
  status 2 and a normal invalid-value diagnostic. Do not add a deprecation
  diagnostic or compatibility shim.
- Remove `generate_semir_native_profile_with_origin` from the public codegen
  API and driver.
- Remove `mod semir_native`, `src/codegen/semir_native.rs`, and
  `src/codegen/semir_native/`.
- Remove native-only helpers and cases from `src/codegen/tests.rs`,
  `tests/inline_asm.rs`, and compiler relocation tests after the coverage audit.

### 3. Simplify developer tools

- Make `actionc-semir-sweep` bridge-only. Remove the native candidate,
  unsupported-native parsing, dashboard, and coverage/mixed policy; retain the
  exact bridge comparison, profile/origin selection, verbose output, and useful
  report formatting.
- Remove the SemIR-native re-origining row from
  `tools/check-mads-listings.sh`.
- Remove SemIR-native values and aliases from `tools/compile-run-atr.sh` while
  retaining `ast` and `semir` developer selection.
- Keep generic survey forwarding of `--codegen-source` where it remains useful;
  remove or archive only SemIR-native-specific invocations and reports.

### 4. Retire active documentation

- Move `SEMIR_NATIVE_ARCHITECTURE.md`, `SEMIR_NATIVE_BACKEND_STATUS.md`,
  `SEMIR_NATIVE_STRESS_BACKLOG.md`, and
  `SEMIR_NATIVE_VALIDATION_POLICY.md` into the SemIR-native archive with a
  short retirement header.
- Remove their active links from `docs/README.md` and keep this removal note as
  the current decision record until the cleanup is complete.
- Rewrite `SEMIR_SWEEP.md` as a bridge-only workflow.
- Update active references in `USAGE.md`, `RELEASE_PLAN.md`,
  `PROOF_ARCHITECTURE.md`, `SEMANTIC_INVARIANTS.md`, listing/relocation plans,
  and the Action 2027 rollout note so they describe classic or NIR/MIR6502
  ownership.
- Mark the SemIR-native toolkit survey historical and remove its active index
  link. Do not mechanically rewrite documents already under `docs/archive/`.
- Add a release-note entry that the experimental API and CLI aliases were
  removed without a deprecation window.

### 5. Rename the retained tracked emitter

Completed after the removal merged: `NativeTrackedEmitter` and
`native_emitter.rs` became `TrackedEmitter` and `tracked_emitter.rs`.
The processor-state vocabulary remains unchanged and can be considered
separately.

## Suggested Delivery

Use one removal PR with three reviewable commits:

1. `test: transfer SemIR-native-independent coverage`
2. `codegen: remove the SemIR-native backend and selectors`
3. `docs: archive SemIR-native guidance and record removal`

One PR avoids an intermediate tree where a selectable backend has lost part of
its tests. The implementation should still keep each commit buildable where
practical.

## Verification

Run:

```sh
cargo test --locked --test inline_asm
cargo test --locked --test actionc_cli
cargo test --locked nir_fixtures_match_snapshots
cargo run --locked --bin actionc-nir-sweep -- fixtures/nir
cargo run --locked --bin actionc-semir-sweep -- --profile modern fixtures/semir
cargo test --locked
cargo test --locked --test actionc_cli --test embedded_modules
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml
python3 -m unittest discover -s tools/tests -p 'test_*.py'
```

Also run MADS round-trip checks when MADS is available. Compare representative
objects and listings for compatibility, optimized classic, MIR6502, cart, and
standalone modes against the pre-removal baseline. Only attempts to select the
removed backend should change behavior.

Finish with an active-tree reference audit:

```sh
rg -n -i 'semir[-_ ]native|SemIrNative|generate_semir_native' \
  src tests tools USAGE.md docs surveys \
  --glob '!docs/archive/**' \
  --glob '!docs/SEMIR_NATIVE_REMOVAL_PLAN.md'
```

Any remaining match must be an intentional historical marker, not executable
selection, active guidance, or a maintained-backend expectation.

## Implementation Result

The backend implementation, Rust API, compiler selector, CLI aliases, native
sweep modes, helper invocations, and native-only tests were removed. Shared
tracked-emitter/state infrastructure remains because MIR6502 consumes it. The
four backend documents are archived, and independently owned relocation and
inline-assembler coverage remains on the SemIR bridge, classic, NIR, and
MIR6502 paths.

The modern-profile SemIR bridge fixture sweep reports 32 exact matches. The
legacy profile still reports the pre-existing
`native_fragile_zero_page_vectors.act` mismatch: compatibility AST codegen
loads bytes from a routine body where the SemIR bridge materializes the routine
address. Removal does not change either maintained output to conceal that
separate bridge-fidelity issue.

## Completion Criteria

- No buildable or selectable SemIR-native path remains.
- All former aliases fail closed as ordinary invalid values.
- No active tool or CI job invokes the removed backend.
- Shared language and inline-assembler coverage remains on classic and/or
  NIR/MIR6502 owners.
- MIR6502 retains the tracked emitter and its tests.
- Active documentation describes only maintained paths; historical material is
  clearly archived.
- All required checks pass with no intentional object-code changes outside the
  removed backend.
