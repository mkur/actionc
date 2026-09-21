# Documentation Index

This directory is for current documentation. Historical implementation plans and
old investigation notes live under `archive/` so they can be searched without
crowding the active reference set.

## Start Here

- [../USAGE.md](../USAGE.md) - command-line reference for `actionc` and the run
  helper.
- [ACTIONC_RUN.md](ACTIONC_RUN.md) - cross-platform `actionc-run` usage,
  emulator discovery, and artifact lifetime.
- [Modules tutorial](tutorials/MODULES.md) - step-by-step introduction to
  project modules, imports, embedded Atari interfaces, and runtime selection.
- [ATARI_SIO_LIBRARY_DESIGN.md](ATARI_SIO_LIBRARY_DESIGN.md) - FujiNet-focused
  SIO library design with device constants, variant results, convenient network
  calls and optional buffering.
- [Native REAL tutorial](tutorials/REAL.md) - practical introduction to REAL
  arithmetic, conversions, storage, math, and I/O.
- [CODEGEN_PROFILES.md](CODEGEN_PROFILES.md) - user-facing profile/backend
  naming and policy.
- [ACTION_STORAGE_LAYOUT.md](ACTION_STORAGE_LAYOUT.md) - Action! storage layout
  and compatibility notes.
- [SEMANTIC_INVARIANTS.md](SEMANTIC_INVARIANTS.md) - semantic rules the compiler
  should preserve.
- [NIR_TARGET_SHAPE.md](NIR_TARGET_SHAPE.md) - target NIR contract.
- [FIXED_MULTIDIMENSIONAL_ARRAYS_IMPLEMENTATION_PLAN.md](FIXED_MULTIDIMENSIONAL_ARRAYS_IMPLEMENTATION_PLAN.md)
  - proposed fixed shapes, row-major indexing, shared lowering and matrix1/DCT
    acceptance on both 6502 backends and MIR68K.
- [NIR_TARGET_INDEPENDENCE_IMPLEMENTATION_PLAN.md](NIR_TARGET_INDEPENDENCE_IMPLEMENTATION_PLAN.md)
  - completed sliced migration making NIR consumable by independent 6502,
  65816, and 68k backends while separating compatibility and native layouts.
- [NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md](NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md)
  - completed follow-on for reentrant native calls and invocation-scoped local
  storage while preserving the classic Atari routine ABI.
- [MIR65816_EXEC_READINESS_REQUIREMENTS.md](MIR65816_EXEC_READINESS_REQUIREMENTS.md)
  - compiler contracts and executable acceptance gates required before starting
    standalone Exec implementation in native Action! on the 65816.
- [MIR65816_EXEC_ACCEPTANCE.md](MIR65816_EXEC_ACCEPTANCE.md)
  - G1–G6 emulator evidence for the initial Exec subset, reproduction commands,
    supported operations, stack limits and remaining board validation.
- [MIR65816_EXEC_READINESS_REVIEW.md](MIR65816_EXEC_READINESS_REVIEW.md)
  - baseline audit, lowering corrections, physical ABI decisions and proposed
    implementation sequence for reaching the Exec readiness gates.
- [MIR65816_LOWERING_CONTRACT.md](MIR65816_LOWERING_CONTRACT.md)
  - preserved signedness, data identities, initialization, alignment and relocation
    byte selection in both native 65816 models, with regression coverage.
- [MIR65816_PHYSICAL_ABI_V1.md](MIR65816_PHYSICAL_ABI_V1.md)
  - Specifies the native far-call ABI, aligned stack arguments, A/X results,
    direct-page ownership, interrupt frames and first-task construction, with
    machine-readable constants and verified layout plans. Initial Exec subset
    emission and context qualification are complete.
- [MIR65816_EMISSION_CONTRACT.md](MIR65816_EMISSION_CONTRACT.md)
  - Native scalar emission, freestanding image/assembly interfaces, checked
    allocation, execution evidence and the initial driver's supported subset.
- [MIR65816_STATE_TRACKER_DESIGN.md](MIR65816_STATE_TRACKER_DESIGN.md)
  - Instruction-aware native state tracking based on the 6502 emitter:
    register/flag/width facts, memory ownership, barriers and staged integration.
- [MIR65816_STATE_TRACKER_IMPLEMENTATION_PLAN.md](MIR65816_STATE_TRACKER_IMPLEMENTATION_PLAN.md)
  - Completed first state-tracker slice: typed emission, checked mode/stack contracts,
    unchanged forwarding policy and byte-identical qualification.
- [MIR65816_STATE_TRACKER.md](MIR65816_STATE_TRACKER.md)
  - Qualified tracker integration: identical code and measurements, independent
    register/flag/home traces, preserved interrupts, relocation and stack guards.
- [MIR65816_CODE_QUALITY_PLAN.md](MIR65816_CODE_QUALITY_PLAN.md)
  - Measured native code-quality roadmap, completed optimizations and remaining
    control-flow, copy and register-allocation work.
- [MIR65816_CONTROL_FLOW_IMPLEMENTATION_PLAN.md](MIR65816_CONTROL_FLOW_IMPLEMENTATION_PLAN.md)
  - Proposed slices 3a–3c: checked MIR-entry width omission, adjacent-block
    fallthrough and bounded short branches, with independent qualification gates.
- [MIR65816_IMPLEMENTATION_PLAN.md](MIR65816_IMPLEMENTATION_PLAN.md)
  - Tracks separately committed ABI, frame, emission and context-qualification
    slices with explicit completion checks.
- [MIR65816_POINTER_ALLOCATION_PLAN.md](MIR65816_POINTER_ALLOCATION_PLAN.md)
  - Plans native pointer promotion and bounded direct-page allocation, including
    scratch ownership, physical-location maps and execution/code-size checks.
- [MIR65816_CPU_EXECUTION_CHECKPOINT.md](MIR65816_CPU_EXECUTION_CHECKPOINT.md)
  - X65 C qualification, safe Rust port, VM integration, Altirra cross-check,
    status-timing corrections, and remaining CPU/hardware scope.
- [65816 CPU test drive](../tools/vm65816-runtime-tests/README.md)
  - pinned jgenesis core, executable CPU qualification and confirmed NMI/reset
    limitations of that earlier CPU experiment.
- [MIR68K_EXECUTION_CONTRACT.md](MIR68K_EXECUTION_CONTRACT.md)
  - stable storage and control-flow facts, native ABI metadata, and verifier guarantees.
- [MIR68K_AMIGA_EXECUTABLE_IMPLEMENTATION_PLAN.md](MIR68K_AMIGA_EXECUTABLE_IMPLEMENTATION_PLAN.md)
  - implemented runtime adapters, relocatable HUNK output, Shell startup and
    console I/O; the AmigaOS 3.1 smoke run passed in vAmiga.
- [AMIGA.md](AMIGA.md)
  - experimental Amiga CLI, supported calls, examples and reproducible vAmiga smoke procedure.
- [MIR68K_INTEGER_COMPLETION_PLAN.md](MIR68K_INTEGER_COMPLETION_PLAN.md)
  - multiplication, division/remainder, native faults and portable benchmark acceptance.
- [MIR68K_CODE_QUALITY_PLAN.md](MIR68K_CODE_QUALITY_PLAN.md)
  - native DCT/ADPCM acceptance, measured baselines, temporary forwarding,
    compact instruction selection and checked branch relaxation.
- [MIR68K_C_COMPARISON.md](MIR68K_C_COMPARISON.md)
  - equivalent MC68000 GCC benchmarks, shared r68k validation and measured
    priorities for alignment, register allocation and control flow.
- [MIR68K_OPTIMIZATION_IMPLEMENTATION_PLAN.md](MIR68K_OPTIMIZATION_IMPLEMENTATION_PLAN.md)
  - proposed slices for proven pointer alignment, direct comparison branches,
    native scalar promotion and bounded register allocation across blocks.
- [MIR68K_MINIMAL_EXECUTION_PLAN.md](MIR68K_MINIMAL_EXECUTION_PLAN.md)
  - completed minimal path from verified NIR to MC68000 code, an r68k
  execution harness, symbol-based test access and insertion-sort acceptance.
- [NATIVE_TYPE_SURFACE_IMPLEMENTATION_PLAN.md](NATIVE_TYPE_SURFACE_IMPLEMENTATION_PLAN.md)
  - completed sliced implementation of 32-bit integers, general function
  results, typed callable pointers, and target-sized address and size values.
- [NIR_ATARI_BASELINES.md](NIR_ATARI_BASELINES.md) - byte-exact Atari object
  guardrails for the target-independence migration.
- [NIR_QUALITY_IMPLEMENTATION_PLAN.md](NIR_QUALITY_IMPLEMENTATION_PLAN.md) -
  active plan for storage-to-value promotion and routine-wide NIR quality.
- [NIR_KNOWN_CONSTRUCTOR_TAG_PROPAGATION_PLAN.md](NIR_KNOWN_CONSTRUCTOR_TAG_PROPAGATION_PLAN.md)
  - proposed bounded U8 subregion propagation to fold known-constructor CASE
    dispatch through verified NIR, with CFG, snapshot and effect safety gates.
- [PRIVATE_AGGREGATE_FORWARDING_IMPLEMENTATION_PLAN.md](PRIVATE_AGGREGATE_FORWARDING_IMPLEMENTATION_PLAN.md)
  - shared plan for eliminating private record, union and variant staging while
  preserving value snapshots, effects and aggregate ABI boundaries.
- [PRIVATE_AGGREGATE_FORWARDING_BASELINE.md](PRIVATE_AGGREGATE_FORWARDING_BASELINE.md)
  - shared NIR/VM corpus, logical versus ABI-expanded copy counts and initial
  read-only aggregate storage proofs.
- [PRIVATE_AGGREGATE_FRESH_INITIALIZATION.md](PRIVATE_AGGREGATE_FRESH_INITIALIZATION.md)
  - slice 2 direct fresh initialization, conservative fallbacks, regression
  coverage and before/after code-size and VM-cycle measurements.
- [PRIVATE_AGGREGATE_BOUNDED_FORWARDING.md](PRIVATE_AGGREGATE_BOUNDED_FORWARDING.md)
  - slice 3 same-block snapshot proofs, complete reference accounting, retained
  mutation/ABI boundaries and four-target NIR/Atari VM measurements.
- [COMPILER_API_IMPLEMENTATION_NOTE.md](COMPILER_API_IMPLEMENTATION_NOTE.md) -
  plan for a reusable, side-effect-free file compilation API shared by
  `actionc` and `actionc-run`.
- [EMULATOR_ADAPTERS_IMPLEMENTATION_NOTE.md](EMULATOR_ADAPTERS_IMPLEMENTATION_NOTE.md)
  - sliced plan for the Atari800 and Altirra adapters used by `actionc-run`.
- [INLINE_ASSEMBLER_IMPLEMENTATION_PLAN.md](INLINE_ASSEMBLER_IMPLEMENTATION_PLAN.md)
  - sliced plan for an integrated, MADS-style 6502 inline assembler with stable
    Action! object references and shared compiler effects.
- [MADS_COMPATIBLE_LISTING_IMPLEMENTATION_PLAN.md](MADS_COMPATIBLE_LISTING_IMPLEMENTATION_PLAN.md)
  - completed first-stage implementation note for byte-preserving MADS
    assembly listings.
- [REORIGINABLE_MADS_LISTING_IMPLEMENTATION_PLAN.md](REORIGINABLE_MADS_LISTING_IMPLEMENTATION_PLAN.md)
  - completed follow-up for re-origining generated listings by changing one
    origin definition.
- [MIR6502_PSEUDO_MACHINE_CONTRACT.md](MIR6502_PSEUDO_MACHINE_CONTRACT.md) -
  MIR6502 contract and verifier shape.
- [INLINE routine implementation plan](INLINE_IMPLEMENTATION_PLAN.md) -
  declaration modifier, typed preference propagation, wider scalar
  inlining and Q4.12 wrapper rollout in six slices.
- [MIR6502_REWRITE_WORKFLOW_PLAN.md](MIR6502_REWRITE_WORKFLOW_PLAN.md) -
  implementation plan for routine-aware analyses and transactional MIR6502
  rewrites.
- [MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md](MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md)
  - active general plan for counted loops, register facts, and direct value
  consumers.
- [MIR6502_COMPARE_BRANCH_FUSION_PLAN.md](MIR6502_COMPARE_BRANCH_FUSION_PLAN.md)
  - implemented comparison-result copy elimination before branch selection, with
  shared use proofs, numeric-result fallbacks, and measured Mandelbrot validation.
- [MIR6502_WIDE_SHIFT_COPY_REDUCTION.md](MIR6502_WIDE_SHIFT_COPY_REDUCTION.md)
  - byte and nibble projections for constant wide shifts, preserving captured
  operands while reducing staging for narrow results.
- [MIR6502_COUNTED_LOOP_LATCH_RELAXATION_PLAN.md](MIR6502_COUNTED_LOOP_LATCH_RELAXATION_PLAN.md)
  - proposed follow-on for first-entry machine-state reconstruction and
  trip-count-aware latch profitability.
- [MIR6502_STATIC_ARRAY_AFFINE_INDEX_PLAN.md](MIR6502_STATIC_ARRAY_AFFINE_INDEX_PLAN.md)
  - focused follow-on for widened byte indexes, direct indexed accumulation,
  and full-range Y-carried loops.
- [MIR6502 rewrite workflow baseline](../surveys/tn/mir6502-rewrite-workflow-baseline.md)
  - reproducible TN artifacts and the checked migration inventory for that
  plan.
## Tooling

- [ACTIONC_VM_USAGE.md](ACTIONC_VM_USAGE.md) - in-repo compiler
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

- [Signed Q8.8 fixed point](FIXED_POINT_Q8_8.md) - embedded library using INT
  storage and LONGINT intermediates, with explicit rounding, wrapping, and
  division faults across all six compiler mode/runtime combinations.
- [Signed Q4.12 fixed point](FIXED_POINT_Q4_12.md) - a smaller range with
  twelve fractional bits, explicit floor multiplication, and unscaled wide squares.
- [IF and CASE expressions](tutorials/IF_CASE_EXPRESSIONS.md) - choosing integer
  and enum values, exhaustive variant matching, exact arm types and evaluation order.
- [Modules tutorial](tutorials/MODULES.md) - task-oriented guide to creating
  and running modular programs.
- [Native REAL tutorial](tutorials/REAL.md) - task-oriented guide to using the
  six-byte Atari packed-decimal type and its first-party libraries.
- [Variants and pattern matching](tutorials/VARIANTS.md) - constructors, immutable
  snapshots, generic records/variants, nested patterns, guards and fixed arenas.
- [Variant storage contract](VARIANT_STORAGE_CONTRACT.md) - identical/disjoint
  checked transfers, overlap faults and single-copy assignment lowering.
- [Untagged unions](tutorials/UNIONS.md) - overlapping typed storage, snapshots,
  generics, volatile access and the backend support matrix.
- [Union code-quality baseline](Action_2027/UNIONS_CODEGEN_AUDIT.md) - direct
  views versus explicit aliases, with aggregate copy costs accounted separately.
- [ADT code-quality baseline](ADT_CODEGEN_BASELINE.md) - executable comparison
  with handwritten tagged records, measured costs and existing optimization coverage.
- [ACTIONC_ANNOTATIONS.md](ACTIONC_ANNOTATIONS.md) - supported `;@actionc`
  annotations.
- [ACTION_SYMBOL_TABLE.md](ACTION_SYMBOL_TABLE.md) - Action! symbol table notes.
- [ATASCII_ESCAPES.md](ATASCII_ESCAPES.md) - textual ATASCII and ANTIC
  screen-code escape formats.
- [NAME_RESOLUTION.md](NAME_RESOLUTION.md) - name lookup rules.
- [Action 2027 modules and runtime usage](Action_2027/MODULES_AND_RUNTIME_USAGE.md)
  - named-module syntax, lookup, examples, and user-module versus runtime code
  inclusion.
- [RUNTIME_HELPER_EFFECTS.md](RUNTIME_HELPER_EFFECTS.md) - known runtime helper
  effects.
- [SYNTAX_EXTENSIONS.md](SYNTAX_EXTENSIONS.md) - supported syntax extensions.
- [ENUM and CASE design](Action_2027/ENUM_AND_CASE_DESIGN.md) - implemented modern
  BYTE enum types and non-fallthrough dispatch with ordered guards.
- [ENUM and CASE implementation plan](Action_2027/ENUM_AND_CASE_IMPLEMENTATION_PLAN.md)
  - sliced delivery of TYPE-based enums, enum function results, and CASE/ESAC,
    with backend/runtime acceptance gates; guard follow-on delivered by the ADT plan.
- [Algebraic data types](Action_2027/ALGEBRAIC_DATA_TYPES_IMPLEMENTATION_PLAN.md)
  - sliced delivery of aggregate values/calls, variants, generics and matching.
- [Untagged unions](Action_2027/UNIONS_IMPLEMENTATION_PLAN.md) - accepted plan for
  overlapping typed storage views; public support remains gated during delivery.
- [resident_library.md](resident_library.md) - resident library notes.

## Architecture And Status

- [SEMIR_NATIVE_REMOVAL_PLAN.md](SEMIR_NATIVE_REMOVAL_PLAN.md) - implemented
  decision record for removing the direct SemIR-native backend without a
  deprecation window while retaining shared MIR6502 emission infrastructure.
- [CODEGEN_PROOFS.md](CODEGEN_PROOFS.md) and
  [PROOF_ARCHITECTURE.md](PROOF_ARCHITECTURE.md) - proof/fact layer.
- [OBSERVABILITY_NORTH_STAR.md](OBSERVABILITY_NORTH_STAR.md) - observability
  direction.
- [BACKLOG.md](BACKLOG.md) - cross-cutting backlog.

## Triage Notes

- [bugs/](bugs/) - focused bug notes that still explain known or recently fixed
  behavior.
- [archive/implementation-plans/](archive/implementation-plans/) - historical
  plans, grouped by subsystem.
- [archive/notes/](archive/notes/), [archive/reviews/](archive/reviews/), and
  [archive/snapshots/](archive/snapshots/) - old investigation artifacts
  retained for archaeology.

- [Native 65816 context interface](MIR65816_CONTEXT_INTERFACE.md): task fabrication,
  assembly entry/restore and platform configuration.
