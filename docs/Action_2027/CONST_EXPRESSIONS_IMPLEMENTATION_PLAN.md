# CONST Expressions Implementation Plan

Implementation status: complete on the `Action-2027` branch. The slices below
also serve as the feature's design record and regression checklist.

## Goal

Add first-class compile-time constants to `actionc` without changing Action!'s
textual `DEFINE` facility:

```action
CONST BYTE TOP_BLANK_ROWS = 4
CONST BYTE FIRST_VISIBLE_VCOUNT = 2 + TOP_BLANK_ROWS
CONST CARD DISPLAY_LIST_A_BASE = $5000
CONST CARD DISPLAY_LIST_B_BASE = DISPLAY_LIST_A_BASE + $400
```

`CONST` is an `actionc` language extension. It is accepted in both the `legacy`
and `modern` profiles and must work with the classic and MIR6502 backends. The
original Action! cartridge compiler does not accept it.

## Language Contract

The initial grammar is:

```text
const-declaration := CONST [ scalar-type ] const-entry { "," const-entry }
scalar-type       := BYTE | CHAR | CARD | INT
const-entry       := identifier "=" constant-expression
```

`CONST` is recognized contextually at the beginning of a declaration. This
avoids assigning an original Action! token number to an actionc-only construct
and avoids unnecessarily reserving `CONST` in other identifier positions.

Constants may be declared globally or in a routine's declaration section. They
participate in the ordinary, case-insensitive symbol namespace and follow the
same local-before-global lookup order as other symbols. A local constant may
shadow a global constant, but duplicate declarations in one scope are errors.

An entry may refer to constants that are already visible at its declaration.
Entries in the same comma-separated declaration are processed from left to
right. Forward references are deliberately excluded from the first version;
dependency graphs and cycle diagnostics can be added later.

A constant:

- has the scalar type inferred by normal Action! expression typing when the
  declaration omits `scalar-type`;
- otherwise has the declared scalar type, which applies to every
  comma-separated entry and behaves exactly like an explicit outer cast;
- can still use explicit casts such as `CARD(256)` inside its expression;
- has no storage address and emits no bytes;
- is a value, never an lvalue;
- cannot be assigned to or passed to address-of;
- is replaced by its typed value before executable NIR.

The initial constant-expression subset contains:

- numeric and character literals;
- earlier visible constants;
- `BYTE`, `CHAR`, `CARD`, and `INT` casts;
- parentheses and unary `+`/`-`;
- `+`, `-`, `*`, `/`, `MOD`, `LSH`, `RSH`, `AND`, `OR`, and `XOR`.

Calls, strings, variables, array access, fields, pointer operations,
address-of, dereference, and the current-location `*` value are rejected.
Arithmetic uses the existing Action! typing and wrapping rules. Division or
modulo by zero is a compile-time diagnostic.

Legacy `DEFINE` remains unchanged. In particular, it continues to support
textual type aliases, directive macros, and machine-byte macros that are not
constant expressions.

## Architecture

SemIR owns the meaning and value of `CONST`. The semantic model records a
stable symbol ID, inferred type, value, declaration scope, and source span.
Every consumer queries that fact instead of reparsing source text.

NIR receives typed literals. It must not gain an executable `Const` operation,
string name lookup, or another constant evaluator. MIR6502 therefore sees the
same ordinary literal values as the classic backend.

The classic backend currently consumes some AST expressions directly. Until
that path is fully replaced, it must query the semantic constant facts rather
than introduce a backend-specific evaluator.

## Implementation Slices

### Slice 1: Syntax and AST

- Add `ConstDecl` and `ConstEntry` AST nodes.
- Recognize contextual `CONST` in global and routine declaration positions.
- Parse the optional `BYTE`, `CHAR`, `CARD`, or `INT` declaration type.
- Parse each initializer with the ordinary expression parser.
- Add parser tests for valid expressions, multiple entries, case-insensitive
  spelling, malformed declarations, and unchanged `DEFINE` behavior.

### Slice 2: Semantic Facts and Evaluation

- Add `SymbolClass::Const` and constant symbol subjects.
- Store canonical typed constant facts by `SymbolId`.
- Bind and evaluate entries in source order.
- Apply a declared type with normal scalar cast, wrapping, and truncation
  semantics to every entry in the declaration.
- Diagnose non-constant constructs, invalid use, duplicate names, unavailable
  forward references, and division/modulo by zero.
- Preserve readable constant declarations in SemIR.

### Slice 3: NIR Boundary

- Lower resolved constant references to typed literal values.
- Skip declaration metadata when building executable blocks.
- Tighten verification and add a regression proving constants do not survive
  as executable NIR metadata or unresolved names.
- Update focused SemIR and NIR fixtures.

### Slice 4: Backend and Context Coverage

- Make constants work in ordinary expressions, initializers, array bounds,
  absolute addresses, `SET`, system routine addresses, conditions, and loop
  bounds.
- Use the same semantic facts in legacy/classic, modern/classic, and
  modern/MIR6502 configurations.
- Compare emitted bytes with equivalent literal-only programs.

### Slice 5: Inline Assembler and Documentation

- Export visible numeric constants into inline-assembler symbol resolution.
- Respect declaration order and local/global scope.
- Keep existing numeric `DEFINE` support intact.
- Document `CONST` in the syntax and name-resolution references.

## Validation

Each semantic or IR slice runs its focused tests. Before completion run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Required regression families include:

- scalar type inference and explicit casts;
- wrapping arithmetic, shifts, and signed values;
- division and modulo by zero;
- declaration order, shadowing, and duplicate symbols;
- illegal assignment and address-taking;
- global and local constants;
- every supported constant-use context;
- classic and MIR6502 output parity with literal expressions;
- inline assembler visibility;
- unchanged legacy `DEFINE` parsing and expansion.

## Deferred Extensions

The first implementation does not include forward references, address-valued
or relocatable constants, `SIZEOF`, constant strings or arrays, compile-time
functions, or named MADS equates in generated listings. These can build on the
semantic constant fact table without changing the source contract above.
