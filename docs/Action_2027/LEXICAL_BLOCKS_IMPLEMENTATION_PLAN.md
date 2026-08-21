# Lexical Blocks Implementation Plan

Plan status: implemented.

Slices 0 through 6 landed as commits `ddcfd0b`, `694a24c`, `9376801`,
`71a057d`, `ca9ab98`, `9fe9d5f`, and `dfc8f01`. The final artifact,
documentation, sample, and runtime-proof slice completes the eight-slice
rollout described below.

## Goal

Add explicit, nestable lexical blocks to the modern Action! profile. A block
may declare names which shadow outer names, and those declarations are visible
only until the matching block terminator.

The intended source form is:

```action
PROC Main()
  BYTE value

  value=1
  BEGIN
    CARD value
    value=1000

    BEGIN
      BYTE value
      value=2
    END

    ; CARD value is visible again here.
  END

  ; BYTE value is visible again here.
RETURN
```

`BEGIN` and `END` are contextual words, not lexer keywords. They receive no
Action cartridge token IDs and remain legal identifier spellings in existing
contexts such as `Begin()`, `End()`, `begin=1`, or declarations named `END`.
The parser commits to block syntax only for a bare marker at a statement
boundary. The initial grammar requires each marker to occupy its own physical
source line; this gives deterministic parsing without making newlines general
Action tokens.

The first implementation is an Action 2027 feature. The profile-neutral parser
may represent the syntax, but compatibility-profile semantic analysis rejects
it explicitly.

## Language Contract

### Scope

- A routine body remains the root local scope.
- Every explicit `BEGIN`/`END` creates exactly one child lexical scope.
- Nested blocks may shadow parameters, routine locals, outer block locals,
  globals, module members introduced with `USE ALL FROM`, and built-ins.
- A duplicate spelling in the same block is an error, case-insensitively.
- Sibling blocks may independently declare the same spelling.
- After `END`, lookup resumes in the parent scope.
- An identifier used outside the block cannot resolve to a declaration from
  the completed block.
- `IF`, `ELSEIF`, `ELSE`, `WHILE`, `DO`, and `FOR` bodies do not implicitly
  create source scopes. An explicit block is required when branch- or
  loop-local declarations are wanted. This avoids changing existing Action!
  name resolution merely because a control-flow construct is present.

### Declarations

The block begins with a declaration prefix followed by executable statements:

```text
lexical-block := BEGIN block-declaration* statement* END
```

The first complete milestone supports the ordinary semantic declaration
classes already accepted as routine locals:

- scalar and pointer variables;
- arrays;
- `CONST`, including `CONST REAL` in the modern profile;
- `TYPE` and `RECORD`.

Declarations after the first executable statement receive a focused
"block declarations must precede statements" diagnostic. `PUBLIC`, `EXTERNAL`,
`INCLUDE`, `SET`, nested routines, and module declarations are not legal in a
block.

Block-local `DEFINE` is deferred. `DEFINE` affects parsing as textual syntax as
well as semantic lookup, while the current parser keeps one compilation-wide
define environment. It must not be made apparently lexical while its expansion
still leaks past `END`. A later extension may add push/pop parser define
environments and then admit `DEFINE` as a block declaration.

Declaration initializers retain their current Action! storage meaning. This
feature does not turn an absolute address declaration into a runtime value
initializer and does not add constructors or block-entry initialization.
Existing declaration-class source-order rules remain in force; the block body
sees the completed declaration prefix.

### Control flow and lifetime

- `RETURN` crosses any number of lexical blocks and exits the routine.
- `EXIT` crosses lexical blocks and still exits the nearest enclosing loop.
- A lexical block is not a loop and does not consume `EXIT`.
- There are no block values, closures, destructors, deferred actions, or nested
  routines.
- Taking the address of a block local is legal. The pointer may outlive source
  visibility because Action! locals use static routine storage, not a machine
  stack.
- Initial block locals receive distinct storage for the whole routine. The
  compiler does not reuse storage merely because two lexical lifetimes are
  disjoint. Address escape, aliases, calls, interrupts, inline assembler, and
  machine blocks make unconditional reuse unsafe. Proven storage coalescing is
  a separate optimization project.

## Compatibility Contract

- `BEGIN` and `END` stay `Ident` tokens and do not alter historical token IDs.
- Calls, assignments, declarations, fields, and type names using those
  spellings continue to parse as before.
- Legacy and original-compiler-compatible source gains no implicit scope at an
  existing `IF`, loop, or `DO` boundary.
- Compatibility mode diagnoses a parsed lexical block as requiring the modern
  profile; it never silently ignores the scope.
- Modern classic and MIR6502 backends must implement identical binding and
  storage behavior under both cart and standalone runtimes.
- The feature must not require named modules. A legacy-shaped single source
  file selected with the modern profile may use lexical blocks.

## Pre-Implementation Compiler Baseline

This section is retained as the historical baseline used to design the slices;
it no longer describes the implemented compiler.

The implementation order is constrained by the following current shapes:

- `Routine.locals` contains declarations only from the prefix immediately
  following a routine header. `Stmt` has no declaration-bearing block form.
- `parse_statement_list_until` treats a declaration start as a body boundary,
  so nested declarations currently terminate statement parsing rather than
  becoming statements.
- Semantic analysis creates builtin, global/module, and routine scopes only.
  All statements nested under control flow are analyzed with the unchanged
  routine `ScopeId`.
- `ScopeKind` has no lexical-block variant, and `SemanticModel` records only
  routine-to-scope ownership.
- SemIR keeps all routine declarations in `SemRoutine.locals`; `SemStmt` has no
  block node carrying a child scope or nested declarations.
- SemIR and AST control-flow fact walkers have exhaustive statement matches
  which must learn that a lexical block has exactly the flow facts of its body.
- NIR already uses stable `LocalId` and `ParamId` in executable places, but its
  lowerer constructs and looks up those IDs through several case-folded name
  maps. Two visible semantic symbols named `value` cannot safely reach that
  boundary today.
- Local aliases, storage types, inline-assembly relocations, effect regions,
  array bases, and initializer relocations also contain name-based lookup paths
  in the NIR lowerer.
- MIR6502 and materialization are fundamentally ID-based once NIR is correct,
  but debug names and some map/listing construction assume one local spelling
  per routine.
- The classic backend projects SemIR back to an AST and uses local names as
  storage keys. It must flatten nested declarations with collision-free
  internal link names; flattening two source spellings unchanged would merge
  their storage.
- Modern classic compilation still uses the original AST directly for a
  non-module program without native REAL. Any program containing a lexical
  block must be routed through SemIR so codegen never re-resolves source scope.

These are general identity migrations, not reasons to alpha-rename source
symbols in executable NIR or to teach a backend how lexical lookup works.

## Architectural Shape

```text
source BEGIN/END
        |
        v
AST: LexicalBlockSyntaxId + declarations + nested statements
        |
        v
semantic model: child ScopeId, parent chain, stable SymbolIds
        |
        v
SemIR: explicit lexical block with resolved declarations and references
        |
        +-----------------------------+
        |                             |
        v                             v
NIR: flatten body, assign           classic projection: hoist storage,
SymbolId -> LocalId, no             assign collision-free link names,
executable scope operation          flatten executable body
        |                             |
        v                             v
MIR6502/emission                   classic emission
```

SemIR owns every source-language decision: block nesting, visibility,
shadowing, declaration legality, and the symbol selected at each use. NIR sees
only distinct storage IDs and normalized control flow. MIR6502 and emission
must not compare source names to recover scope.

## Proposed Representations

Names are illustrative; stable identity and ownership are the requirements.

### AST

```text
LexicalBlockSyntaxId(u32)

Stmt::LexicalBlock {
    syntax_id: LexicalBlockSyntaxId,
    declarations: Vec<Decl>,
    body: Vec<Stmt>,
    span: Span,
}
```

The parser assigns deterministic preorder IDs within each source unit. The
semantic key includes source/module identity so IDs from different named
module files cannot collide.

### Semantic model

```text
ScopeKind::LexicalBlock

SemanticLexicalBlock {
    syntax_id,
    scope: ScopeId,
    parent: ScopeId,
    routine: SymbolId,
    depth,
    ordinal,
    span,
}
```

Block-qualified symbol identities include the routine and lexical path. Two
block-local record types with the same display name must not share a
`qualified_name`, record-layout key, or field table. `LookupStage::Local`
applies to both routine and lexical-block scopes.

### SemIR

```text
SemStmt::LexicalBlock {
    scope: SemLexicalScopeRef,
    declarations: Vec<SemDeclaration>,
    constants: Vec<SemConst>,
    body: Vec<SemStmt>,
    span: Span,
}
```

The declaration symbols and all uses in the body carry `SymbolId`. The SemIR
printer retains the nested shape so shadowing is reviewable. Constants remain
semantic metadata and lower to typed literal values at executable uses.

### NIR and MIR6502

No executable `BeginScope` or `EndScope` operation is added. Before lowering a
routine body, NIR recursively inventories every storage-bearing declaration in
deterministic source order and assigns one `LocalId` to each semantic
`SymbolId`. Nested bodies then lower normally.

`NirLocal.name` remains display/debug metadata. Duplicate display names are
legal; executable references use `LocalId`. If a backend or artifact needs a
unique textual label, it derives one from routine identity, lexical ordinal,
and `LocalId` rather than changing source binding.

The NIR verifier continues to require every local place to reference a declared
`LocalId`. It must not require local display names to be unique. No lexical
metadata operation is permitted inside verifier-clean executable blocks.

## Implementation Slices

Each slice is independently testable and should be committed separately.

### Slice 0: Contract and Compatibility Baseline

- Add parser regressions proving `BEGIN` and `END` remain ordinary identifiers
  in calls, assignments, declarations, fields, and named types.
- Record the selected contextual grammar, modern-profile gate, declaration
  prefix rule, shadowing rules, and static-storage lifetime in this document.
- Inventory every AST/SemIR statement walker and every name-keyed local lookup
  below SemIR.
- Add a compile-only baseline for representative routine locals, arrays,
  aliases, records, inline assembler, native REAL, and both backends before
  changing identity plumbing.

Exit criterion: the syntax and compatibility decisions are fixed, and the
identity migration has an explicit test inventory.

Suggested commit: `language: establish lexical block contract`.

### Slice 1: Contextual Syntax and AST Shape

- Parse bare, line-delimited contextual `BEGIN` and `END` markers without
  extending `Keyword` or `action_token_id`.
- Add `LexicalBlockSyntaxId` and `Stmt::LexicalBlock`.
- Parse a declaration prefix separately from the executable body.
- Diagnose missing `END`, stray `END`, declarations after statements, illegal
  declaration classes, and top-level blocks.
- Preserve nested blocks and precise full-block spans.
- Update generic AST visitors, source validation, constant materialization,
  source metrics, and flow walkers to recurse through the new node.
- Until semantic support lands, issue one explicit modern-feature diagnostic
  rather than letting either backend see an unresolved block.

Tests:

- empty, nested, and control-flow-contained blocks;
- contextual identifier compatibility;
- malformed delimiter and declaration-order recovery;
- parser snapshots showing declarations belong to the block rather than the
  routine prefix.

Exit criterion: syntax is losslessly represented and all old source parses
unchanged, but code generation remains deliberately gated.

Suggested commit: `parser: add contextual lexical block syntax`.

### Slice 2: Semantic Scope Tree and Binding

- Add `SemanticOptions::lexical_blocks`, enabled by the modern profile only.
- Add `ScopeKind::LexicalBlock` and treat it as `LookupStage::Local`.
- Record `SemanticLexicalBlock` facts keyed by stable syntax/source identity.
- Allocate a child scope, analyze the declaration prefix there, and analyze the
  body with that child scope.
- Generate block-qualified canonical and display identities for every local
  symbol, including legacy-shaped modern source and named modules.
- Keep module-alias resolution working by following parent scopes to the
  owning module.
- Reject lexical blocks in compatibility mode with one focused diagnostic.
- Preserve the current rule that ordinary control-flow bodies do not create
  scopes.

Tests:

- same-block duplicates fail case-insensitively;
- nested and sibling shadowing produce distinct `SymbolId` values;
- parameter, routine-local, global, imported, and builtin shadowing;
- references after `END` bind to the parent or report undefined;
- block-local types and records have distinct identities and field layouts;
- `EXIT` and `RETURN` validation is unchanged across a block boundary.

Exit criterion: every identifier inside a valid block resolves to the intended
stable declaration, independent of backend support.

Suggested commit: `semantics: bind explicit lexical scopes`.

### Slice 3: SemIR Ownership and Control Flow

- Add the structured SemIR lexical-block form with scope, declarations,
  constants, body, and span.
- Lower each AST block using its recorded child `ScopeId`; never rediscover the
  scope from a name or source span.
- Update SemIR printers to display nested scopes, symbol IDs, declarations, and
  shadowed references readably.
- Teach control-flow facts that a block inherits `may_continue`, `may_return`,
  `always_returns`, `may_exit_loop`, and loop depth from its body without
  adding a loop.
- Recurse through blocks in effects, standalone validation, native-REAL scans,
  source materialization, and all other SemIR visitors.
- Add an explicit backend gate until the next two slices support the selected
  backend.

Exit criterion: SemIR completely owns lexical meaning and has no unresolved
block-local name; flow facts match the equivalent unwrapped body.

Suggested commit: `semir: preserve lexical block ownership`.

### Slice 4: Stable Local Identity and NIR Lowering

- Replace name-keyed parameter/local maps in the NIR lowerer with maps from
  semantic `SymbolId` to `ParamId`/`LocalId`.
- Recursively inventory block storage before lowering executable statements.
- Convert local aliases, array bases, storage types, initializer relocations,
  inline-assembler targets, machine effects, and storage regions to use the
  resolved semantic symbol identity.
- Lower a lexical block by lowering its body in place. Do not emit metadata
  operations or force a NIR basic block merely for a source scope boundary.
- Permit duplicate `NirLocal.name` display strings while requiring unique
  `LocalId` values.
- Audit optimizers, verifier, printer, storage analysis, promotion, and
  MIR6502 lowering for any remaining executable name lookup.
- Keep all block locals in distinct routine storage; do not add lifetime-based
  slot reuse.

Tests:

- NIR fixture with three shadowed locals of the same spelling and three
  distinct local IDs;
- assignments before, inside, and after nested blocks target the correct IDs;
- no executable lexical metadata appears in printed NIR;
- inline assembler and address-of operations select the inner local;
- verifier rejects undeclared IDs but accepts duplicate display names.

Required checks for this and every later NIR-changing slice:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Exit criterion: MIR6502 compiles scalar block locals without consulting source
names, and verifier-clean NIR is no weaker.

Suggested commit: `nir: lower lexical locals by stable identity`.

### Slice 5: Classic Backend Projection

- Recursively collect block declarations into the projected routine storage
  prefix.
- Build one projection-name table keyed by semantic `SymbolId`. Keep an
  unchanged source spelling where unambiguous; generate a deterministic
  collision-free internal name for shadowed symbols.
- Rewrite projected declarations, type references, expressions, lvalues,
  initializer targets, machine items, inline-assembler relocations, and native
  REAL facts through that table.
- Flatten the executable block body after declarations have been hoisted. The
  classic backend must never perform lexical lookup.
- Route any classic program containing a lexical block through SemIR even when
  `CodegenSource::Ast` is selected, just as named modules and native REAL
  already force semantic projection.
- Preserve source display names and lexical ordinals in maps/listings while
  keeping internal storage keys unique.

Tests:

- classic and MIR6502 produce the same observable results for nested shadows;
- cart and standalone classic modes allocate distinct addresses for distinct
  block locals;
- block-local native REAL values use distinct hidden/storage facts;
- MADS listings have deterministic, collision-free symbols and reassemble to
  the same bytes.

Exit criterion: scalar lexical blocks work in all four backend/runtime pairs.

Suggested commit: `codegen: add classic lexical block parity`.

### Slice 6: Complete Declaration Surface

- Enable block-local arrays, pointers, `VOLATILE` storage, absolute aliases,
  local-storage aliases, native REAL values, `CONST`, `TYPE`, and `RECORD`.
- Preserve array decay, pointer/record typing, field IDs and byte offsets, and
  initializer relocation identities through SemIR and NIR.
- Ensure two same-named block-local types have different record identities and
  cannot cross-assign accidentally.
- Ensure a block-local constant becomes a typed literal below SemIR and cannot
  leak as metadata or a string name.
- Resolve analyzed inline assembler against the innermost visible local and
  retain conservative effects for pointers, calls, absolute memory, and
  machine blocks.
- Keep block-local `DEFINE` explicitly unsupported until parser expansion can
  be scoped honestly.

Tests:

- arrays and indexed writes in sibling blocks with the same spelling;
- record/type shadowing and field layout;
- pointer address escape and dereference after the block through an outer
  pointer;
- native REAL copy/arithmetic in a nested block;
- volatile repeated accesses remain observable;
- absolute and alias-backed locals retain current addresses.

Exit criterion: every supported routine-local declaration class works inside a
lexical block under both backends.

Suggested commit: `language: complete lexical block declarations`.

### Slice 7: Artifacts, Documentation, and Runtime Proof

- Give SemIR, NIR, maps, and source listings readable lexical paths such as
  `Main::block2::value` while executable identity remains numeric.
- Add `samples/lexical-blocks.act` demonstrating nested shadowing, a
  branch-local explicit block, block-local types, and address escape.
- Update `NAME_RESOLUTION.md` with the lexical lookup chain and explicit rule
  that control-flow bodies alone do not introduce scopes.
- Add user-facing syntax and compatibility notes to the Action 2027 language
  documentation.
- Add a VM fixture that writes distinct results from outer, nested, and sibling
  declarations to fixed memory.
- Run the fixture under modern classic/MIR6502 and cart/standalone.
- Compile the complete sample, Toolkit, TN, embedded-module, and runtime fixture
  sets to detect accidental contextual-word or scope regressions.

Validation:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
cargo test --features experimental-named-modules
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml
```

Exit criterion: the feature is documented, inspectable in artifacts, and has
runtime parity across supported backend/runtime combinations.

Suggested commit: `docs: finish lexical block rollout`.

## Validation Matrix

| Boundary | Required proof |
| --- | --- |
| Lexer | `BEGIN`/`END` remain identifiers and historical token IDs do not move |
| Parser | nested structure, declaration prefix, delimiter recovery, stable syntax IDs |
| Semantic | parent-chain lookup, shadowing, duplicates, out-of-scope errors, modern gate |
| SemIR | explicit block ownership, resolved symbol IDs, correct flow/effect recursion |
| NIR | distinct `LocalId`s, no executable scope metadata, verifier-clean lowering |
| MIR6502 | distinct frame slots and no SemIR/name lookup |
| Classic | deterministic hoisting and collision-free projection names |
| Inline assembler | innermost symbol identity and conservative effects |
| Artifacts | readable, unambiguous lexical storage names |
| Runtime | classic/MIR6502 x cart/standalone observable parity |
| Compatibility | existing identifiers and control-flow scopes unchanged |

## Deferred Work

- implicit scopes for every control-flow body;
- declarations interleaved with executable statements;
- block-local `DEFINE` with a scoped parser expansion environment;
- block expressions or values;
- nested routines, captures, or closures;
- destructors, cleanup handlers, or `defer`;
- automatic/runtime stack allocation;
- reuse of storage between disjoint lexical lifetimes;
- debug-info ranges describing exact live program-counter intervals.

These can build on explicit lexical scope identity later. None should be
smuggled into the initial feature through backend name mangling or heuristic
storage reuse.

## Completion Criteria

The lexical-block feature is complete when:

- a modern program may nest explicit `BEGIN`/`END` blocks and shadow outer
  declarations predictably;
- every use is resolved to a stable semantic symbol before backend lowering;
- verifier-clean NIR identifies locals only by `LocalId`, with names retained
  solely for display;
- classic and MIR6502 produce the same results in cart and standalone modes;
- compatibility source using `BEGIN` or `END` as ordinary names remains valid;
- unsupported declaration forms receive focused diagnostics;
- all required repository and VM checks pass; and
- documentation clearly distinguishes lexical visibility from static Action!
  storage lifetime.
