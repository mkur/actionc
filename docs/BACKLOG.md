# Backlog

This file tracks cross-cutting compiler work that does not naturally belong to a
single backend or survey note.

## Oscar64 Conformance Regressions

The [first eight test ports](../fixtures/runtime/oscar64/README.md#compiler-regressions)
now pass all 258 VM cases (14 tests, no ignored cases). Both
`OSCAR-CLASSIC-WORD-INDEX` and `OSCAR-MIR-SELF-INDEX-STORE` are fixed without
changing the source loops or expected values. MIR post-home rewrites check
replacement dependencies, and verification rejects undefined private scratch
reads. The broader indexed-backing coverage audit remains a separate follow-up.

The [second-batch plan](OSCAR64_TEST_PORTING_PLAN.md) has arithmetic composition
and reverse-copy ports implemented. `OSCAR-CLASSIC-COMPUTED-INDEX` is now fixed:
the general pointer-index fallback retains its captured base across recursive
index materialization. All 2,172 Oscar64 VM cases pass, including the 120
formerly failing classic reverse-copy cases, with unchanged fixture expressions
and oracles. See [the diagnosis and fix](bugs/CLASSIC_COMPUTED_POINTER_INDEX_BUG.md).

Stage 3 adds 1,536 passing nested-call VM cases. `OSCAR-COMPAT-NESTED-CALL`
is fixed by sharing protected argument staging across classic profiles, looking
through casts, and materializing each stacked argument at the public ABI base.
All 3,708 Oscar64 cases now pass without changing port expressions or oracles.
The separately exposed optimized word-return accumulator-fact regression is
also fixed: inferred return facts now use the existing register/value equality
proof instead of equating `Unknown` descriptions. Focused execution covers
assignment, argument and pointer-index consumers, plus multiple return paths.
See the [diagnosis and repairs](bugs/CLASSIC_NESTED_CALL_ARGUMENT_BUG.md).

Stage 4 now distinguishes cartridge-compatible branches from the agreed modern
comparison-value extension. Shared comparison machinery supports BYTE 0/1
values in modern classic and MIR6502; Compatibility rejects value uses during
semantic analysis. The broader grid also exposed and repaired signed-subtract
overflow in both classic profiles. The 408 branch/count cases and 264 modern
value cases bring Oscar64 coverage to 4,380 cases in 24 tests. Stage 5's record
ports remain next; see [the contract and repairs](bugs/COMPARISON_VALUE_MATERIALIZATION_GAPS.md).

Stage 5 is paused while [embedded fixed-length record arrays](EMBEDDED_RECORD_ARRAYS_IMPLEMENTATION_PLAN.md)
are implemented. Preserve inline member storage in `structmembertest.c` rather
than adapting it to pointer fields. The feature is not publicly enabled yet.

## Standalone Runtime Licensing

- Replace the GPL-only standalone `SYS` implementation with an independently
  maintained syslib under a more permissive license.
- Preserve the public `SYS` interface and selective-linking boundary so existing
  programs do not need source changes.
- Until that replacement exists, retain the standalone GPL warning for selected
  `SYS` procedures, compiler helpers, and their runtime dependencies.

## Arithmetic Compatibility Follow-ups

- Extend the executable original-compiler probe corpus for integer arithmetic;
  the compiler regressions now cover ordered multiplication types, products
  above `$FF`, comparison contexts, unary negation, constant BYTE widening, and
  explicit truncation.
- Audit mixed-type shift result width against the cartridge and document any
  intentionally rejected quirks.
- Audit signed-helper selection for `CARD` division and remainder against the
  cartridge before changing those operators.

## Builtin Symbol Coverage

- Add tests that enumerate all valid Action! builtin symbols and verify that
  each compiler path recognizes them consistently.
- Cover semantic analysis, legacy/compat codegen, modern/MIR6502 codegen, and
  SemIR/NIR lowering where applicable.
- Distinguish intentionally unresolved symbols from missing support, so names
  such as resident variables and library/runtime routines do not silently drift
  between backends.
- Include builtin routines, predefined/resident variables, byte arrays, pointer
  forms, and aliases/case variants accepted by Action!.
