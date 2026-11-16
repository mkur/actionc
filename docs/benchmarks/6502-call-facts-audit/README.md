# MIR6502 fact-contract audit evidence

See the [audit](../../MIR6502_FACT_CONTRACT_AUDIT_2026-11-16.md) for conclusions,
severity, consumers and repair order. This is a historical diagnostic snapshot
at `2c5ae7add99f42aeb22ed34fc068ee60f2c9ea80`, not an expected-output test that
requires the compiler to preserve the identified defects.

## Artifacts

- [controls.json](controls.json): nine small independent source programs, their
  selected MIR, assembly, actual entry/exit facts and residence decisions.
- [synthetic.json](synthetic.json): offset projection for all six applicable
  memory-region kinds; finite versus unknown may-write liveness; identical-body
  result composition with integer/pointer ABI facts; store-only read footprint.
- [tn.compact.json](tn.compact.json): facts for all 95 selected TN routines,
  including each direct MIR call's residence-input check. This omits full MIR
  text, repeated globals/machine payloads and repeated write-footprint queries.
- [selected.asm](selected.asm): emitted assembly for eleven routines discussed
  in the audit. Addresses correspond to the qualified 9,811-byte TN image.
- [receipt.json](receipt.json): revision, initial diagnostic result and artifact
  hashes. The initial run also asserted byte equality with the pre-existing
  acyclic-call-homes qualification image.
- [reproduction.json](reproduction.json): successful retained-runner check;
  archived facts reproduced, TN hash unchanged, production module restored.
- [probe.rs](probe.rs) and [run.py](run.py): reproducible private-analysis probe.
  The retained probe records the image; the runner compares its hash with the
  archived receipt, avoiding a dependency on a pre-existing local build.

Entry/exit facts are read from the selected program, after final selection.
`legacy_ignores_flags`, `precise_entry_flags`, `machine_entry`, `leaf_entry` and
`acyclic_entry` are deliberately separate: their disagreements are audit data.
`precise_entry_flags` does not compose known call contracts. The acyclic entry
does. A null entry means the corresponding proof failed or is unsupported.

The current probe also records `direct_result_abi` and each entry fact's
`defined_results`. Its issue-4 checks require matching integer/pointer-parameter
result composition, retain the Unknown-result negative control, and verify the
corrected `Next`/`Ord`/`Items` facts. Archived JSON remains the original audit
evidence; these checks describe the repaired compiler.

Direct call `op` indices are local to their block. The compact census preserves
order but is not a unique-site patching format. Calls embedded inside machine
payloads do not appear in the direct MIR call list.

## Reproduce

From the repository root:

```sh
python3 docs/benchmarks/6502-call-facts-audit/run.py
```

The runner temporarily appends a test-only include to `known_callees.rs`, runs
exactly one diagnostic library test, then removes the include in a `finally`
block. Do not run concurrent builds or edits of that file. It honors an existing
`CARGO_TARGET_DIR` and otherwise reuses `build/checked-arena/shared-target`.

Generated files go to ignored `build/call-facts-audit-20261116/`. The full TN
inventory there includes assembly and printed MIR. The archived evidence is
not overwritten. On a repaired compiler, changed fact answers are expected;
`matches_audited_tn_image` reports whether the TN binary still matches this
historical baseline.

The diagnostic compiles source controls and TN, and executes the analysis
queries. It does not execute the generated programs in a 6502 VM. It establishes
the reported analysis-contract failures, not a new end-to-end runtime failure.
Only the focused diagnostic was run; no full test suite was required for this
audit. No frontend/NIR/compiler behavior was changed.
