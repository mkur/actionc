# ML-style algebraic data types: implementation plan

Status: accepted; implementation in progress on `main`, 2026-09-08.
Inspected baseline: `ceecea1`. Public algebraic-type capabilities remain disabled
until their end-to-end acceptance gates below pass.

## 1. Objective and boundaries

Provide ML-style nominal sum/product types: typed constructors, variant values,
immutable pattern bindings, exhaustive matching, type parameters, and recursive
data through explicit pointers. Retain Action!'s `TYPE`, record-field syntax,
`LET`, and `CASE`/`WHEN`/`THEN`/`ELSE`/`ESAC` conventions.

This is not an untagged-union implementation and not an implicit-heap runtime.
Raw `UNION`, automatic boxing/allocation, garbage collection, ownership/borrowing,
generic routines, closures, GADTs, polymorphic variants, structural equality,
tuple/unit/never syntax, and DISTINCT wrappers are separate work. CASE remains
a statement; value-producing MATCH/CASE and LET destructuring are not required.
Ordinary record expressions, copies, value arguments, and returns are shared
infrastructure in scope; a new record-literal syntax is not required.

Modern classic and MIR6502 must ultimately support the common Atari subset with
both cartridge and standalone linking. Compatibility diagnoses the new features
explicitly. Existing scalar, ENUM, CASE and record behavior must not change.
Native 68k/65816 receive verified layout/call/frame lowering and capability
diagnostics, not a claim of executable support that those backends lack today.
Oscar64 porting, volatile-test porting and standalone error-screen work remain
deferred. No allocator or runtime licensing replacement is included.

Commit each verified major slice separately.
Keep incomplete capabilities internal and disabled in public profiles; do not
add temporary public command-line switches or silently lower variants to BYTE.

## 2. Inspected compiler foundations

| Area | Present today | Required extension |
| --- | --- | --- |
| `ast.rs`, `parser/enums.rs` | TYPE distinguishes records and enums; TypeRef supports qualified names, pointers and callables | Variant definitions, type applications, constructor expressions, typed patterns |
| `semantic/enums.rs`, `semantic/types.rs` | Nominal enum identities; records still have name-based type references; named routine result syntax exists | Stable aggregate identities and a finite recursive type graph; aggregate result checking |
| `semantic/ir.rs`, `semantic/subject.rs` | Scalar expressions, addressable record copies, sequential LET scopes | Aggregate values/initialization, constructor facts, pattern scopes and value captures |
| `semantic/case.rs`, `parser/case.rs` | Integer/enum intervals; no guards or payload patterns | Constructor patterns, usefulness/exhaustiveness, ordered guarded arms |
| `nir/facts.rs`, `nir/ir.rs`, `nir/lowerer.rs` | Structured layouts/storage, CopyBytes, scalar temp call results | Finite aggregate layout references, aggregate value temporaries and call/return operands, explicit fault control flow |
| `codegen/semir.rs`, `codegen/record_copy.rs` | Canonical record layout and overlap-safe copies; pointer-valued record fields are explicitly rejected | Shared pointer-field carriers and aggregate value/result projection |
| `mir6502/abi.rs`, `mir6502/materialize/abi.rs` | Typed scalar/pointer ABI, argument staging and helper effects | Aggregate argument/result passing without changing existing signatures |
| `mir68k/`, `mir65816/`, `target.rs` | Target layouts, native activation/frame/call planning | Aggregate result/frame canaries using target pointer sizes and alignments |

Relevant existing contracts:

- [ENUM and CASE design](ENUM_AND_CASE_DESIGN.md), including open enum byte values
  and the proposed ordered guard extension.
- [LET](LET_IMPLEMENTATION_PLAN.md), including immutable source places and
  executable, exactly-once initialization.
- [Record values and copies](../RECORD_INITIALIZERS_AND_ASSIGNMENT_IMPLEMENTATION_NOTE.md)
  and [embedded arrays](../EMBEDDED_RECORD_ARRAYS_IMPLEMENTATION_PLAN.md).
- [Native activation and ABI](../NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md).
- [Runtime faults](../ATARI_RUNTIME_ERRORS.md).

These components are foundations, not evidence that aggregate expressions,
generic types, pattern matching or aggregate function returns already work.

## 3. Accepted source contract

All examples in this document describe future syntax.

### 3.1 Definitions and constructors

```action
TYPE Event=VARIANT [
  NONE
  KEY [BYTE code]
  MOVE [INT x,y]
]

Event current

PROC Main()
  current=Event.MOVE(12,-3)
  LET saved=current

  CASE saved OF
  WHEN Event.NONE THEN
    PrintE("No event")
  WHEN Event.KEY(code) THEN
    PrintB(code)
  WHEN Event.MOVE(x,y) THEN
    DrawAt(x,y)
  ESAC
RETURN
```

- VARIANT is contextual after `TYPE name=`; it is not a cartridge keyword.
- A definition contains named alternatives, optionally followed by record-like
  payload fields in brackets. Whitespace separates alternatives; commas may
  separate alternatives as for ENUM. Duplicate alternative/field names are errors.
- Constructors are qualified by their type. A payload-free constructor is a
  value (`Event.NONE`); a payload constructor takes positional arguments in
  flattened field declaration order (`MOVE(x,y)`). Named arguments and taking
  a constructor's function pointer are deferred.
- Each declaration/instantiation is nominal. Equal tags/layouts do not make two
  types interchangeable. Imports preserve the defining identity. PUBLIC exports
  the type and its constructors, as for ENUM; validate the reachable payload-type
  interface instead of resolving private implementation names in the importer.
- Preserve Action!'s case-insensitive shared declaration namespace. A type and
  variable in the same scope need distinct names (`Event current`, not
  `Event event`); ADTs do not introduce a separate type namespace.
- Constructor arguments use ordinary assignment conversions to the declared
  field types. An INT/CARD argument expression retains its existing arithmetic
  width unless an operand is explicitly widened. No new arithmetic semantics.
- Evaluate arguments once, left to right. Capture aggregate arguments as values
  at their evaluation point, before later arguments can mutate their sources.
- Assignment captures its destination according to the existing evaluation-order
  contract. Construct the full RHS before compiler-generated destination writes;
  preserve user side effects and overlapping/self-referential source values.
  This is value semantics, not an interrupt-atomic update guarantee.
- Payload fields are not independently writable through the variant. Replace
  the whole value to change its constructor/payload. Pointer pointees remain
  mutable; matching is the mechanism for accessing payload values.
- Initial payloads are supported scalar/enum/REAL/data-pointer values and complete
  records/variants by value. Classic's LONGINT/LONGCARD restrictions remain.
  Records containing fixed arrays can be payloads. Direct ARRAY payload fields,
  bare array type arguments and owning array values are deferred, rather than
  quietly applying pointer decay where an inline value was requested.

### 3.2 Storage, validity and initialization

Initial representation, frozen in slice 0:

- One unsigned byte tag on every target; reserve tag **0 for invalid/uninitialized**.
  Assign constructors tags 1..255 in declaration order. Thus the first version
  allows **255 alternatives**, not 256. No explicit tag assignments, tag casts,
  tag-field lvalues, niche encoding or serialization ABI is introduced.
- A constructor's payload is laid out like a record. A common payload area starts
  after the tag, padded to the maximum payload alignment; total size is rounded
  to the variant alignment. Use TargetLayout and checked extent arithmetic.
  On packed 6502, Event above is five bytes; a Tree with INT and two pointers
  is seven bytes. Native layouts may have padding and wider pointers.
- Constructors initialize their active fields and zero remaining payload/padding
  bytes. Aggregate copies preserve the full extent using existing overlap-safe
  semantics. Bytewise equality is not a language operation on variants.
- Zero-filled storage is not a constructed value, even when the first alternative
  has no payload. Explicitly assign `Event.NONE` where that state is desired.
  Unused arena entries may remain invalid. Existing declaration/static versus
  runtime initialization rules are not reinterpreted.
- Establish invalid tag zero at the start of compiler-managed object storage
  lifetime: load-image initialization for Atari routine-static/global storage,
  activation entry for native automatic storage. Include variant tag slots in
  ordinary records/arrays; an invalid outer variant needs no inactive-field
  initialization. This new-type initialization rule prevents uninitialized native
  stack bytes from accidentally appearing constructed. It does not reset Atari
  static locals at every call or change old scalar/record initialization. Elide
  the tag clear only when a dominating full initialization makes it unobservable.
- Diagnose definitely unconstructed local reads when existing flow facts suffice.
  A variant-value read (copy source, LET initializer, argument, return or CASE)
  captures and validates its value unless reusable facts already prove validity.
  Validation examines active inline nested variants but **does not follow pointers**.
  Do not inspect inactive payload alternatives or promise dangling-pointer safety.
- Invalid/uninitialized tags fault before exposing payloads or executing a CASE
  arm, including ELSE. An external write must not become optimizer undefined
  behavior merely because source matching was exhaustive.
- Add a semantic/internal `InvalidVariant` fault, separate from its target code.
  Atari mapping: existing invalid-argument **Error 100**, through the
  established A/X/Y convention and defensive non-return guard. This is a new
  actionc use of that convention, not a historical cartridge variant error.
  Do not add a fatal-screen API or change standalone Error's DOS handoff.
- Initially reject direct VOLATILE variant objects, absolute/alias-backed variant
  declarations and raw byte-list variant initializers. Ordinary typed pointers
  remain explicit low-level access and receive value-read checks; arbitrary
  memory safety and concurrent mutation are not implied. A future checked binary
  decoding/FFI facility is separate from constructor syntax.

The reserved tag and validation policy are accepted ADT rules, not changes to
the existing ENUM rules. They avoid
silently inventing a valid constructor from default zero-fill. Reordering
constructors can change layout bytes; saving raw objects is not portable storage.

Static constructor initializers may later reuse the aggregate image builder when
all arguments are existing compile-time constants/relocations. Do not interpret
`Event value=expression` as a runtime declaration initializer: `=` already has
Action storage-binding meaning. Runtime assignment and LET suffice for the first
delivery; a typed static-constructor spelling is a separate syntax decision.

### 3.3 Pattern matching and immutable bindings

- Preserve `CASE expression OF` with one selector evaluation, source-order arms,
  no fallthrough and unchanged loop EXIT/routine RETURN rules.
- Capture a stable aggregate value, not just its tag or the address of a mutable
  object. If an arm/guard changes the original, current matching and bindings
  still refer to the captured value. This is shallow across pointers: it captures
  the pointer, not the pointed-to graph. Copy elision requires lifetime/effect proof.
- Start with one constructor pattern per WHEN. Require exact arity and selector
  type. Each payload position is a fresh binder or `_` discard. Binding the same
  name twice is an error, not an equality test. Constructor names are not binders.
- Bindings are immutable arm-local values with LET's write/address/alias/machine
  access restrictions. Aggregate binders are immutable snapshots too, not hidden
  writable references. Their pointer fields do not make pointees immutable.
- The pattern introduces a child scope for its bindings, visible in its future
  guard and body only. Earlier/sibling arms are unaffected. Ordinary declarations
  and LET in bodies continue to require the existing BEGIN/END where applicable;
  scalar CASE arms do not silently gain a new declaration policy.
- Every valid constructor must be covered or there must be a final ELSE. Flat
  repeated constructors are errors before guarded/nested patterns are enabled.
  Invalid tags are a fault path, not another source constructor to cover.
- Reuse source return-flow analysis, but distinguish complete valid-value coverage
  from optional ELSE. An exhaustive variant CASE whose arms return can satisfy
  return checking; its fault path cannot fall through. Integer/enum CASE retains
  its current no-match and open-representation rules.

Nested by-value constructor patterns are a later slice:

```action
CASE wrapped OF
WHEN Wrapped.VALUE(ReadResult.FAILED(code)) THEN
  Report(code)
WHEN Wrapped.VALUE(result) THEN
  Handle(result)
WHEN Wrapped.NONE THEN
  Skip()
ESAC
```

Usefulness/exhaustiveness must then operate on patterns, not just top-level tags.
Support scalar literal subpatterns with existing typed-constant checks; use `_`
or a binder for full scalar-domain coverage. Do not implicitly dereference a
pointer subpattern. `CASE child^ OF` makes that memory access explicit.
OR-patterns, payload ranges and arbitrary computed expression patterns are deferred.

Guards follow the existing design:

```action
WHEN Event.KEY(code) IF code<>0 THEN
```

Match first, establish bindings, then evaluate the guard once. False continues
to the next arm with the original capture. Guarded arms do not count toward
exhaustiveness in the first implementation. ELSE is the final unguarded fallback;
`WHEN _ IF condition THEN` is a guarded catch-all, not a bare WHEN _ alias for
ELSE. Do not speculate calls, faults or volatile reads into unselected paths.

### 3.4 Function values and ABI

```action
TYPE ReadResult=VARIANT [
  END
  VALUE [BYTE value]
  FAILED [BYTE code]
]

ReadResult FUNC ReadNext()
  RETURN(ReadResult.VALUE(42))

PROC Main()
  LET result=ReadNext()
  Consume(result)
RETURN
```

Constructor expressions, ordinary record/variant places, LET, assignment,
arguments, CASE selectors and RETURN must compose as value consumers.
An aggregate value parameter is an independent value, not an implicit reference
to the caller's original storage. Mutation of ordinary parameters must not
modify the caller; explicit pointer parameters retain reference semantics.

Prefer caller-owned result storage and captured aggregate argument buffers.
The concrete initial Atari aggregate-result ABI uses a hidden destination pointer
as the first physical argument; subsequent physical operands follow the existing
argument planner. Aggregate arguments can travel as addresses to captured values,
with callee-owned copying where needed to preserve mutable parameter semantics.
Keep this physical lowering separate from source signature identity and semantic
argument order. Existing scalar/enum/pointer/SYS/external signatures are unchanged.

The result location must remain valid across nested calls. No single global
return buffer, no exposing callee-private storage, and no reuse of a live outer
call's captures for an inner call. Protect hidden destinations across calls using
the same capture/liveness machinery as other pointers. Handle ignored results,
callee fallthrough diagnostics, aliasing source/result places and return forwarding.
Direct-to-final-destination construction is an optimization, not the semantic
definition. Same result size does not imply callable type compatibility.

Native ABI planners choose their own hidden placement and invocation frame objects
using data-pointer width/alignment. Atari routine-static activation is unchanged;
aggregate returns and recursive data do not enable recursive/reentrant procedures.
Typed indirect aggregate calls need matching ABI/signature support before their
gate opens. Unknown foreign aggregate ABIs must be rejected, not guessed.

### 3.5 Recursion and type parameters

```action
TYPE Tree=VARIANT [
  EMPTY
  NODE [INT value Tree POINTER left,right]
]

; Final generic form, after the generic-type slice:
TYPE TreeOf<T> = VARIANT [
  EMPTY
  NODE [T value TreeOf<T> POINTER left,right]
]

TYPE Option<T> = VARIANT [NONE SOME [T value]]
TYPE Result<T,E> = VARIANT [OK [T value] ERROR [E error]]
```

- Permit self and mutually recursive types only when every layout cycle crosses
  a pointer. Predeclare TYPE identities in their type scope; reject incomplete
  by-value/array cycles with a useful cycle diagnostic. Do not broaden forward
  variable/routine visibility or introduce cyclic module imports.
- Pointers may refer to named objects or entries in fixed record/variant arrays.
  An arena allocator and iterative traversal belong in ordinary sample/library
  code. Pointer ownership, reclamation and NIL checks are explicit program concerns.
  An EMPTY sentinel is an ordinary object initialized by its constructor, not a
  magical null pointer or permanently read-only runtime singleton.
- Generic TYPE parameters range over complete supported value types, including
  pointers; not arbitrary array storage, values, routines or type constructors.
  Support generic records and variants using the same type-application mechanism.
- Use explicit applications initially: `Option<BYTE>`, `Result<CARD,IOError>`,
  `Option<BYTE>.SOME(42)`, `Option<BYTE>.NONE`. LET infers the resulting concrete
  type. Inferring omitted generic arguments and generic routines are deferred.
- Intern instances by definition identity plus canonical concrete type arguments.
  Insert recursive placeholders before resolving fields; share identical instances
  across call sites/import aliases. Build a finite graph, not recursively nested
  cloned Rust structures or mangled executable name strings.
- Reject recursive specialization that grows its type arguments, and impose
  documented depth/instance budgets with diagnostics. A pointer cycle does not
  itself make an infinite sequence of distinct generic instances acceptable.
- No runtime type dictionaries or automatic specialization of user routines.
  Resolve concrete layouts before SemIR/NIR lowering, reuse existing width helpers,
  and test deterministic output and bounded code/data growth.

Angle-bracket parsing is contextual to types and qualified constructor heads.
Handle nested applications and `TYPE X<T>=...` without changing ordinary `<`, `>`,
`>=` expression tokenization. A type-context token view may split a closing `>`
from `>=`; do not globally rewrite lexer behavior or invent a string-reparse path.

## 4. IR and implementation ownership

### Semantic layers

Add stable type-definition/instance and constructor IDs. Extend the existing
record/enum type facts deliberately rather than creating a second naming system.
Preserve nominal identity through imports, local shadowing, generic substitution,
callable signatures and both typed expression layers. Existing printable record
names may remain metadata; recursive equality/layout must not chase names or expand
the graph indefinitely. Do not force unrelated scalar types into a large new framework.

SemIR owns constructor identity/arity, field types, runtime evaluation order,
aggregate initialization/copy meaning, validity requirements, patterns, binding
scopes, exhaustiveness and guards. Introduce typed aggregate construction/value
facts shared with record values; generalize RecordCopy only where that shared
contract is actually needed. Use targeted boundary validation, not a new whole
SemIR verifier project. Exported type interfaces retain template/constructor facts
and reachable payload type identities without exposing private implementation names.

### NIR

- Carry finite, resolved aggregate layout/type references and storage IDs. A
  recursive pointer refers to an aggregate identity, not an infinitely expanded
  pointee. No generic parameters, source constructor names or pattern syntax
  survive as executable meaning.
- Lower constructed values into compiler-owned aggregate value temporaries,
  scalar field stores and existing CopyBytes. Keep private value captures distinct
  from source-addressable/static homes; record lifetime, initialization, escape and
  effects as reusable facts. This is necessary aggregate infrastructure, not a
  LET-specific storage optimizer or an assumption that all Atari locals are private.
- Extend call/result/return operands to distinguish scalar temps from aggregate
  values/result places. Include complete layout and parameter passing semantics
  in verified facts. Never hide an aggregate behind a fake CARD or scalar TempId.
- Lower match decisions into existing typed loads, comparisons and CFG edges.
  Capture before dispatch; extract fields only on the proven matching path.
  Preserve existing integer interval lowering; do not add a benchmark-specific
  switch instruction or prematurely add jump tables.
- Add a general non-returning fault representation if the existing fault path
  cannot express invalid-tag checks. It carries a semantic fault kind, not a raw
  Atari error number. Update CFG, effects, liveness, home synchronization, DCE,
  promotion, printers and all backend consumers. Prior observable stores must
  survive a potential handler call, just as for division by zero.
- Verify type/layout completion, checked offsets/extents, constructor tag domains,
  aggregate call/result compatibility, temporary lifetime and dominating validity/
  constructor checks. Retain structured checked-projection facts until their
  proof is validated; do not try to recover an active alternative from a bare
  field offset after erasing that information. No unknown-size aggregate scalar
  load/store is accepted.

### MIR, classic projection and emission

Reuse canonical layout, typed pointer/word moves, aggregate copy staging, argument
capture, frame/home planning and transactional rewrite proofs. Classic must gain
general pointer-valued aggregate field support, not special-case Tree payloads.
Adapt classic from structured SemIR facts without generating/parsing source or
looking up constructor names in the backend.

MIR alone decides register/zero-page use, concrete hidden arguments, copy loops,
scalar replacement and physical spills. Emission writes final bytes/relocations
and consumes verified plans. Unsupported target/type combinations fail before
partial emission. Existing scalar ABI output and unchanged fixtures remain gates.

## 5. Major slices and acceptance gates

### Slice 0 — Freeze contracts and establish a baseline

- Review the proposed grammar, 255-alternative/invalid-zero rule, snapshot/value
  semantics, error mapping, explicit recursion and initial ABI above.
- Record compiler/NIR/MIR/VM baselines and representative XEX sizes/cycles. The
  last verified baseline was 2,858 compiler tests, 139 VM harness tests, 38 NIR
  and 167 MIR fixtures; rerun rather than relying on these counts later.
- Add independent internal capabilities for aggregate values, variants/patterns,
  aggregate calls, generic types and guards, initially publicly disabled.
- Establish focused test modules and capability diagnostics; no lexer keywords
  or public syntax claims change yet.

### Slice 1 — Finite nominal aggregate identities

- Introduce canonical definition/instance references and provisional type entries;
  preserve enum and existing record identity/qualified-name behavior.
- Resolve layouts with cycle detection, pointer barriers, overflow diagnostics and
  target policy. Update relevant type/signature serialization and native canaries.
- Test self/mutual pointer cycles, illegal by-value cycles, aliases/import identity,
  shadowing, recursive pointer equality and unchanged old layouts/emission.

### Slice 2 — Shared aggregate values and classic pointer fields

- Extend typed expressions and SemIR/NIR carriers for record values and private
  aggregate captures; reuse ordinary stores and overlap-safe CopyBytes.
- Support record-valued LET initialization/copies with immutable field/escape
  checking. Do not add owned arrays or record constructor syntax.
- Repair the general classic pointer-valued field carrier for data pointers;
  validate nested and array-backed fields with captured addresses.
- Exercise value snapshots, self-copy, both overlap directions, effectful source
  addresses and calls before exposing this shared capability publicly.

### Slice 3 — Monomorphic construction, validity and flat matching end to end

- Add variant syntax, canonical alternatives/layouts and qualified constructors.
- Implement zero-invalid storage, staged construction, value-read validation and
  the shared non-returning InvalidVariant/Error path on both Atari backends.
- Add flat constructor patterns, `_` payload discard, immutable binding scopes,
  exhaustive valid-value coverage and correct return/EXIT flow facts.
- Integrate variants into variables, LET, assignment, pointer dereference, arrays
  and record fields through the shared aggregate path. Runtime assignment, not a
  new static-initializer notation, establishes valid objects.
- Open the first public variant gate only when construction and consumption both
  execute on classic/MIR6502 and both runtimes. Diagnose aggregate calls, generic
  syntax, guards and unsupported payloads while their gates remain closed.

### Slice 4 — Recursive data and fixed-arena execution

- Enable self/mutual recursive variant references from slice 1 through data
  pointers; integrate with array element addresses and complete layout consumers.
- Add a binary-search-tree sample with explicitly initialized EMPTY sentinel,
  fixed arena, capacity checks and iterative traversal; no implicit allocator.
- Cover shared subtrees, pointer mutation, NIL without implicit dereferencing,
  non-page-aligned pools and both common Atari backends. Native layout canaries
  demonstrate distinct pointer widths without claiming Atari recursive calls.

### Slice 5 — Direct aggregate parameters and function results

- Generalize named function result checking, source callable facts and value
  parameter handling for records and variants.
- Add verified aggregate call/result/return carriers and target physical planning
  for caller-owned result/argument buffers, preserving all old signatures.
- Support direct calls, RETURN of a constructor/place/call, nested aggregate
  arguments, ignored results, immediate CASE and LET consumers on both runtimes.
- Test mutable value parameters, explicit pointer parameters, source/result overlap,
  source evaluation order, hidden-pointer clobbers and simultaneous live results.
  Open direct aggregate calls only after the entire producer/consumer path passes.

### Slice 6 — Typed indirect calls and native ABI boundaries

- Extend typed FUNC POINTER signatures/results to concrete aggregate types and
  retain nominal distinctions even for equal machine layouts.
- Share direct/indirect call staging, hidden result locations and callee contracts;
  validate mixed scalar/aggregate arguments and result forwarding.
- Add 68k/65816 argument/result/frame and activation canaries. Keep unsupported
  executable paths explicit, and reject external aggregate calls without a declared
  supported ABI. No new cartridge/SYS entry signatures are inferred.

### Slice 7 — Generic TYPE definitions and concrete instantiation

- Add type parameters/applications through all relevant declaration/result/callable
  type contexts, LET annotations, payloads and qualified constructor heads.
- Implement one identity-based finite instantiation cache for records and variants;
  support regular recursive instances and reject expanding specialization cycles.
- Ship concrete Option/Result/TreeOf examples. Test wrong arity, unresolved parameters,
  forbidden type arguments, exact nominal compatibility, module exports, nested
  applications, depth limits and deterministic sharing. No generic routines yet.

### Slice 8 — Nested patterns and pattern usefulness

- Add nested by-value constructors and scalar literal subpatterns; reuse constant
  typing/range facts. Pointer dereference remains an explicit source operation.
- Replace flat variant duplicate tracking with a bounded, memoized constructor/
  product-pattern coverage algorithm. Later useful fallback patterns may overlap
  earlier specific ones; reject fully shadowed arms. Report a missing pattern
  witness for non-exhaustive matches without enumerating large scalar domains.
- Maintain scalar/enum CASE's existing overlap policy. For nested scalar domains,
  conservative wildcard requirements are acceptable; do not falsely claim coverage.
  Bound pathological analysis with a diagnostic, never an optimistic success.
- Verify extraction dominance, unused binders, inner/outer binding identity, nested
  invalid tags and unchanged captured values after writes to the original.

### Slice 9 — Ordered CASE guards

- Implement the already-designed guard syntax for variants and existing scalar/
  enum CASE. Generalize pattern/interval coverage without cloning dispatch engines.
- Guarded arms do not discharge exhaustiveness; later unconditional fallbacks may
  repeat their constructor/label. Diagnose unreachable arms after unconditional
  coverage and preserve the original selector snapshot when guards mutate storage.
- Validate counted/false/faulting guards, volatile access order, RETURN/EXIT,
  guarded catch-all and ELSE. Bare WHEN _ remains outside the syntax contract.

### Slice 10 — Public documentation, examples and code-quality gate

- Publish supported syntax, invalid-tag behavior, constructor numbering/layout
  caveats, value-copy/ABI rules, aggregate LET restrictions and capability matrix.
- Deliver Event, OptionalByte/generic Option, ReadResult/generic Result, and
  fixed-arena Tree examples. Include records as products and explicit-pointer
  recursive data; do not present them as implicit-heap ML values.
- Measure generated bytes, cycles, temporary storage, tag checks and copies against
  equivalent handwritten tagged-record code on representative complete programs.
- First use existing storage promotion, copy forwarding/elision, effect/escape
  facts and MIR selection. Keep legitimate improvements separate from correctness
  slices; a tagged type alone does not authorize removing checks across effects.
  Record remaining costs instead of promising universally zero-cost ADTs.

## 6. Verification and regression matrix

For each relevant slice:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test
```

Run focused execution tests during development and the full pinned VM suite
before a public capability gate or completed major backend slice:

```sh
cd tools/vm-runtime-tests
cargo test --locked --no-fail-fast
```

Use independent host-value and complete guarded-memory oracles, not output copied
from one backend. Cover modern classic/MIR6502, raw/optimized paths and both
runtimes wherever supported. Compatibility rejection is a semantic test, not a
VM execution claim. Preserve the existing LET, arithmetic, enum/CASE, record-copy,
argument-capture, pointer-scratch, sample-build and Oscar64 regression suites.

Required new coverage includes:

- Empty/payload alternatives; 1 and 255 constructors; rejection of 0 and 256;
  invalid tag 0 and out-of-domain nonzero tags; byte tag independent of target.
- Native alignment and pointer widths; checked size/stride arithmetic; equal-sized
  distinct types; nested aggregate payloads and record-contained fixed arrays.
- Every constructor, inactive payload guards, padding, partial initialization,
  explicit construction of arena entries, and source-order initializer effects.
- Copies and destinations crossing pages; sizes around byte/word copy boundaries
  including 31/32/33 and 255/256/257; self-copy and both overlap directions.
- Reads from potentially corrupted memory; no normal continuation after faults;
  a returning Error handler; prior observable stores visible to that handler.
  Snapshot checks do not establish concurrent-memory or pointer-lifetime safety.
- Arm bindings cannot be assigned, addressed, aliased or referenced in assembly;
  pointers extracted from them can still be used to mutate their pointees.
- CASE selector and guards with counted calls, source mutation, nested matches,
  skipped effects and faulting expressions; scope and exhaustiveness diagnostics.
- Direct/indirect nested calls, caller/callee buffer separation, independently
  captured arguments, mutable value parameters, result forwarding and ABI bounds.
- Self/mutual recursion through pointers, illegal inline cycles, generic instance
  reuse, growing-instance rejection and deterministic qualified signature identity.

Update fixture counts when adding fixtures. Explain changed snapshots as an
intentional typed-IR contract extension; unrelated snapshots and existing scalar
ABI listings must stay unchanged. Do not characterize new miscompilations as
expected failures: repair them separately or keep the affected feature gated.

## 7. Completion criteria and next action

The plan is complete only when nominal variants can be constructed, copied,
bound by LET, passed/returned by value and exhaustively matched with immutable
bindings; concrete generic instances and explicit-pointer recursive data compose
with those operations; nested patterns/guards obey the capture/effect contract;
and the declared backend/runtime matrix has executed its corresponding tests.

Implementation starts with shared aggregate foundations. The overlapping payload
layout will be reusable, but does not require a public untagged UNION feature.
Do not add implicit boxing, a new allocator, or speculative optimization.

## 8. Implementation progress

### Slice 0 — Contracts and baseline (complete)

- Accepted the contract above: tag 0 is invalid, constructors use 1..255,
  value snapshots are shallow across pointers, invalid values use Error 100 on
  Atari, recursion is explicit through pointers, and aggregate results use
  caller-owned storage.
- Added independent internal acceptance gates for aggregate values, variants,
  direct/indirect aggregate calls, generic types and CASE guards. Both public
  profiles leave every gate closed on every target; no CLI switch or syntax
  claim is added.
- Fresh baseline at `ceecea1`: all 2,858 compiler tests, NIR snapshots, all 38 NIR
  fixtures, all 167 MIR6502 fixtures and all 139 locked VM harness tests pass.
  The latter include the LET bytes/cycles comparison oracles. A fresh standalone
  MIR6502 AES build remains 4,211 XEX bytes; no new AES execution timing is claimed.
- The two capability-gate tests and `cargo check --all-targets` pass after adding
  the gates. Runtime-source analysis explicitly keeps them disabled too.
### Slice 1 — Finite nominal aggregate identities (complete)

- Resolved records now carry their defining SymbolId and canonical signature key.
  Equality, record-family checks, layout dependencies, field lookup, initializer
  walks, NIR storage extents and classic type projection use that identity.
  Pointers retain finite references; existing record layout/field tables remain
  the graph. Concrete generic instance interning is still slice 7, not an unused
  parallel registry introduced ahead of its consumers.
- Reused the named-declaration resolver for provisional type entries in ordinary
  module/routine/lexical type scopes. This new visibility is behind the internal
  aggregate-values gate; it does not enable forward constants, variables or
  routines. Existing public type visibility is unchanged.
- NIR retains record definition identities and rejects name-only references.
  Callable signature keys remain stable across unrelated declaration-ID shifts.
  Readable record dumps retain their previous format; no snapshot update is
  intended.
- Corrected homonymous routine-local record handling: classic's flat layout
  registry gives colliding declarations distinct projected names, while NIR
  selects field/array extents by declaration identity.
- Initializers consume the type resolved at the declaration head, preserving
  legal local declarations such as `Holder holder=[@first]` after the entry
  shadows the type name. The native entry-initialization regression suite passes.
- Semantic/NIR and native backend canaries cover self/mutual pointers, inline/
  array layout cycles, import aliases, typed callback parameters and local
  shadowing. A separate independent whole-memory guard oracle covers same-named
  local record layouts/copies on classic/MIR6502, raw/optimized NIR and both
  Atari runtimes. Classic pointer-valued fields remain a slice 2 boundary.
- Verification: all 2,871 tests in the full compiler run passed, followed by the
  final 11-test aggregate-identity run (one additional callback-alias test).
  All 140 locked VM harness tests pass. NIR snapshots, 38 NIR and 167 MIR6502
  fixtures pass unchanged; `cargo check --all-targets` passes. Fresh AES XEX and
  assembly are byte-for-byte identical to the slice 0 baseline (4,211 XEX bytes).
- Slice 2 and subsequent public capabilities are not implemented yet.
