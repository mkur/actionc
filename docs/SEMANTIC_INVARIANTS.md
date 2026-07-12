# Semantic Invariants

This note records language-level invariants that `actionc` should preserve as
we move from AST codegen toward semantic IR and later TAC/SSA.

These are not optimization preferences. They are semantic rules or strong
working assumptions about Action! behavior. If a future probe contradicts one,
update this file and the corresponding tests before changing compiler behavior.

## Source-Order Visibility

Action! user symbols are not forward-visible.

An identifier use can bind only to:

- a resident/built-in/predefined symbol available before compilation starts; or
- a user declaration already encountered earlier in source order.

This applies to ordinary variables, arrays, `DEFINE`, `TYPE`, `RECORD`, `PROC`,
and `FUNC` names.

Consequences:

- a routine cannot call a later routine unless that routine was already
  introduced by some valid earlier declaration form;
- a routine cannot use a global variable declared later;
- a routine cannot use a `TYPE` or `RECORD` declared later;
- a source-order resolver must not pre-scan all routines and make them visible
  before their declarations.

This is a key difference from a multi-pass modern compiler. `actionc` may later
offer modern extensions, but the compatible Action! semantic path should reject
forward user references.

Currently modeled predefined variables include:

- `color`
- `device`

## Lookup Order

Once a name is eligible by source order, ordinary identifier lookup uses the
Action! order documented in `docs/NAME_RESOLUTION.md`:

1. local routine scope;
2. global scope;
3. resident-library/built-in scope;
4. undefined-symbol error.

The resident library is not searched until user local/global symbols fail, so a
user symbol can shadow a resident-library name.

## Shadowing

Action! allows shadowing across different lookup stages:

- a routine local can shadow a global;
- a routine parameter can shadow a global;
- a global user symbol can shadow a resident/predefined symbol;
- a routine local can shadow a resident/predefined symbol.

Action! rejects duplicate declarations in the same scope. This includes
duplicate global declarations and duplicate routine-local declarations.

Each scope has one symbol namespace. Variables, arrays, parameters, `DEFINE`,
`TYPE`, `RECORD`, `PROC`, and `FUNC` names all conflict when declared with the
same normalized name in the same scope. `SymbolClass` describes what a resolved
symbol is; it does not create a separate lookup namespace.

This behavior is confirmed by the dedicated probes in
`surveys/probes/original-compiler/shadowing/`.

## Symbol Binding

Every identifier use should eventually bind to exactly one semantic target:

- `SymbolId` for ordinary symbols;
- a record/type-relative field identity for field access;
- an explicit unresolved placeholder only after a diagnostic.

Codegen should not perform ordinary source-name lookup. It should consume
already-bound symbols, field descriptors, types, layout facts, and resident
library metadata.

## Symbol Class Versus Use Context

Name resolution chooses a symbol. Semantic validation then decides whether that
symbol class is legal in the current context.

Examples:

- a `TYPE` or `RECORD` symbol may be legal in a declaration but not as a value;
- a `PROC` symbol may be legal as a call target or routine-assignment target,
  but not as an ordinary scalar value;
- a variable may be legal as an expression or assignment target, but not as a
  type name.

This separation is important: binding should not silently skip a symbol just
because the current use is illegal. It should bind, then report the context
error.

## Typed Nodes

Typed semantic subjects are the authoritative representation for expression,
place, callable, and type-reference meaning.

The current analyzer still exposes `SemanticModel.expression_observations` as a
compatibility/debug projection of expression span, category, and type. This side
table must not drive SemIR lowering or code generation. It exists to keep older
tests and observability hooks working while the compiler migrates toward typed
semantic nodes and later TAC/SSA.

Consequences:

- semantic validation should consume `SemExpr`, `SemPlace`, `SemCallable`, and
  `SemTypeRef`, not re-derive meaning from observation rows;
- SemIR lowering should derive type and category from SemIR node structure,
  symbols, signatures, layout facts, and semantic types;
- SemIR tests should assert node-local type invariants, such as dereference
  pointees, address-of pointer types, array decay targets, field references, and
  call return types;
- new tooling may read `expression_observations`, but it must treat them as
  diagnostics/observability output, not as compiler authority.

## Scope Lifetime

The original compiler reuses local symbol-table storage between routines. That
is an implementation detail, not the semantic model `actionc` should expose.

Semantic analysis should keep a stable routine scope for every routine, with
stable symbol IDs. Original-compiler local table reuse matters for compatibility
observability, not for semantic identity.

## Control-Flow Scope

Action! control-flow constructs do not introduce source scopes.

This applies to:

- `IF`/`ELSE`/`FI`;
- `WHILE`/`DO`/`OD`;
- `DO`/`UNTIL`/`OD`;
- `FOR`/`TO`/`STEP`/`DO`/`OD`.

Names used inside these bodies resolve in the enclosing routine or global
scope. A `FOR` target is an ordinary assignment target resolved through normal
lookup, not a loop-local declaration.

Compiler-generated storage needed to implement a loop, such as cached end
values or step values, is not a source symbol. It must be represented as
generated codegen/layout storage or semantic temporaries, and it must not
receive a `SymbolId`.

## Field Resolution

Record/type fields are not ordinary global names.

For `base.field`, semantic analysis should:

1. bind `base` through ordinary name resolution;
2. determine the named record/type identity of `base`;
3. bind `field` inside that record/type layout.

Field binding uses a stable record-relative `FieldId`, not only the textual
field name. The descriptor records:

- the owning `TYPE`/`RECORD` symbol;
- the declared field name;
- the field type;
- the byte offset within the record layout.

This matters because two records may both contain `tag`, but `A.tag` and
`B.tag` are different semantic fields with different owners. SemIR field refs
should carry that field identity forward so layout/codegen does not have to
redo textual field lookup.

Semantic analysis also builds `SemanticLayoutFacts`. Record layout facts group
field layout entries by owning `TYPE`/`RECORD` symbol, preserving field ID,
name, type, offset, and total record size. Downstream code should consume these
facts instead of rebuilding record layouts from declaration text.

The shared semantic type model exposes record shape as `RecordType`:

- `name` is the source record/type name;
- `fields` is the resolved ordered field list with field ids, types, and
  offsets;
- `size` is the total byte size implied by the field layout.

SemIR `TYPE` and `RECORD` declarations should carry `RecordType` alongside the
lowered field nodes. Current codegen may still bridge through `ValueType`, but
future SemIR-native lowering should use `RecordType` for field layout and record
pointer reasoning.

`ValueType` still bridges named records and record pointers for the transitional
backends. Code that needs record family identity should use the record helpers
(`record`, `record_pointer`, `as_record_identity`, and `same_record_family`)
instead of matching `ValueTypeBase::Named` plus the raw pointer flag directly.

## Record Pointer Semantics

A source record value may be used where a matching record pointer is expected.
This is an Action!-style implicit address-of operation:

- `Pair POINTER p; Pair rec; p = rec` means `p = @rec`;
- `PROC Touch(Pair POINTER p); Touch(rec)` passes `@rec`;
- a different record family does not match, even if the fields are shaped the
  same.

Semantic IR represents this as explicit implicit-address lowering, preserving
the reason as `RecordToPointer`. This keeps downstream codegen from having to
rediscover the conversion from raw names and types.

Explicit address-of over record fields is typed by the field type:

- `@rec.tag` has type `BYTE POINTER` when `tag` is `BYTE`;
- `@rec.word` has type `CARD POINTER` when `word` is `CARD`;
- assigning either address to the wrong pointer type is rejected.

## Scalar Type Foundation

The canonical scalar semantic model is `ScalarType`:

- `BYTE`: 1 byte, unsigned;
- `CHAR`: 1 byte, unsigned;
- `CARD`: 2 bytes, unsigned;
- `INT`: 2 bytes, signed.

Existing `ValueType` remains the bridge used by the current analyzer and
codegen, but scalar decisions should route through the scalar model rather than
duplicated width/signedness tables.

## Array Semantics

Source arrays have an element type. For example, `BYTE ARRAY a(10)` has `BYTE`
elements, and `CARD ARRAY w(10)` has `CARD` elements.

Indexing an array produces an assignable element place:

- `a(i)` / indexed syntax over a `BYTE ARRAY` has type `BYTE`;
- indexing over a `CARD ARRAY` has type `CARD`;
- indexing over a pointer has the pointer pointee type.

An array name used in a pointer context decays to a pointer to its element type.
This is allowed only when the pointer pointee type matches exactly:

- `BYTE ARRAY b(4)` can pass to `BYTE POINTER`;
- `CARD ARRAY c(4)` can pass to `CARD POINTER`;
- `BYTE ARRAY` does not pass to `CARD POINTER`;
- `BYTE ARRAY` does not pass to `CHAR POINTER`.

Array parameters are array-like source symbols even though their runtime ABI is
a two-byte base pointer. Inside a callee, an array parameter may decay to a
matching element pointer in the same way as a normal array name.

Semantic IR represents array-name use in value/pointer context as explicit
array decay. The decay records the element type, pointer type, and whether the
array originated as global storage, routine-local storage, or a parameter. This
origin is a semantic/layout fact; it should let later codegen choose the right
addressing path without rediscovering array provenance from raw names.

`SemanticLayoutFacts` also records source array facts by symbol: element type,
derived pointer type, and origin (`Global`, `Local`, or `Parameter`). SemIR and
SemIR-native codegen should use those facts instead of re-deriving array shape
from symbol class and scope.

The shared semantic type model exposes this shape as `ArrayType`:

- `element` is the source element type;
- `length` is the declared constant bound when it is statically available;
- `pointer_type()` is the exact decay target type.

SemIR array declarations and array parameters should carry an `ArrayType`
alongside their existing element `SemType`. The element `SemType` remains the
bridge for current codegen, but future semantic-IR/native lowering should use
`ArrayType` when it needs array shape or decay information.

Plain scalar variables do not decay to pointers. A pointer can still be
assigned a `CARD` value as an explicit raw-address escape hatch.

## Evaluation Order

Action! expression evaluation order matters. Where probes show left-to-right
evaluation, semantic IR should preserve that order explicitly. This is
especially important for calls, assignment expressions, and expressions with
pointer/array side effects.

Modern optimization must not reorder effectful expressions unless the semantic
model can prove the reordering is safe.

## Runtime And Built-In Effects

Resident-library calls and machine-code blocks are not pure by default.

Semantic analysis should represent their effects conservatively unless
annotations or resident-library metadata say otherwise. Effects include:

- register clobbers/preserves;
- zero-page reads/writes;
- absolute memory reads/writes;
- OS/CIO calls;
- opaque unknown effects.

These effects are semantic facts for correctness first and optimization inputs
second.

## Compatibility Versus Modern Extensions

The compatible path should model Action! semantics, including restrictions such
as source-order visibility.

Modern profile may eventually add extensions, but they should be explicit and
reported as modern behavior. They should not silently change the default meaning
of Action! source.

## Current Implementation Gaps

Known gaps between these invariants and the current implementation:

- many identifier uses still carry strings or are re-looked-up downstream
  instead of being fully bound once in semantic analysis;
- the older AST codegen still rebuilds record and array layout facts from AST
  declarations;
- SemIR-native does not yet consume the full semantic binding/fact model.

These gaps should be closed slowly, with tests added before broad rewiring.
