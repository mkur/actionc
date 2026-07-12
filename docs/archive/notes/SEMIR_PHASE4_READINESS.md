# SemIR Phase 4 Readiness

Date: 2026-05-23

Phase 4 established a faithful SemIR bridge: the compiler can lower parsed and
analyzed Action source into SemIR, reconstruct the AST-facing backend input, and
produce byte-identical code for every source in the current sweep where the AST
path itself succeeds.

The current bridge-fidelity gate is:

```sh
cargo run --bin actionc-semir-sweep --
```

Latest full sweep:

```text
SemIR sweep summary: match=86 mismatch=0 ast_failed=10 semir_failed=0 load_failed=1
```

## What This Proves

- SemIR preserves scalar declarations, grouped declarations, array/string
  declarations, record/type declarations, routine declarations, parameters,
  locals, statement structure, control flow, machine blocks, calls, returns,
  and the Action-specific `array(index)` lvalue shape well enough to round-trip
  through the current backend.
- For AST-compilable sources in probes, stress tests, experiment tests, and the
  extracted Action Toolkit set, SemIR bridge output is byte-identical to AST
  output.
- The bridge now keeps compatibility-sensitive declaration grouping, which is
  important because current layout rules distinguish grouped declarations from
  separately reconstructed declarations.
- The bridge preserves unresolved library names and machine-define call
  statements in their original AST-like shapes, avoiding accidental conversion
  into unsupported indirect calls or pointer dereferences.

## Known Non-Bridge Failures

`ASTFAIL` entries are existing frontend/backend limitations. They are not SemIR
bridge regressions because there is no successful AST output to compare against.
Current examples include:

- compat-profile rejection of nested function calls in routine-call arguments
  and arithmetic expressions
- resident-library probes that use call surfaces not yet accepted by current
  AST codegen
- `ALLOCATE.ACT` expression shapes that the current backend still rejects
- the existing `zero_page.act` compatibility layout failure

The single `LOADFAIL` in the default sweep is an include-path/setup issue for
`lib_quit_fixup.act`, not a SemIR lowering result.

## Phase 4 Boundary

Phase 4 should be considered complete for bridge fidelity. Further work should
avoid adding more bridge-only compatibility patches unless `actionc-semir-sweep`
finds a new `MISMATCH` or `SEMFAIL`.

The bridge is still intentionally transitional. It proves that SemIR can carry
today's semantics without changing generated output, but codegen still lowers by
reconstructing AST nodes. That is useful as a migration guard, not the final
architecture.

## Recommended Phase 5 Entry

Start native SemIR-backed lowering in very small slices while continuing to run
the bridge sweep as a regression guard.

Suggested first slices:

1. Native SemIR declarations/storage read model: no behavior change, just build
   direct access to declaration groups, types, and storage facts.
2. Native scalar assignment lowering for trivial `symbol = literal` and
   `symbol = symbol` cases behind an internal switch.
3. Native lvalue/address-shape helpers for symbol, deref, field, and
   call-indexed array lvalues.
4. Keep AST and SemIR outputs byte-identical for compat while allowing modern
   profile changes only after each slice has observability and tests.

The key rule: use SemIR to remove accidental AST-shape dependence, but keep the
current bridge as the compatibility oracle until the native lowering path is
strong enough to replace it file by file.
