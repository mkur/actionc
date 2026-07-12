# Stress Compatibility Survey

Repeatable compatibility and MIR6502 analysis workflows for the maintained
stress inputs in [`fixtures/stress`](../../fixtures/stress/README.md).

This directory owns survey evidence and generated analysis, while
`fixtures/stress` contains only compiler inputs and their short usage note.

## Workflows

Compile every stress fixture with the normal regression policy:

```sh
scripts/check-stress-fixtures.sh
```

Compare selected fixtures with the original Action! compiler through
`action-compiler-vm`:

```sh
surveys/stress/compare-original.sh pointers
surveys/stress/compare-original.sh all
```

Generate MIR6502, materialized-MIR, and source-listing dumps, then classify any
errors:

```sh
surveys/stress/mir6502-sweep.sh
surveys/stress/classify-mir6502-errors.sh
```

Generated captures and dumps go under `surveys/stress/outputs/` and are not
tracked. Curated results are recorded in [STATUS.md](STATUS.md) and detailed
original-versus-`actionc` notes in [COMPARISON.md](COMPARISON.md).
