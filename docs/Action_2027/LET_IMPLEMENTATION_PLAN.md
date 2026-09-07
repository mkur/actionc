# Immutable runtime LET bindings

Status: complete within the initial type/backend capability matrix below.

## Contract

`LET [type] name=expression` introduces an immutable runtime binding in modern
Action!. One binding per statement; an initializer is mandatory. Existing
typed variable declarations remain mutable and CONST remains compile-time.
LET initializers are executable, not static declaration initializers.

The initializer executes once whenever control reaches the statement. Infer
its existing canonical expression type, or apply ordinary assignment
conversions to the explicit type. An annotation does not widen operands:
`LET LONGCARD n=a*b` differs from `LET n=LONGCARD(a)*b` for narrow a and b.
Enum identity and pointer/callable types are preserved. LET values are not
compile-time constants even when their initializers are literal.

LET is permitted directly in routine and explicit BEGIN/END statement lists,
including after executable statements. Control-flow arms and loop bodies need
an explicit block. The name becomes visible after its initializer and expires
at the end of the containing routine/block. Sequential LET shadowing is legal,
including shadowing ordinary variables, parameters, types and module aliases.
The initializer sees the previous binding; earlier references keep their
original meaning. Ordinary same-scope declaration duplication remains illegal.

Assignment, compound assignment and FOR induction writes to a LET are errors.
Taking its address, creating storage aliases to its home, or referencing its
home from machine/assembly blocks is initially rejected. An immutable pointer
binding does not make its pointee immutable. Reading volatile memory captures
a value; subsequent binding reads do not reread the original memory.

Initial coverage is scalar integers, enum values, REAL and typed data/callable
pointers within each backend's existing capabilities. Whole records/arrays,
strings as owned values, globals, deferred initialization, LET MUT, destructuring
and expression-form LET/IN are excluded. LET follows existing target activation
and storage rules; it does not add Atari recursion/reentrancy or stack storage.

## Architecture

AST retains a distinct LET statement and stable syntax identity. Semantic
analysis introduces a fresh child lexical scope for the remainder of a
routine/block statement list. The initializer is analyzed in its parent scope.
This reuses immutable scope chains instead of re-resolving earlier references
against a symbol table modified by later shadowing.

Reuse existing read-only place access with explicit immutable-binding symbol
facts, and shared assignment conversion checks. SemIR expresses the resolved
scope/storage plus a runtime initialization assignment, with no static
initializer on that storage. Source writes remain prohibited; compiler-owned
initialization is distinct from an assignable source place.

NIR reuses local IDs, typed computation, stores, loads and existing control flow.
It must not gain an executable LET instruction, source lookup, or lexical
metadata. Initialization remains at the source execution point, including
inside loops. Classic projection collects storage and preserves the ordered
initialization. MIR6502 receives only existing normalized operations.

Immutability does not prove initializer purity, permanent read-only storage,
or safe motion across calls/volatile accesses. Reuse verified optimization
passes and conservative effects. Do not introduce LET-specific optimizers.

## Commit slices

1. Record the contract; add contextual syntax/AST and traversal coverage with
   focused diagnostics until semantic support is ready.
2. Implement sequential binding scopes, inference/conversions, read-only and
   escape checks, plus resolved SemIR and ordinary NIR lowering.
3. Prove classic/MIR6502 backend and runtime parity, full supported type
   coverage, effects and shadowing; fix general integration gaps if uncovered.
4. Complete runtime/optimization regressions, public docs, examples and final
   acceptance. Commit each verified major slice independently.

## Required validation

- Parser: inferred/annotated, qualified types, malformed syntax, identifier
  compatibility (`Let()`, `let=1`), normal statement separators.
- Semantics: modern gate, exact types, source-order visibility, repeated LETs,
  self-reference with/without an outer binding, nested blocks/modules, invalid
  placement, all write/escape paths, compile-time-only contexts.
- IR: distinct stable storage IDs, initializer in parent scope, explicit
  runtime store before reads, no static initialization or unresolved names.
- Execution: counted initializer calls, skipped branches, repeated calls/loop
  entries, volatile snapshots, pointer pointee writes, enum/REAL/wide values,
  narrow wrap-before-widen, and baseline/optimized equivalence.
- Compiler suite, NIR snapshots and sweep as required by AGENTS.md; locked VM
  suite on cart/standalone and both backends within their existing capability
  matrix. Native targets receive lowering/ABI canaries, not execution claims.

## Progress

- Contract and contextual syntax are committed. Parser/AST traversal coverage
  preserves ordinary LET identifiers, accepts inferred and annotated bindings,
  and diagnoses malformed initializers. Semantic use is explicitly gated until
  the next slice. Compiler tests, unchanged NIR snapshots and all 37 sweep
  fixtures pass.
- Sequential semantic scopes, immutable symbol/place facts, shared assignment
  conversion checks and SemIR-to-NIR lowering are implemented. Classic projection
  and MIR6502 execute the basic binding/effect fixture on both runtimes without
  new backend operations. Ten semantic/IR tests and the four-way runtime check
  pass; the compiler suite, unchanged snapshots and 37 NIR fixtures pass.
- Static initializer and bound consumers reject evaluated LET dependencies;
  unevaluated layout queries remain legal. The existing typed-place traversal
  is shared with embedded-array validation. The broad corpus now includes one
  additional positive runtime fixture (329 positive roots).
- Extended coverage includes eleven semantic/IR tests and five VM tests across
  baseline/optimized execution and supported backend/runtime combinations.
  It covers unused effectful initialization, REAL, enum CASE, record/array
  pointee writes, callable signatures, wide arithmetic, annotation versus
  operand widening, module aliases and native 68k/65816 lowering. Bare routine
  initializers now receive a value-expression diagnostic instead of producing
  an incomplete semantic model.
- A new lowered/optimized NIR snapshot pair proves distinct homes, executable
  initialization and retained volatile effects when unused storage disappears.
  Existing snapshots are unchanged; the NIR sweep has 38 positive fixtures and
  the broad fixture corpus has 330 positive roots. No new NIR or MIR operations,
  optimizer passes or target strategies were introduced.
- Existing capabilities remain explicit: classic indirect calls accept zero
  arguments, Atari wide integers need MIR6502, and REAL uses the OS floating-point
  package on either runtime. These are not LET-specific restrictions. The full
  locked VM suite passes 136 tests; focused LET tests also pass after diagnostic
  hardening. The compiler suite passes 2,845 tests, with all 38 NIR sweep fixtures
  and snapshot checks passing.
- Public syntax, name-resolution, semantic-invariant and NIR-boundary documents
  describe the completed contract. `samples/let-bindings.act` demonstrates
  immutable initialization, sequential shadowing and loop-entry initialization;
  the sample build matrix covers classic/MIR6502 with cart/standalone runtime.
