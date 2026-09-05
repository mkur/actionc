# Fixed-length arrays embedded inside records

Status: implementation started, 2026-09-05. Public syntax remains disabled.
Baseline: `6c73c1d`; 2,662 root tests passed (22 existing ignored), 100 VM
tests passed, including 4,380 Oscar64 cases. No stage-5 ports have been added.

## Purpose and profile policy

Enable structurally faithful ports of Oscar64 `structmembertest.c` without
replacing embedded arrays with pointer fields or separate backing tables.
`structarraycopy.c` also belongs to stage 5, but needs no new source construct.

The initial policy is modern-only: modern classic and MIR6502, with both
ActionCart and Standalone linking. Compatibility will diagnose the extension
during semantic analysis. This is the proposed default pending an explicit
different profile choice. Keep the public modes disabled until semantic
lowering, both backends and aggregate behavior are verified. An experimental
semantic capability may be used to exercise development slices; it is not a
claim of end-to-end compiler support.

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

Static initializers referring to inline-array subobjects are explicitly gated
until slice 5: the existing scalar initializer path cannot yet encode their
addresses. This includes explicit address-of/casts and element addresses, not
just implicit decay. Compile-time layout queries remain allowed. No public
profile exposes this intermediate capability.

### 4. Backend support

Status: 4a canonical layout projection and 4b executable array places complete.
4c shared compound-operation typing follow-up remains before public enablement.

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

4c is required by extra stress cases: BYTE compound multiplication produces
invalid NIR, and division by a CARD-valued 256 produces `$FF` instead of zero
on MIR. Both reproduce with ordinary named arrays and are retained as two
explicit characterization tests, not counted as successful semantic cases.
Follow [the shared compound typing note](bugs/COMPOUND_ASSIGNMENT_TYPED_LOWERING_GAPS.md):
reuse ordinary SemIR binary typing, lower computation and store conversion
separately, and settle old-value read ordering before expanding the execution
matrix. Do not add a special backend implementation for inline arrays.

### 5. Aggregate behavior and public enablement

Status: pending.

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

Slice 4b adds executable embedded-array access under the experimental capability.
The compound-operation typing/order follow-up in slice 4c remains open.
Aggregate initialization, subobject address relocations, full record-copy
validation and public enablement remain slice 5. Structurally faithful Oscar64
stage-5 ports remain slice 6.

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
