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

Status: in progress; the initial layout foundation is implemented.

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

Remaining prerequisite: named-module type/CONST dependency ordering. The
current resolver lays out all records before evaluating that module's CONST
declarations, so even this source still fails under the experimental capability:

```action
MODULE Data
PUBLIC CONST Count=100
PUBLIC TYPE Buffers=[INT ARRAY x(Count),y(Count)]
ENDMODULE
```

It reports `constant Count is not available before its declaration` (with
backticks around the name). Legacy global/local constants and constants from
already resolved dependency modules work. Resolve this dependency gap without
breaking existing layout-query constants or inventing silent retry/fallback
values; add same-module and cyclic-dependency tests. This is unfinished feature
support, not a regression in a publicly enabled form.

### 3. Array places, SemIR and NIR

Status: pending.

- Generalize indexing, decay and layout queries from array symbols to array
  places, including nested/indexed record fields.
- Carry resolved shape into SemIR; lower address calculations using existing
  NIR field/index/address operations.
- Preserve full storage extent without permitting scalar array loads/stores.
- Tighten NIR validation of extents, stride and addressable subobjects. Add
  structural/snapshot tests for direct, pointer and nested cases and ordered
  effectful base/index evaluation.

### 4. Backend support

Status: pending.

Classic: project canonical layouts; generalize field-based indexing and array
decay through existing effective-address/staging paths. MIR6502: retain inline
subobject backing and consume the normalized address with existing selectors
and fallbacks. Verify loads, stores and compound assignments on both backends
and runtimes, especially offsets above 255 and pointer changes during calls.

### 5. Aggregate behavior and public enablement

Status: pending.

- Extend the shared initializer leaf walker through embedded array elements;
  preserve partial initialization, zero-fill, relocations and diagnostics.
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

These checks validate the foundation and preservation of existing behavior;
they do not establish executable embedded-array support. Slice 2's module
dependency work and slices 3–6 remain unfinished.

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
