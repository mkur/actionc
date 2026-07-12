# Name Resolution And Symbol Binding

This note records the name-resolution rules we currently believe Action! uses
and the direction `actionc` should take while moving toward semantic IR.

The goal is deliberately narrow: every identifier use should eventually point
to exactly one declaration, or to an explicit unresolved/error placeholder. This
note documents the rules before changing compiler behavior.

## Evidence Level

The following rule is documented in the Action! symbol-table material and is
also consistent with our VM symbol dumps:

1. search the local symbol table;
2. search the global symbol table;
3. search the built-in cartridge resident-library table;
4. if all fail, report an undefined symbol.

See `docs/ACTION_SYMBOL_TABLE.md`, especially the "Library Lookup Path"
section.

Everything below is our current working model unless marked as an open
question.

## Symbol Spaces

Action! appears to use one ordinary user-symbol space per visible scope, not
separate namespaces for variables, routines, types, and defines.

User-visible symbol classes include:

- `DEFINE`
- `TYPE`
- `RECORD`
- scalar variables
- array variables
- routine parameters
- `PROC`
- `FUNC`

`actionc` already models these categories as `SymbolClass`; name resolution
should choose the symbol first, then later context checks whether that symbol
class is legal at the use site.

Examples:

- a `TYPE` name may be legal in a declaration but illegal as a value expression;
- a `PROC` name may be legal as a call target but not as an ordinary scalar
  value;
- a variable may be legal as a value or assignment target but not as a type
  name.

## Scope Order

Inside a routine body, local scope wins over global scope. The local scope
contains parameters and declarations after the routine header, including local
`DEFINE`, `TYPE`, and `RECORD` declarations.

At global/module level, only the global scope is searched before the resident
library.

The resident library is searched only after user local/global lookup fails.
This means a user symbol can shadow a resident-library name.

## Global And Module Declarations

The Action! manual describes globals as:

- all `PROC` and `FUNCTION` names;
- all names before the first routine;
- names encountered between a `MODULE` keyword and the next routine.

For `actionc`, the important binding rule is that globals are available to
later routines through the global symbol table. We should preserve source-order
rules only where probes or the manual show that forward references behave
differently for a specific class.

Open question: whether every global class is forward-visible exactly like
routine names, or whether some declaration classes are only visible after their
source declaration.

## Routine Scope Lifetime

The original compiler reuses local symbol-table space between routines. The
monitor can see only the last compiled routine's locals. That is an
implementation detail of the original compiler.

For semantic analysis, `actionc` should still create a separate stable routine
scope for every routine. Symbol IDs should remain stable even if the original
compiler would discard a previous routine's local symbol-table entries.

## Defines

`DEFINE` names participate in ordinary symbol lookup. After binding, expansion
or interpretation is context-sensitive:

- a define used where a type is expected can act as a type alias, such as
  `DEFINE STRING="CHAR ARRAY"`;
- a define used in an expression may expand to source text or machine data,
  depending on parser context;
- nested `DEFINE` directives are invalid.

Semantic binding should record the define symbol used. It should not leave a
bare string lookup for codegen.

## Types And Records

`TYPE` and `RECORD` names bind like ordinary symbols, but their legal use is
context-sensitive.

Field names are different: they are resolved relative to the bound record/type
of the base expression, not through the ordinary local/global/library lookup
chain.

Example model:

```text
rec.field
```

Resolution steps:

1. resolve `rec` through normal identifier lookup;
2. determine the record/type identity of `rec`;
3. resolve `field` within that record/type layout.

Field binding should eventually point to a stable field ID or canonical field
descriptor, not just a field-name string.

## Resident Library

Resident library names are not ordinary RAM symbol-table entries. They are the
third lookup stage after local and global user symbols.

For `actionc`, this argues for modeling resident library entries as symbols in a
distinct built-in/library scope. They should have normal symbol IDs once bound,
but the binding should preserve that they came from the resident library.

This lets codegen choose a cartridge entry point or compatibility shim without
re-resolving the textual name.

## Machine Blocks

Names inside machine-code blocks can refer to user symbols and labels/routines.
They should use the same visible lookup chain unless probes show machine blocks
have special resolution rules.

Open question: whether all machine-block names are resolved at compile time
with identical local/global/library search order, especially for resident
library names and runtime helper labels.

## Routine Assignment

Action! permits assigning routines in some contexts, for example trampoline or
fixed-address routine patterns. Therefore routine names are not purely
call-only symbols.

Binding should still resolve a routine name to its routine symbol. A later
semantic legality check should decide whether the current use context permits
routine assignment.

## Desired `actionc` Invariant

Eventually, every identifier use in semantic IR should be one of:

- `Resolved(SymbolId)` with source span and use kind;
- `ResolvedField(record/type id, field id)` for record fields;
- `Unresolved(name, span)` only in error-tolerant IR after a diagnostic.

Codegen should not perform ordinary source-name lookup. It should consume bound
symbols, field descriptors, types, layout facts, and resident-library metadata.

## Immediate Next Step

Before changing lowering, add tests around the documented lookup order:

- local variable shadows global variable;
- user global shadows resident-library name;
- local define/type/record names bind before global names;
- type name in value context is bound but rejected by context validation;
- record fields resolve through the base type, not global scope;
- unresolved names produce one diagnostic and one explicit unresolved use.
