# Backlog

This file tracks cross-cutting compiler work that does not naturally belong to a
single backend or survey note.

## Oscar64 Conformance Regressions

The [first eight test ports](../fixtures/runtime/oscar64/README.md#outstanding-compiler-regressions)
retain one outstanding correctness issue as two explicitly ignored VM tests:

- `OSCAR-MIR-SELF-INDEX-STORE`: isolate why an inline word-array store with the
  same induction variable as index and value reads an undefined index spill;
  restore the typed word-index definition through lowering/materialization.

Enable the retained correct-result tests after each fix; do not change their
source loops or accept the current wrong values as backend-specific semantics.

`OSCAR-CLASSIC-WORD-INDEX` is fixed: the general pointer path now uses the array
path's two-carry schedule. All three classic boundary regressions are active.

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
