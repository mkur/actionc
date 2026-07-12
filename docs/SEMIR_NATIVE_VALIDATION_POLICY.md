# SemIR Native Validation Policy

Native SemIR lowering uses byte-for-byte comparison as a calibration tool, not
as the long-term product contract.

The main risk is overfitting to incidental code shape from the original Action!
compiler. Exact matching is useful for proving ABI and layout facts, but it can
also preserve single-pass compiler accidents that would block modern codegen.

## Validation Classes

Exact fixtures:

- Small language atoms in `fixtures/semir`.
- Storage and ABI facts: scalar layout, array descriptors, SArgs, return slots,
  zero-page aliases, pointer dereference width, record field offsets.
- These should normally remain byte-for-byte identical to the AST backend while
  native SemIR is being brought up.

Coverage programs:

- Stress tests, toolkit files, and large real programs.
- These should prove that native SemIR can compile the construct mix and produce
  useful diagnostics.
- Byte differences are reported as `DELTA`, not automatically treated as bugs.

Bug candidates:

- Any exact fixture mismatch.
- Any semantic/layout difference with observable consequences.
- Any unsupported construct that is already required by the chosen phase.
- Any native backend panic or non-diagnostic failure.

Expected future deltas:

- Branch inversion.
- Tail calls.
- Register reload elimination.
- Pointer/effective-address reuse.
- Better call/result materialization.
- Later TAC/SSA-driven optimizations.

## Practical Rule

Use exact matching to lock down Action! semantics and externally visible layout.
Use coverage mode to keep space for better 6502 code generation.

See `SEMIR_NATIVE_BACKEND_STATUS.md` for the latest supported/matched fixture
counts and current TN runtime-validation state.

Do not add a native lowering rule only because the original compiler happened to
emit a particular opcode sequence. First identify the semantic, ABI, or layout
reason. If there is no such reason, the difference belongs in coverage as a
modernization opportunity.

## Tooling

Strict fixture calibration:

```sh
cargo run --bin actionc-semir-sweep -- --candidate native --dashboard fixtures/semir
```

Mixed validation:

```sh
cargo run --bin actionc-semir-sweep -- \
  --candidate native \
  --dashboard \
  --validation-policy mixed \
  fixtures/semir fixtures/stress
```

In mixed mode:

- `fixtures/semir/**` remains exact.
- other paths are coverage programs.
- successful non-identical output is reported as `DELTA`.
- `DELTA` does not make the tool exit nonzero.
