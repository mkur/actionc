# Semantic Invariants

This note records language-level invariants that `actionc` should preserve as
we move from AST codegen toward semantic IR, NIR, and MIR6502.

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

## Modern Immutable Runtime Bindings

LET is an executable local binding, not CONST or a static declaration
initializer. Analysis creates a fresh child scope for the remaining routine or
explicit-block statements. The annotation and initializer use the parent scope;
later uses bind to the new immutable SymbolId. Sequential shadowing never
changes earlier resolutions. Control-flow bodies need an explicit BEGIN/END.

Inference retains canonical scalar, enum, REAL, data-pointer, callable and record types.
An annotation uses assignment conversions; it cannot widen narrow intermediate
arithmetic. Records are complete value snapshots; owned arrays and record
constructor syntax are not introduced. Invalid/non-value initializers
must produce a diagnostic, never a successful model with a missing binding scope.

Semantic symbols carry immutable-binding facts and source places are read-only.
Assignment, compound assignment and FOR writes are rejected, as are address
escape, static storage aliases and machine/ASM references to the binding home.
Read-only access propagates through inline record fields and inline-array
elements. Neither explicit addresses, implicit record/array decay, nor casts may
expose these subobjects. Dereferencing an immutable pointer (including a pointer
field of an immutable record) produces an ordinary writable pointee place.
Runtime LET dependencies cannot enter compile-time/static initializer or bound
contexts; unevaluated layout queries do not read the binding value.

SemIR represents a LET with existing lexical storage and one compiler-owned
initialization assignment or typed `RecordCopy` at the source execution point. Its declaration has
neither a static initializer nor an executable declaration initializer. All
subsequent source places remain read-only. A loop executes the initialization
on each encounter; an untaken branch does not execute it. The target's existing
activation/storage rules are unchanged.

Immutability is not purity: calls, volatile reads and other initializer effects
remain ordered and cannot be discarded merely because the result is unused.
It also does not imply permanently read-only storage or immutable pointees.
Classic projection and NIR consume resolved scopes, types and ordinary ordered
initialization; neither backend decides LET source semantics.

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

CASE retains ordered arms and the distinction between no ELSE and an explicit
empty ELSE. SemIR owns selector type, constant interval validation, overlap
diagnostics, and return/EXIT flow. Labels never execute. NIR captures the selector
once and emits ordinary typed comparisons and CFG; classic projection uses a
collision-free captured scalar and existing IF branches. Neither backend
re-evaluates source labels or duplicates selector effects.

Typed semantic subjects are the authoritative representation for expression,
place, callable, and type-reference meaning.

The current analyzer still exposes `SemanticModel.expression_observations` as a
compatibility/debug projection of expression span, category, and type. This side
table must not drive SemIR lowering or code generation. It exists to keep older
tests and observability hooks working while the compiler migrates toward typed
semantic nodes and NIR.

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

SemIR owns the resolved `FOR` step control fact. Constant steps are classified
as ascending or descending with an explicit magnitude; an unclassifiable step
remains explicit as unknown. NIR and later backends consume this fact instead
of recovering loop direction from expression syntax.

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

SemIR-driven classic codegen now projects canonical `RecordType` layouts into
its existing record table before allocating storage. Nested record identities
and complete sizes are registered before field references are resolved. Field
offsets, complete inline extents and array shape do not depend on projected AST
bounds. Application and selected runtime layout tables are combined with local
record-ID rebasing. The old AST layout collector remains only for direct
AST-only codegen entry points, not as a fallback for SemIR-driven compilation.
Pointer-valued record fields still lack a classic field-place carrier and
produce an explicit unsupported diagnostic rather than being treated as
inline records.

Inline-array indexing uses those projected field facts for
element width, signedness and record identity, independently of the full field
extent. Runtime decay computes the subobject address; it never reads an array
descriptor from the field. Static-address queries do not emit code. Dynamic
address evaluation happens once, with captured destinations preserved across
later index/RHS calls. Integer compounds evaluate the RHS before reading the
captured destination's current value, matching the cartridge. Named arrays
retain their existing descriptor/backing rules. Public support remains gated
until aggregate initialization and copy validation are complete.

`SemStmt::CompoundAssign` carries `SemCompoundOperation`: the ordinary binary
result type and an optional final conversion to the destination type. An INT
multiplication result and a CARD divisor must not be narrowed merely because
the destination is BYTE. NIR emits the typed operation followed by an explicit
cast when the store type differs. Named array pointer-cell updates use the
canonical layout's pointer type, not the array element type. Classic receives
the same facts through its projection and uses shared captured-place lowering
for effectful or otherwise unsafe indirect compound fallbacks. Compatibility
surface restrictions are unchanged. Native REAL remains on its existing
separate lowering path; this integer fix does not change its evaluation order.

Resolved field facts also carry storage shape, complete byte extent and target
alignment. Scalar fields use their value width. Embedded fixed-length array
fields retain an `ArrayType` and element stride separately from their complete
storage extent; they are not scalar values or pointer cells. Field placement
and record tail padding use checked arithmetic, and incomplete or recursive
by-value layouts are diagnosed before layout facts are published.

Named-module record layouts and constants resolve through a dependency
lifecycle keyed by defining SymbolId. Layout consumers request needed facts;
declarations are not speculatively evaluated or silently retried. Cycles and
failed dependencies cannot publish successful layout facts. Constant source
visibility is independent of resolution order, so resolving a constant for a
later record does not make forward CONST references legal. A pointer's own
width does not require a complete pointee layout.

Embedded fields are enabled in modern classic and MIR6502 with both Atari
runtimes; Compatibility rejects the extension. See the
[implementation plan](EMBEDDED_RECORD_ARRAYS_IMPLEMENTATION_PLAN.md).

Record arrays decay to their element backing before considering implicit
address-of for individual records. Their element record type must not make
them appear to be single record objects. Local fixed-address arrays preserve
the same resolved backing facts and pointer-image rules as globals. NIR's
object layout describes storage extent, separately from element width; MIR
consumes that extent while retaining its existing explicit pointer-view and
descriptor/backing allocation rules.

The shared semantic type model exposes record shape as `RecordType`:

- `name` is the source record/type name;
- `fields` is the resolved ordered field list with field ids, types, and
  offsets;
- `size` is the total byte size implied by the field layout.

SemIR `TYPE` and `RECORD` declarations should carry `RecordType` alongside the
lowered field nodes. Current codegen may still bridge through `ValueType`, but
NIR lowering should use `RecordType` for field layout and record-pointer
reasoning rather than rebuilding it from source text.

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

## Whole-Record Assignment

Assignment between addressable values of the same declared record family copies
the complete record storage value. Equal-sized or equal-shaped records from
different declarations are not assignment-compatible. Record compound
assignment remains invalid, and assigning a record to a matching record pointer
keeps the implicit-address behavior described above.

The destination place is evaluated first and the source place second, exactly
once each. The copy has value semantics: all source bytes are observed before
any destination byte is changed. Self-assignment is valid and partially
overlapping aliases behave like `memmove`, in either overlap direction.

SemIR represents this operation as `SemStmt::RecordCopy` with typed places and
the resolved record extent. NIR carries `CopyBytes`; ordinary scalar loads and
stores reject record types. MIR6502 and classic code generation consume these
structured facts rather than recognizing record syntax or reconstructing type
identity from names.

## Scalar Type Foundation

Nominal BYTE enums retain their defining type identity through semantic subjects,
constants, and SemIR. `as_scalar()` deliberately excludes enums; the separate
representation query is for storage and already-validated constant payloads,
not implicit numeric compatibility. Only NIR lowering and classic projection
erase enum values to unsigned bytes. Every byte representation is defined, so
named-member coverage does not prove an enum CASE exhaustive. Enum declarations
and CONST bindings remain semantic metadata and allocate no runtime storage.
The legacy AST materializer retains enum CONST bindings rather than replacing
them with untyped numeric literals and losing identity under shadowing.
Enum initializer leaves are checked against the destination's nominal type
before the canonical static-data plan encodes their bytes. Partial aggregate
initializers keep existing zero-fill semantics, including unnamed zero values.
Type metadata survives selective linking but does not allocate storage.

Function and callable signatures carry a resolved `ValueType` result, including
nominal enum/aggregate identity and concrete generic applications. Signature IDs
use canonical identities, not alias spelling or byte width. Only the backend
adapter erases enum results to the existing BYTE ABI. Modern aggregate arguments
are independent mutable copies; aggregate results use caller-owned storage and
a separately lowered physical ABI, on direct and exactly typed indirect calls.

Returning A equal to a result slot does not prove that the callee's N/Z flags
describe A. Classic call facts keep this distinction. MIR rewrite-time callee
summaries publish exact N/Z only for immutable machine-code routines: local
rewrites do not yet preserve an interprocedural flag-exit contract for mutable
MIR routines. Analysis of a fixed MIR program may still report exact N/Z.

The canonical scalar semantic model is `ScalarType`:

- `BYTE`: 1 byte, unsigned;
- `CHAR`: 1 byte, unsigned;
- `CARD`: 2 bytes, unsigned;
- `INT`: 2 bytes, signed.

Existing `ValueType` remains the bridge used by the current analyzer and
codegen, but scalar decisions should route through the scalar model rather than
duplicated width/signedness tables.

### Arithmetic migration policy

All actionc profiles, including Compatibility, share correct
target-independent integer semantics. Cartridge runtime linking is not a
numeric dialect: compiler-owned helpers may coexist with cartridge services.
Historical behavior will be obtained by running the original compiler in the
separately planned VM mode, not by emulating its bugs in actionc.

The [legacy audit](bugs/LEGACY_INTEGER_ARITHMETIC_AUDIT.md) records historical gaps;
the [modern arithmetic plan](MODERN_INTEGER_ARITHMETIC_IMPLEMENTATION_PLAN.md)
records the numeric rules and implemented rollout. Operand types,
result types, faults and conversions belong to semantic/NIR contracts; helper
signatures and physical bindings must implement those contracts, not define them.

Division/MOD use CARD's unsigned domain if either operand is CARD, otherwise
INT's signed domain if either is INT, otherwise unsigned BYTE/CHAR. Convert
operands explicitly before the operation; destination narrowing/widening does
not change that operation or its operand tree. Signed division truncates toward
zero, and nonzero remainder has the dividend's sign. INT_MIN/-1 wraps to
INT_MIN with remainder zero. Multiplication retains its existing INT result.

Literal/foldable converted-zero divisors are semantic errors. Dynamic zero
raises the non-returning DivisionByZero fault. Atari 6502 helpers call the
existing Error entry with A=101, X=0, Y=101: cartridge `$04CB`, or linked
SYSLIB Error in standalone builds. If it returns, clear decimal mode, restore
A/Y=101, set carry, and enter a BCS self-loop. See the
[Atari runtime error audit](ATARI_RUNTIME_ERRORS.md) for the code and ABI evidence.
An unused result does not make a potentially faulting computation discardable
or movable across effects. Unexecuted runtime branches do not fault. All
constant evaluators and backends implement these same rules. Potential faults
also observe prior fixed/escaped storage through the error handler. NIR home
promotion synchronizes such storage before the operation; dead-store elimination
preserves those writes. MIR exposes unknown handler memory/OS effects separately
from the arithmetic kernel's bounded returning-path scratch.

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
NIR lowering should use those facts instead of re-deriving array shape from
symbol class and scope.

The shared semantic type model exposes this shape as `ArrayType`:

- `element` is the source element type;
- `length` is the declared constant bound when it is statically available;
- `pointer_type()` is the exact decay target type.

SemIR array declarations and array parameters should carry an `ArrayType`
alongside their existing element `SemType`. The element `SemType` remains the
bridge for current codegen, but future semantic-IR/native lowering should use
`ArrayType` when it needs array shape or decay information.

Modern-profile embedded array fields follow the same array-place model, with
their canonical shape obtained from the owning `FieldId`. The semantic model
retains resolved array-place types by scope and expression source site for
SemIR; these are authoritative facts, not expression observations. Indexing
produces an element place even when the field belongs to a nested record,
record pointer or indexed record. `SIZEOF` uses the full field extent,
`ELEMENTS` uses its bound, and layout-query operands remain unevaluated.

A bare embedded array field is not a scalar load, store, record-copy operand,
or rebindable array descriptor. Runtime assignments and arguments with an
exactly matching element-pointer type lower to `SemArrayDecay` with
`RecordField` origin. Explicit address-of and casts remain available. This
does not change the existing named-array conversion policy.

`SemFieldRef` carries storage shape and full byte extent in addition to the
element type and offset. Complete field extent is distinct from element width:
an `INT ARRAY values(100)` field occupies 200 bytes but has a two-byte element.
NIR lowering consumes this distinction without introducing array scalar values.
Static pointer initializers may decay a matching inline field or explicitly
address a subobject of known storage with constant indexes. Initializer-list
address leaves also accept these places, including low/high-byte selectors
and constant byte addends. Semantics resolves each to a storage SymbolId plus
byte addend using canonical field offsets and strides; SemIR emits the existing
typed static-write plan. Runtime pointers, array parameters and dynamic indexes
are not static bases. Scalar non-pointer alias declarations are not generalized
by this extension; use pointer declarations or address-valued list leaves.

Plain scalar variables do not decay to pointers. A pointer can still be
assigned a `CARD` value as an explicit raw-address escape hatch.

## Aggregate Static Initializers

A bracketed initializer for a record or an array of records is a flat source
sequence interpreted against the resolved destination layout. Values map to
scalar leaves in recursive declaration order; record field widths and offsets,
not a uniform enclosing-element width, determine every write.

For example, `Pair ARRAY pairs(2)=[1 $2345 2 $6789]` for
`TYPE Pair=[BYTE tag CARD word]` initializes packed bytes
`01 45 23 02 89 67`. Direct record variables use the same rule. Nested records
are recursively flattened, while their field paths remain diagnostic metadata
and do not become executable field-name dependencies.

For embedded arrays, the same shared leaf walk repeats
each inline field's resolved element count and stride, recursively visiting
record elements. Padding is not a source element. Partial lists zero-fill the
remaining full object extent; diagnostic paths include inline element indexes.
Validation and SemIR planning use this one canonical walk, including on aligned
targets, rather than independently reconstructing scalar-only record layouts.

Semantic analysis and SemIR own this interpretation. A verified
`SemStaticInitializer` records the total initialized extent plus typed writes
with explicit byte offsets, widths, values, stable relocation targets, and
source spans. NIR and either backend consume that plan; they must not infer
record layout from initializer syntax.

The storage extent follows these rules:

- an explicit record-array bound determines the full extent and missing leaves
  are zero-filled;
- an inferred record-array bound rounds a partial final record up to one full
  record and zero-fills its trailing leaves;
- excess values, invalid leaf types, invalid address widths, overflow, and
  unrepresentable layouts are diagnostics rather than implicit zero data.

Address-valued leaves reuse the relocatable-static-initializer contract in
`docs/RELOCATABLE_STATIC_INITIALIZER_IMPLEMENTATION_PLAN.md`: SemIR resolves
the target and selector, NIR carries stable-ID relocations, and emission patches
the final address after layout.

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

Modern extensions are explicit capabilities, including LET, variants, generic
types and guarded matching. Runtime selection does not choose language semantics:
modern generated code can link cartridge or standalone services. Running the
original cartridge compiler is distinct from selecting the cartridge runtime.

## Variant values and concrete generic types

Modern `TYPE Name=VARIANT [EMPTY VALUE [BYTE value]]` declarations define
nominal alternatives. Qualified constructors are runtime values: nullary
constructors omit parentheses; payload arguments are positional and captured
once, left to right. Assignment captures its destination address first, stages
constructor/call results as needed, then replaces the whole destination. Checked
value-to-value assignment captures both addresses, rejects partial overlap unless
canonical object facts prove identity/disjointness, validates the source in place
and transfers once. Exact self-assignment validates but need not copy. Whole-value
transfers containing inline variants require identical or disjoint complete
ranges; nested containment and same-object pointer aliases remain valid. Unknown
range checks use target ADDRESS arithmetic and the existing InvalidVariant fault,
not a global no-alias assumption. Payload/tag fields are not source lvalues.
Ordinary records, embedded arrays and typed pointers reuse
their existing layouts and aggregate-copy semantics.

Tags are BYTE on every target: 1..255 in declaration order; zero is invalid.
Construction zeros padding/inactive payload bytes and writes the tag last.
The private constructor capture is filled in argument order, then only gaps
outside the tag and complete active field extents are zeroed. Fully covered
constructors perform no clearing. Copied aggregate fields retain their entire
byte image, including internal padding and union storage; they are not cleared
again. The completed capture is published through the existing whole-value copy,
so argument effects and failures cannot expose a partially built destination.
Variant-containing declarations have an explicit zero image: load time for
Atari routine-static/global storage, activation entry for native automatic
storage. Immutable captures initialize at execution, not activation entry.
Direct volatile, absolute/alias and static-initializer variant declarations are
rejected. Typed pointers can address low-level memory; copying or matching such
a value still validates its tag, but does not establish pointer lifetime/safety.
See [the variant storage contract](VARIANT_STORAGE_CONTRACT.md) for overlap
enforcement, ordering and the distinction from plain record/union copies.

CASE patterns resolve constructor IDs, canonical field paths and immutable
arm-local symbol IDs. Nested by-value constructor and scalar literal patterns
compose; binders and `_` cover remaining payload domains. A bounded memoized
constructor/product analysis rejects fully shadowed patterns and reports missing
coverage witnesses. Scalar literal lists never establish complete payload-domain
coverage by themselves. Invalid tags fault before any arm, including ELSE or a
guarded catch-all. Exhaustive returning matches count toward function return
coverage; pointer payloads are not followed.

Guards execute exactly once after matching and initializing bindings. False
continues to the next arm using the original captured value. Guarded arms do not
establish coverage, even when the guard is constant true. Guard calls, volatile
accesses and faults are not speculated into unmatched paths. ELSE stays the final
unguarded fallback; a guarded `_` is distinct from ELSE. Scalar CASE preserves
disjoint unconditional labels while allowing useful guarded overlaps.

The shared SemIR builder captures and validates source values, checks projection
owner, constructor membership, exact nominal type and canonical extent, and only
then erases patterns into typed copies, field projections and dispatch. NIR
receives no pattern strings or backend-specific aggregate ABI. Ordered typed
arm refinements dominate extraction; guarded binding declarations/initialization
remain visible to storage, link, effect and feature visitors.

Generic record/union/variant instances are interned by defining SymbolId and canonical
concrete ValueTypes, not printed names. Recursive placeholders retain finite
identities across pointer cycles. Inline layout cycles and structurally expanding
specializations are rejected; explicit depth/work/instance limits diagnose
pathological input. No generic routine inference, runtime dictionaries, implicit
allocation or recursive Atari activation model is introduced.

## Untagged unions

Semantic type observations include destination-only places, including nested
fields, indexes and pointer dereferences. Backend capability checks must not
miss an unsupported scalar store just because no expression reads that type.
Observations are identified by source site and class, not incidental vector order.

UNION definitions are supported in modern profiles; compatibility rejects them.
Canonical aggregate layouts distinguish Record, Union and Variant explicitly.
Member FieldIds retain ownership but do not imply disjoint storage: every direct
union member has offset zero, while nested record fields remain sequential.
Extent/alignment and layout queries use the shared target layout machinery.

The initial union representation excludes inline VARIANT, REAL and callable
pointers, including through records/arrays; ordinary data pointers are traversal
barriers. Positional/string initializers for inline union-containing storage are
rejected before the scalar-leaf initializer walk can visit overlapping members.
Generic union instances use the same finite cache and post-substitution checks.
There is no active member, tag validation, implicit clear on member writes or
numeric conversion between views. Enum members admit all byte representations.
Whole-value copies include the complete padded extent and are overlap-safe;
LET and pattern binders capture immutable snapshots. Pointer copies share
pointees. Exact nominal direct/indirect call boundaries reuse the aggregate ABI.
Union values are not scalar arithmetic operands, truth values or CASE selectors.
RAM absolute/alias bindings and object-level VOLATILE preserve existing storage
semantics. Volatile whole copies access the complete extent, without atomicity or
hardware-register safety guarantees. Existing qualifier restrictions remain.
No union-specific executable NIR operation or AST-only layout recovery is added.
Actual wide scalar operations require MIR6502 on Atari; native canaries cover
layout/access/ABI planning, not native runtime execution. Enclosing a union in a
variant preserves outer-tag validation and native Error-adapter restrictions.
See the [union tutorial](tutorials/UNIONS.md) and
[acceptance plan](Action_2027/UNIONS_IMPLEMENTATION_PLAN.md).

## Current Implementation Gaps

Known gaps between these invariants and the current implementation:

- many identifier uses still carry strings or are re-looked-up downstream
  instead of being fully bound once in semantic analysis;
- direct AST-only codegen still rebuilds record layouts, and classic still
  derives ordinary array storage details from projected AST declarations;
- NIR still needs more of the semantic binding/fact model represented as
  structured, verifier-checked facts.

These gaps should be closed slowly, with tests added before broad rewiring.
