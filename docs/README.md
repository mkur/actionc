# Documentation Index

This directory is for current documentation. Historical implementation plans and
old investigation notes live under `archive/` so they can be searched without
crowding the active reference set.

## Start Here

- [../USAGE.md](../USAGE.md) - command-line reference for `actionc` and the run
  helper.
- [CODEGEN_PROFILES.md](CODEGEN_PROFILES.md) - user-facing profile/backend
  naming and policy.
- [ACTION_STORAGE_LAYOUT.md](ACTION_STORAGE_LAYOUT.md) - Action! storage layout
  and compatibility notes.
- [SEMANTIC_INVARIANTS.md](SEMANTIC_INVARIANTS.md) - semantic rules the compiler
  should preserve.
- [NIR_TARGET_SHAPE.md](NIR_TARGET_SHAPE.md) - target NIR contract.
- [NIR_QUALITY_IMPLEMENTATION_PLAN.md](NIR_QUALITY_IMPLEMENTATION_PLAN.md) -
  active plan for storage-to-value promotion and routine-wide NIR quality.
- [MIR6502_PSEUDO_MACHINE_CONTRACT.md](MIR6502_PSEUDO_MACHINE_CONTRACT.md) -
  MIR6502 contract and verifier shape.
- [MIR6502_REWRITE_WORKFLOW_PLAN.md](MIR6502_REWRITE_WORKFLOW_PLAN.md) -
  implementation plan for routine-aware analyses and transactional MIR6502
  rewrites.
- [MIR6502 rewrite workflow baseline](../surveys/tn/mir6502-rewrite-workflow-baseline.md)
  - reproducible TN artifacts and the checked migration inventory for that
  plan.
- [TAC_BOUNDARY_FOR_6502_MIR.md](TAC_BOUNDARY_FOR_6502_MIR.md) - TAC/MIR
  boundary vocabulary.

## Tooling

- [ACTION_COMPILER_VM_USAGE.md](ACTION_COMPILER_VM_USAGE.md) - in-repo compiler
  VM workflow.
- [ALTIRRA_BRIDGE_USAGE.md](ALTIRRA_BRIDGE_USAGE.md) - AltirraBridge workflow
  notes.
- [CODEGEN_COMPARISON_TOOL.md](CODEGEN_COMPARISON_TOOL.md) - focused
  classic-vs-MIR6502 artifact diffs.
- [COMPARE_TOOL.md](COMPARE_TOOL.md) - original compiler comparison workflow.
- [MAP_QUERY_TOOL.md](MAP_QUERY_TOOL.md) - generated map query helper.
- [PROBE_SWEEP_PROCESS.md](PROBE_SWEEP_PROCESS.md) - original compiler probe
  process.
- [SEMIR_SWEEP.md](SEMIR_SWEEP.md) - SemIR sweep workflow.

## Language And Runtime Reference

- [ACTIONC_ANNOTATIONS.md](ACTIONC_ANNOTATIONS.md) - supported `;@actionc`
  annotations.
- [ACTION_SYMBOL_TABLE.md](ACTION_SYMBOL_TABLE.md) - Action! symbol table notes.
- [ATASCII_ESCAPES.md](ATASCII_ESCAPES.md) - textual ATASCII escape format.
- [NAME_RESOLUTION.md](NAME_RESOLUTION.md) - name lookup rules.
- [RUNTIME_HELPER_EFFECTS.md](RUNTIME_HELPER_EFFECTS.md) - known runtime helper
  effects.
- [SYNTAX_EXTENSIONS.md](SYNTAX_EXTENSIONS.md) - supported syntax extensions.
- [resident_library.md](resident_library.md) - resident library notes.

## Architecture And Status

- [CODEGEN_PROOFS.md](CODEGEN_PROOFS.md) and
  [PROOF_ARCHITECTURE.md](PROOF_ARCHITECTURE.md) - proof/fact layer.
- [OBSERVABILITY_NORTH_STAR.md](OBSERVABILITY_NORTH_STAR.md) - observability
  direction.
- [SEMIR_NATIVE_ARCHITECTURE.md](SEMIR_NATIVE_ARCHITECTURE.md) - SemIR-native
  architecture overview.
- [SEMIR_NATIVE_BACKEND_STATUS.md](SEMIR_NATIVE_BACKEND_STATUS.md) - current
  SemIR-native status.
- [SEMIR_NATIVE_STRESS_BACKLOG.md](SEMIR_NATIVE_STRESS_BACKLOG.md) - active
  SemIR-native stress backlog.
- [SEMIR_NATIVE_VALIDATION_POLICY.md](SEMIR_NATIVE_VALIDATION_POLICY.md) - SemIR
  validation policy.
- [BACKLOG.md](BACKLOG.md) - cross-cutting backlog.

## Triage Notes

- [bugs/](bugs/) - focused bug notes that still explain known or recently fixed
  behavior.
- [archive/implementation-plans/](archive/implementation-plans/) - historical
  plans, grouped by subsystem.
- [archive/notes/](archive/notes/), [archive/reviews/](archive/reviews/), and
  [archive/snapshots/](archive/snapshots/) - old investigation artifacts
  retained for archaeology.
