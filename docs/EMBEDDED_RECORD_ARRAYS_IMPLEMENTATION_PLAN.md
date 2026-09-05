# Fixed-length arrays embedded inside records

Status: slices 1-5 complete, 2026-09-06. Atari modern classic and MIR6502
support is enabled with both runtimes; slice 6 ports remain pending.
Baseline: `6c73c1d`; 2,662 root tests passed (22 existing ignored), 100 VM
tests passed, including 4,380 Oscar64 cases. No stage-5 ports have been added.

## Purpose and profile policy

Enable structurally faithful ports of Oscar64 `structmembertest.c` without
replacing embedded arrays with pointer fields or separate backing tables.
`structarraycopy.c` also belongs to stage 5, but needs no new source construct.

The enabled policy is modern-only: modern classic and MIR6502, with both
ActionCart and Standalone linking. Compatibility will diagnose the extension
during semantic analysis. Public enablement followed semantic, backend and
aggregate validation. Earlier slices used an experimental semantic capability;
`SemanticOptions::modern()` now enables it through the public compiler profiles.

## Source contract

```action
CONST Count=100
TYPE Buffers=[INT ARRAY x(Count),y(Count)]

Buffers data
Buffers POINTER p
Buffers ARRAY rows(3)

PROC Copy(Buffers POINTER a)
  INT i
  i=0
  WHILE i<100 DO
    a.x(i)=a.y(i)
    i==+1
  OD
RETURN
```

- Fields occupy contiguous storage inside their enclosing record. On Atari,
  `Buffers.x` starts at 0, `y` at 200, and the record size is 400 bytes.
- Require explicit positive compile-time bounds; allow visible scalar CONST
  expressions under existing arithmetic and source-visibility rules. Reject
  unsized, dynamic, negative, zero and unrepresentable extents.
  Bounds follow typed CONST arithmetic; use CARD to express positive sizes
  above INT's range. Layout multiplication/addition never wraps or saturates.
- Reuse existing fixed-array element eligibility and target alignment rules.
  Complete nested record types are supported; recursive by-value layouts are
  rejected. Pointer recursion is not by-value recursion.
- Support direct/local/absolute records, record pointers, nested records and
  record-array elements, including `rows(r).x(i)`.
- Embedded array storage cannot be rebound. In a matching pointer or array
  argument context, the array place decays to its first element's address.
  Preserve exact element-type compatibility and existing explicit-address rules.
- `SIZEOF(data.x)=200`, `ELEMENTS(data.x)=100`, and
  `OFFSETOF(Buffers,y)=200`. Layout queries do not execute their operands.
- Enclosing record copies include every embedded byte, preserve record-family
  identity checks, evaluate the destination before the source exactly once,
  and retain self-copy/overlap-safe value semantics.
- Aggregate object initializer lists use recursive declaration/element order,
  existing partial-initializer/zero-fill rules, and existing relocation rules.
  Default field initializers inside TYPE declarations remain unsupported.

Non-goals: runtime bounds checking, dynamic/flexible array fields, general
multidimensional syntax, standalone whole-array assignment, aggregate returns,
new allocation or ABI rules, and new optimizer passes.

## Architecture and reuse

SemIR owns field identity, element type, bound, inline storage, layout and
conversion rules. Reuse `ArrayType` and stable `FieldId`; generalize array-place
queries to handle symbols and fields. Do not make arrays new scalar values.
Store resolved field extents/alignment rather than repeatedly recovering them
from declaration syntax. Reject incomplete or overflowing layouts instead of
using zero, wrapping or saturation as successful sizes.

The existing `SemArrayDecay`, `SemArrayOrigin::RecordField`, field/index places,
`SemStmt::RecordCopy`, NIR `CopyBytes` and static-initializer layout walker are
the starting points. Existing record-field array declaration scaffolding is
not complete support: subject indexing and decay currently assume named
arrays, and downstream consumers still need to honor inline array extents.

NIR should express the address through its ordinary operations:

```text
record base + record index * record stride
            + field offset + element index * element stride
```

Keep complete field storage extent distinct from element width. Inline decay
computes an address, not a pointer load from the field's first bytes. Existing
descriptor-backed top-level arrays and parameters retain their current rules.
Preserve evaluation order, conservative alias/volatile effects and captured
addresses across calls. No executable field names, fake field-global symbols,
source summaries or SemIR lookups may reach MIR6502.

Classic must receive the new layout through its existing SemIR-fact projection;
the AST-based layout builder must not become a competing authority for the new
form. Reuse effective-address calculation, recursive index staging and protected
destinations. MIR6502 reuses normalized field/index addressing and copy
selection. Full 16-bit offset arithmetic must cover fields beyond indirect-Y's
byte offset range. Optimize only if a measured remaining case cannot safely
use the existing machinery.

## Implementation slices

### 1. Language contract and rollout gate

Status: complete.

Save this plan and link stage 5 to it. Add a semantic capability disabled in
all public modes while downstream support is incomplete. Keep existing forms
and profile defaults unchanged; expose no new CLI option.

### 2. Canonical field metadata and checked layout

Status: complete. Public array syntax remains gated until slices 3–5 are ready.

- Add scalar-versus-inline-array shape to resolved record field facts, with
  `ArrayType`, constant element stride, full storage size and alignment.
- Resolve bounds using existing typed constant evaluation; derive field offsets
  and record sizes with checked alignment/addition/multiplication.
- Keep nested record sizes and ordinary arrays of those records consistent.
- Preserve stable field ownership across local scopes and modules.
- Add layout-only tests for packed and naturally aligned targets, mixed widths,
  records above 255 bytes, overflow and incomplete/recursive declarations.

Implemented: `RecordFieldStorage` retains scalar versus inline-array shape,
`ArrayType` and the resolved stride. Semantic fields/layouts retain complete
size and alignment; record construction checks multiplication, field offsets
and tail padding. Tests cover 400-byte records, nested record strides, scalar
widths, target alignment, local constant shadowing, imported bounds, stable
module-owned fields and invalid/overflowing declarations. Public compiler API
tests keep all six mode/runtime combinations gated.

Seventeen existing NIR snapshots now print `storage: Value` in ordinary record
field metadata. This is a printer-only change to those fixtures: offsets,
record sizes and executable instructions are unchanged.

Named-module type/CONST dependency ordering is implemented. The following
source now resolves under the experimental semantic capability:

```action
MODULE Data
PUBLIC CONST Count=100
PUBLIC TYPE Buffers=[INT ARRAY x(Count),y(Count)]
ENDMODULE
```

Record layouts and constants share a per-module pending/resolving/complete/
failed lifecycle keyed by defining SymbolId. Existing semantic consumers
request dependencies when they need a constant, a complete record width or a
field offset. Each declaration is evaluated once; failed dependencies stay
failed and cycles produce a dependency-chain diagnostic. Source visibility is
checked independently of scheduling, including already-cached constants:
later CONST declarations remain unavailable. Pointer widths do not depend on
pointee layouts. Legacy declaration ordering is unchanged.

Seven additional tests cover same-module constant chains, bounds composed from
queries on later record types, cached forward-reference rejection, cycles,
pointer recursion, existing public query constants and one-time failure
diagnostics. Existing local/imported bounds and field-identity tests remain.

This slice also exposed and repaired a pre-existing
[layout-query source-span collision](bugs/LAYOUT_QUERY_SPAN_COLLISIONS.md).
The parser repair is a separate commit and has public-mode code-equivalence
coverage; it is not an embedded-array special case.

### 3. Array places, SemIR and NIR

Status: complete. Public modes remain gated pending slices 4–5.

- Generalize indexing, decay and layout queries from array symbols to array
  places, including nested/indexed record fields.
- Carry resolved shape into SemIR; lower address calculations using existing
  NIR field/index/address operations.
- Preserve full storage extent without permitting scalar array loads/stores.
- Tighten NIR validation of extents, stride and addressable subobjects. Add
  structural/snapshot tests for direct, pointer and nested cases and ordered
  effectful base/index evaluation.

Implemented: canonical array-place queries support symbols and fields; fields
support indexing, exact element-pointer decay in runtime assignments/arguments,
explicit address-of, and unevaluated layout queries. Bare array loads, stores,
rebinding, arithmetic, record-copy operands and unindexed member selection are
diagnosed. Named-array behavior remains unchanged.

SemIR field references retain canonical ownership, complete storage extent,
shape and stride. Field declarations resolve by owner SymbolId rather than
unqualified type spelling, including named modules. NIR uses existing
field/index/address forms with complete element widths. It checks full inline
field extent at lowering and verifies normalized element extents, nonzero
strides and field/index type consistency. No new scalar array type, descriptor
load, runtime bounds check or backend optimization has been introduced.

Thirteen development-only tests cover direct/local/absolute/nested/pointer
records, indexed record arrays, record elements, named modules, layout queries,
ordered calls, volatile elements, invalid scalar uses and malformed IR. The
layout matrix includes all four targets; BYTE/CHAR/INT/CARD/REAL element cases
include bounds 1/129/257 and offsets above 255. A new NIR snapshot records the
400-byte Buffers layout with member offsets 0/200 and two-byte element stride.
Existing NIR snapshots are unchanged.

Slice 3 originally gated static inline-subobject initializers pending a shared
relocation path. Slice 5b replaces that diagnostic for statically resolvable
pointer and list initializers. Compile-time layout queries remain allowed.
No public profile exposes this intermediate capability before slice 5c.

### 4. Backend support

Status: 4a canonical layout projection, 4b executable array places and
4c shared integer compound-operation typing/order are complete.

Classic: project canonical layouts; generalize field-based indexing and array
decay through existing effective-address/staging paths. MIR6502: retain inline
subobject backing and consume the normalized address with existing selectors
and fallbacks. Verify loads, stores and compound assignments on both backends
and runtimes, especially offsets above 255 and pointer changes during calls.

4a projects canonical `RecordType` into classic's existing layout table, keeping
complete field extent distinct from array element stride. All record identities
are registered before nested field references resolve. SemIR-driven allocation
no longer invokes the AST layout builder. Standalone preflight, final linking,
and cartridge runtime extensions carry and merge application/runtime layouts,
rebasing nested record IDs. Direct AST-only entry points retain their existing
collector; no array support was added to that competing layout path.

Six regression tests cover full inline extents, nested record-array strides,
module-owned same-spelling types with different CONST bounds, runtime-table ID
rebasing, malformed canonical facts, and the existing unsupported boundary for
pointer-valued record fields. Codegen checks run under both runtimes. Removing
projected bound expressions leaves allocation and emitted code unchanged,
proving layout no longer depends on that syntax. Existing fixtures are unchanged.

4b generalizes classic indexing to addressable field bases using canonical
element widths and the existing slot-address emitter. Named arrays and pointers
share full-word constant-stride expansion; one/two-byte fast paths remain.
Field decay projects as address-of, while indexing retains the field place.
Static address queries are read-only; dynamic addresses and complex compound
destinations are evaluated once and protected across RHS/index calls.

MIR lowers computed address values with existing loads, word arithmetic and
temps. Ordinary indexed accesses retain existing small-stride selection;
strides above 255 expand without truncating the scale. `AdvanceAddress` accepts
the same stored index lanes as indexed materialization. Large indirect offsets
are incorporated into the pointer; verification rejects unprepared offsets
that cannot fit in Y. Indexed pointer-staging elimination cannot move a source
pointer read across unsummarized calls, machine blocks or barriers. Existing
aggregate-copy selection and both NIR/MIR fixture snapshots are unchanged.

Nine execution tests run classic and MIR6502 with ActionCart and Standalone.
They cover BYTE/CHAR/INT/CARD/REAL, record elements, local/global/absolute/nested/
pointer bases, strides 3/6/400/516, the full planned scalar bound matrix,
page-crossing offsets, guarded neighboring bytes, pointer/array-argument decay,
compound updates and exactly-once ordered calls. The root test executor loads
the cartridge's initial arithmetic-service mapping where needed; these are
development-capability executions, not public compiler-profile support or new
Oscar64 cases. Two MIR verifier tests cover stored-word indexes and rejection
of oversized indirect-Y load/store offsets.

4c repairs the shared BYTE compound multiplication and wide-divisor failures.
SemIR reuses ordinary arithmetic-result typing and explicitly carries the
store conversion; NIR computes at that type before casting to the destination.
Bare array updates retain their canonical pointer-cell type. Classic consumes
the same facts and shares captured-place lowering across scalar, pointer,
named-array and embedded-field targets, including indirect shift fallbacks.
The original cartridge confirms integer compound order: capture the address,
evaluate the RHS, read the current value at the captured address, compute and
store. Native REAL's separate existing ordering is unchanged.

Both characterizations are now correct-oracle regressions. The matrix includes
all scalar type pairs in semantic tests, effectful field indexes/RHS writes,
page-crossing array-pointer updates and public VM execution in all six
mode/runtime combinations. The existing discarded-high-product proof is
generalized to Add/Sub/And/Or/Xor with a sole adjacent truncation and immediate
byte store; high-dependent operations and argument-selector shapes remain
untouched. Existing code-size quality limits remain unchanged. See
[the compound typing note](bugs/COMPOUND_ASSIGNMENT_TYPED_LOWERING_GAPS.md) for
cartridge probes and arithmetic-scope details.

### 5. Aggregate behavior and public enablement

Status: 5a initializer traversal, 5b static subobject addresses and 5c copy
validation/public modern enablement complete and validated.

5a shares one canonical typed leaf walker between semantic validation and
SemIR initializer planning. Inline arrays repeat their resolved count/stride;
nested record elements recurse without treating padding as source elements.
The existing static-write and relocation contract is unchanged. Tests cover
recursive declaration order, partial/inferred zero-fill, local/global storage,
REAL and address leaves, BYTE/CHAR/INT/CARD page-boundary lengths, aligned-target
offsets and precise invalid-leaf diagnostics. Public profiles stayed gated in 5a.

5b resolves static subobjects in semantics to the existing SymbolId/addend
contract. Pointer declarations accept exact field decay, address-of and explicit
pointer casts; flat address lists additionally accept indexed/nested subobjects
with low/high selectors and literal byte addends. Dynamic bases/indexes and
implicit pointer-type mismatches are diagnosed. This does not generalize scalar
non-pointer alias declarations. Tests cover allocated and absolute globals and
locals, nested/record-array offsets above 255, inferred initialized arrays,
module-owned storage and leaf-width errors. MIR data lowering now resolves
local absolute/global/local aliases before omitted frame identities can become
unknown relocation targets; ordinary alias-list regression coverage is included.

5c enables Atari modern classic and MIR6502 with ActionCart and Standalone.
Compatibility remains gated. The supported cross-backend element set is BYTE,
CHAR, INT, CARD, REAL and complete supported records; classic's existing
pointer-valued field restriction remains explicit, including pointer arrays.
Large-copy validation exposed excessive MIR scalar-temp expansion and a classic
destination capture clobbered by nested copying calls. MIR now stages large
copies with bounded-size counted-loop IR and a full-size private buffer;
classic stack-protects the destination capture across source evaluation.
Both fixes generalize the existing record-copy paths. Initializer total-extent
overflow is diagnosed before a checked-layout failure can discard its plan.
Small copies also exposed indirect-Y extent failures at field offsets 254-257.
The existing selector now advances the pointer first when required, and copies
reuse scalar access lowering for full-word parent strides. Root tests cover
these offsets and a 260-byte parent stride; the public VM covers the small-copy
offset-255 case as well as large copies.

Root execution tests cover the full scalar/REAL/record length matrix, overlap
in both directions, self-copy, nested/local/pointer/record-array places and
ordered/reentrant copying calls. A public VM fixture varies nine lengths in
all four modern backend/runtime combinations, checking full memory images,
guards, static addresses, partial initialization and exactly-once calls.

- Extend the shared initializer leaf walker through embedded array elements;
  preserve partial initialization, zero-fill, relocations and diagnostics.
- Resolve inline-array subobject address initializers through the shared static
  address/relocation contract before removing slice 3's explicit diagnostic.
- Verify enclosing `RecordCopy`/`CopyBytes` uses the full extent, including
  copies above 255 bytes, self-copy and overlap in both directions.
- Add guarded VM tests for BYTE/CHAR, INT/CARD and other eligible complete
  element types; direct/local/absolute/nested/pointer storage; record arrays;
  lengths 1, 2, 100, 127, 128, 129, 255, 256, 257; offsets around page boundaries;
  exactly-once calls, unchanged neighbors and source buffers.
- Check module-visible types/constants and negative/profile diagnostics.
- Enable the selected public profiles only after these contracts pass.

### 6. Structurally faithful Oscar64 stage-5 ports

Status: pending.

Port `structarraycopy.c` and `structmembertest.c` in their original record
structure. Keep conditional calls observable, preserve inline member arrays,
and derive full memory/counter oracles independently in Rust. Run the available
profile/runtime matrix explicitly: modern-only array-field fixtures must not
be counted as Compatibility executions. Keep ports separate from compiler
implementation commits when committing is requested.

## Validation and handoff

Initial semantic/layout slice validated on 2026-09-05:

- Root suite: 2,671 passed, 0 failed, 22 existing ignored. This adds eight
  semantic tests and one compiler API gate test to the baseline.
- Isolated VM suite: 100 passed, 0 failed, 0 ignored. Existing Oscar64
  coverage remains 4,380 cases in 24 tests; no stage-5 ports are counted.
- Explicit NIR snapshot check passed; NIR sweep passed all 33 fixtures with
  no load, semantic, lowering, verification or optimization failures.
- `git diff --check` passed. Snapshot edits are the printer-only field
  metadata changes described above.

Slice 2 and its source-span prerequisite validated on 2026-09-05:

- Root suite: 2,681 passed, 0 failed, 22 existing ignored.
- Isolated VM suite: 100 passed, 0 failed, 0 ignored; Oscar64 coverage unchanged.
- Explicit NIR snapshots passed unchanged; all 33 NIR sweep fixtures passed.
- Ten tests added since the foundation: seven dependency tests, two parser
  span tests and one emitted-code equivalence test spanning all six public
  mode/runtime combinations. The inline-string listing expectation now uses
  the literal's actual source column (metadata-only change).

Slice 3 validated on 2026-09-05:

- Root suite: 2,694 passed, 0 failed, 22 existing ignored; 13 new development
  tests exercise the experimental semantic/SemIR/NIR capability.
- Isolated VM suite: 100 passed, 0 failed, 0 ignored. These preserve existing
  execution coverage; they do not yet execute embedded record arrays. Oscar64
  coverage remains 4,380 cases in 24 tests with no stage-5 ports counted.
- Explicit NIR snapshots passed; all 33 public sweep fixtures passed with no
  load, semantic, lowering, verification or optimization failures. The new
  experimental NIR snapshot runs through its dedicated test, not the public
  sweep. Existing fixture expectations are unchanged.
- `git diff --check` passed.

Slice 4a validated on 2026-09-05:

- Root suite: 2,700 passed, 0 failed, 22 existing ignored; six new canonical
  classic layout-projection tests passed.
- Isolated VM suite: 100 passed, 0 failed, 0 ignored. Oscar64 coverage remains
  4,380 cases in 24 tests; no stage-5 ports or embedded-array executions added.
- Explicit NIR snapshots passed unchanged; all 33 NIR sweep fixtures passed.
- `git diff --check` passed. Existing emitted-code and fixture expectations
  are unchanged; the bound-removal regression verifies canonical allocation in
  both runtime-linking paths.

Slice 4b validated on 2026-09-05:

- Root suite: 2,713 passed, 0 failed, 22 existing ignored. Thirteen new tests:
  nine cross-backend/runtime execution tests, two MIR verifier tests and two
  explicit characterizations of the open compound-operation defects. Those
  characterizations are not successful semantic executions of the reproducers.
- Isolated VM suite: 100 passed, 0 failed, 0 ignored. Oscar64 coverage remains
  4,380 cases in 24 tests; no stage-5 ports added.
- Explicit NIR snapshots and all 33 NIR sweep fixtures passed. Existing NIR
  and MIR snapshots and emitted-code expectations remain unchanged.
- `git diff --check` passed. Public embedded-array profiles remain disabled.

Slice 4c validated on 2026-09-05:

- Root suite: 2,722 passed, 0 failed, 22 existing ignored. Nine new tests:
  three execution tests, two semantic/NIR typing tests and four MIR narrowing
  guards/consumer tests. Both former characterizations now execute the correct
  oracle; the existing embedded compound matrix also covers multiply/divide/MOD.
- Isolated VM suite: 101 passed, 0 failed, 0 ignored. The new test adds 30
  public-language VM executions, each checking ten operators across BYTE,
  CARD and INT with guards and counted calls. Oscar64 remains 4,380 cases in
  24 tests; no stage-5 ports are counted.
- Explicit NIR snapshots and all 33 NIR sweep fixtures passed. Existing
  NIR/MIR snapshots and CIRCLE/WARPDEM quality limits remain unchanged. The
  broad corpus ledger increases from 315 to 316 verified entrypoints solely
  because of the new runtime fixture; its five declared non-entrypoints remain.
- Original-cartridge probes confirm RHS-before-load ordering through scalar,
  array and captured-pointer targets. The companion arithmetic probe records
  the cartridge's negative MOD quirk without redefining it in this slice.
- `git diff --check` passed. Public embedded-array profiles remain disabled.

Slice 5a validated on 2026-09-05:

- Root suite: 2,727 passed, 0 failed, 22 existing ignored. Five new tests
  exercise recursive/partial initialization, REAL/relocations, the complete
  scalar length matrix, aligned offsets/paths and invalid scalar leaves.
- Isolated VM suite: 101 passed, 0 failed, 0 ignored; Oscar64 unchanged.
- Explicit NIR snapshots and all 33 NIR sweep fixtures passed unchanged.
- `git diff --check` passed. Public embedded-array profiles remain disabled.

Slice 5b validated on 2026-09-06:

- Root suite: 2,733 passed, 0 failed, 22 existing ignored. Six new tests
  cover static pointer/list relocations, local alias normalization, inferred
  record arrays, module identities and rejected runtime/mistyped addresses.
- Isolated VM suite: 101 passed, 0 failed, 0 ignored; Oscar64 unchanged.
- Explicit NIR snapshots and all 33 NIR sweep fixtures passed unchanged.
- `git diff --check` passed. Public embedded-array profiles remain disabled.

Slice 5c validated on 2026-09-06:

- Root coverage: 2,740 tests verified passing, 22 existing ignored. The full
  root run passed every integration target. One old selector unit expected
  the former offset-range fallback; after updating it to assert pointer
  advancement, the complete library rerun passed all 2,408 tests.
- Seven tests added: five execution tests (320 copy executions across both
  backends/runtimes), one bounded MIR-growth/CFG test and one initializer
  overflow test. Existing profile gates now assert modern acceptance and
  Compatibility rejection; no failing tests were ignored.
- Isolated VM suite: 102 passed, 0 failed, 0 ignored. The new test covers
  36 public modern executions, including small offset-255 copies. Its focused
  rerun also passed. Oscar64 remains 4,380 cases in 24 tests; no stage-5 ports
  are counted.
- Explicit NIR snapshots and all 33 NIR sweep fixtures passed unchanged.
  The broad corpus is 317 verified entrypoints plus the same five declared
  non-entrypoints; only the new runtime fixture changes that ledger. Existing
  MIR snapshots and CIRCLE/WARPDEM quality limits remain unchanged.
- `git diff --check` passed. Fixture/profile changes are additive language
  coverage and deliberate modern enablement; selector/capture changes are
  shared backend correctness and bounded-expansion fixes.

Slices 1-5 provide layout, executable places, aggregate initialization, static
subobject relocations, full record-copy behavior and public modern support.
Structurally faithful Oscar64 stage-5 ports remain slice 6 and are not included
in the compiler implementation commits or current conformance totals.

After each semantic/lowering slice:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Before handoff, also run `cargo test --locked --no-fail-fast` from
`tools/vm-runtime-tests`. Preserve the baseline's active coverage and existing
ignored checks; do not silently skip new failing modes or report unexecuted
cases as passing. State whether fixture changes are language-contract changes,
printer changes or bug fixes.

Update `SEMANTIC_INVARIANTS.md`, `SYNTAX_EXTENSIONS.md`, `CODEGEN_PROFILES.md`,
the aggregate initializer/record-copy notes, and the Oscar64 coverage tables
when their corresponding implementation slice is complete. This plan records
progress but does not claim pending slices are implemented.
