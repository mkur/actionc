# SemIR Bridge Sweep

`actionc-semir-sweep` compares the current AST-backed codegen path with the
SemIR bridge path. It is a bridge-fidelity tool: for every source that the AST
path can compile, SemIR codegen should produce byte-identical output.

Run the default sweep:

```sh
cargo run --bin actionc-semir-sweep --
```

By default the candidate is the SemIR bridge. This compares AST codegen against
`generate_semir_profile_with_origin`, which currently reconstructs the AST-facing
backend input.

The default roots are:

- `surveys/probes/original-compiler`
- `fixtures/stress`
- `corpora/toolkit/original/extracted`

Run selected files or directories:

```sh
cargo run --bin actionc-semir-sweep -- fixtures/stress corpora/toolkit/original/extracted/PMG.ACT
```

Use the modern profile:

```sh
cargo run --bin actionc-semir-sweep -- --profile modern fixtures/stress
```

Prepare for native SemIR codegen comparisons:

```sh
cargo run --bin actionc-semir-sweep -- --candidate native fixtures/stress
```

`--candidate native` is intentionally wired before native SemIR codegen exists.
Until Phase 5 adds the first native lowering slice, it reports `SEMFAIL` with
`native SemIR codegen is not implemented yet`. Once native lowering is wired,
this same mode should compare AST output against native SemIR output.

Track native SemIR backend progress as a support dashboard:

```sh
cargo run --bin actionc-semir-sweep -- --candidate native --dashboard fixtures/semir
```

Dashboard mode treats native "not implemented yet" diagnostics as
`UNSUPPORTED` instead of `SEMFAIL`, then groups them by first blocker reason.
This is intended for planning native-lowering work: normal strict sweeps should
still be used when a candidate is expected to support a file.

Native SemIR can also be run with an explicit validation policy:

```sh
cargo run --bin actionc-semir-sweep -- \
  --candidate native \
  --dashboard \
  --validation-policy mixed \
  fixtures/semir fixtures/stress
```

Validation policies:

- `exact`: byte-for-byte matching is required for every compiled file. This is
  the default and remains the right mode for bridge fidelity and small SemIR
  calibration fixtures.
- `coverage`: successful codegen is the primary signal. Byte differences are
  reported as `DELTA` instead of `MISMATCH`.
- `mixed`: `fixtures/semir/**` stays exact; other paths are coverage programs.
  This is the preferred mode for native SemIR progress sweeps across fixtures
  plus larger stress/toolkit-style sources.

Emit a markdown report instead of line-oriented console output:

```sh
cargo run --bin actionc-semir-sweep -- \
  --candidate native \
  --profile modern \
  --validation-policy coverage \
  --report markdown \
  --dashboard \
  fixtures/stress
```

Markdown report mode is intended for TAC-runway status updates: it preserves
the same result classes and exit behavior while formatting the summary,
per-file rows, and optional support dashboard as pasteable markdown tables.

Result classes:

- `MATCH`: AST and SemIR output match exactly.
- `DELTA`: coverage-policy result where both paths compiled but output differs.
  Treat this as useful comparison evidence, not automatically as a bug.
- `MISMATCH`: both paths compiled, but output differs. Treat this as a SemIR
  bridge bug until proven otherwise, or as a native exact-fixture bug.
- `UNSUPPORTED`: dashboard-only native candidate result for a feature that the
  native backend explicitly has not implemented yet.
- `SEMFAIL`: AST compiled, but SemIR bridge codegen failed or panicked. Treat
  this as a SemIR bridge bug.
- `ASTFAIL`: the AST path failed too. This is outside SemIR bridge fidelity.
- `LOADFAIL`: parsing, include expansion, or semantic analysis failed before
  codegen.

The tool exits nonzero for `MISMATCH` or `SEMFAIL`. `DELTA`, `ASTFAIL`, and
`LOADFAIL` are reported but do not fail the sweep. `ASTFAIL` and `LOADFAIL`
do not fail because there is no successful AST output to compare against.

See also: `docs/SEMIR_NATIVE_VALIDATION_POLICY.md`.

## SemIR Shape Fixtures

The bridge sweep proves byte-equivalence. The fixture snapshots in
`fixtures/semir` prove that the SemIR text shape itself stays stable for key
language constructs. Run them with:

```sh
cargo test --test semir_fixtures
```

Refresh an intentional fixture change with:

```sh
cargo run --bin actionc-emit -- --emit-semir fixtures/semir/name.act > fixtures/semir/name.semir
```
