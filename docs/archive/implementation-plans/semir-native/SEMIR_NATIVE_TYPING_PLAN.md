# SemIR Native Typing Layer Plan

Owner: semantic analysis and SemIR construction.

The typing layer answers what a program means in Action terms. It must not know
6502 opcode sequences, zero-page scratch homes, register pressure, or native
ABI packing details.

## Responsibilities

- value type, width, signedness, pointer target, and record identity;
- lvalue legality and read/write access;
- callable signatures, parameter storage kind, and return type;
- array origin, element type, declared length, and decay rules;
- record field identity, offset, and field type;
- control-flow facts such as function return coverage and loop exits.

Typing may reject invalid Action programs. It should not reject a valid program
only because the current native backend cannot lower it yet; unsupported native
lowering belongs to classification or materialization diagnostics.

## Current Direction

SemIR already carries most of the facts the native backend needs. The remaining
work is to keep backend code from rederiving those facts from source syntax.

Current state: stable. No broad typing work is recommended right now. Reopen
this layer only when native lowering exposes a missing semantic fact, such as a
width, pointer target, record field, array origin, callable signature, or
legality rule that cannot be represented cleanly by classification.

The stress backlog does not currently imply broad typing work. It does,
however, identify facts that every materialization slice should verify before
adding backend lowering:

- mixed-width word expressions carry the intended result width and signedness,
  especially forms such as `RETURN(0 - x)`, `word + byte`, and word comparisons
  against byte constants;
- pointer dereference and pointer-index expressions carry pointee width and
  signedness for `BYTE`, `CARD`, `INT`, and record pointers;
- record-field lvalues carry field identity, offset, width, and record type
  even when the base is a pointer parameter or pointer variable;
- builtin, runtime, indirect, and user calls carry callable signatures,
  argument widths, parameter storage kinds, return types, and effect facts;
- dynamic array compound assignments preserve the operator and target element
  facts needed by materialization;
- fixed-address zero-page aliases retain ordinary scalar or pointer facts rather
  than requiring backend syntax rediscovery.

Near-term plan:

1. Keep SemIR type facts complete enough that backend code does not rederive
   widths, pointer targets, array origins, or record fields from raw expression
   shapes.
2. Prefer adding missing semantic facts to SemIR over adding backend pattern
   matching.
3. Maintain focused semantic tests for every fact the native backend consumes.
4. Keep compatibility quirks out of typing unless they are actual Action
   language legality rules.
5. Document any semantic fact that exists mainly to support native lowering in
   `SEMANTIC_INVARIANTS.md` or this note.
6. For each stress backlog slice, add semantic tests only if one of the facts
   above is missing or ambiguous; otherwise leave typing unchanged and put the
   fix in classification or materialization.

## Boundary Checks

Typing code should not:

- choose native storage addresses;
- emit instructions;
- know A/X/Y or zero-page scratch homes;
- decide whether a shape is cheaper as immediate, storage, or indirect code.

Typing code may:

- expose stable facts on SemIR nodes;
- normalize source forms into typed value/place/call shapes;
- preserve source spans and diagnostics for invalid programs.
