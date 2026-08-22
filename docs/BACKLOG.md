# Backlog

This file tracks cross-cutting compiler work that does not naturally belong to a
single backend or survey note.

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
