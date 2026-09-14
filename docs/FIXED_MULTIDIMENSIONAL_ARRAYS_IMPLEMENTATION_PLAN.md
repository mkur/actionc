# Fixed-size multidimensional arrays

Status: implementation in progress. Slice 1 adds declaration syntax and checked
canonical shapes behind a semantic rollout capability. Public enablement remains
in slice 5, after indexing and executable backend validation.
Baseline: `8262140`, after Amiga 32-bit decimal output.

Deliver fixed-size arrays indexed with one coordinate per dimension, using
the same source on Atari modern classic, MIR6502 and MIR68K. Retain existing
one-dimensional behavior and benchmark references. Implement and commit the
slices separately, with public enablement after the backend checks pass.

## Source contract

```action
CONST Rows=3,Columns=4
LONGINT ARRAY matrix(Rows,Columns)
BYTE ARRAY volume(2,3,5)
CARD ARRAY values(2,3)=[1 2 3 4 5 6]
TYPE Tile=[BYTE tag INT ARRAY pixels(3,4)]
Tile tileData

PROC Main()
  BYTE row,column
  FOR row=0 TO Rows-1 DO
    FOR column=0 TO Columns-1 DO
      matrix(row,column)=LONGINT(row)*100+column
    OD
  OD
  matrix(1,2)==+7
  volume(1,2,4)=255
  tileData.pixels(2,3)=42
RETURN
```

This is a modern-profile extension. Compatibility mode, including the original
cartridge language contract, diagnoses multidimensional declarations. Modern
source may use either legacy file organization or named modules. No new
keyword, command-line mode or runtime library is required.

### Dimensions and indexing

- A declaration supplies all dimensions as positive compile-time integer
  expressions. Resolve CONST visibility, imports, shadowing and layout queries
  through the existing semantic machinery. Do not accept runtime bounds,
  inferred missing dimensions, zero/negative dimensions or an empty list.
- Support fixed rank two and higher through the same representation; do not
  hard-code a two-dimensional special case. Cover rank three in execution
  tests. Existing one-dimensional declarations and inferred lengths retain
  their behavior.
- Dimensions are element counts, indexes start at zero, and the last index
  varies fastest. `values(1,0)` above is 4. There is no row padding beyond the
  existing element stride and target layout rules.
- Require exactly one index per dimension. Reject `matrix(row)` and
  `matrix(row,column,extra)` with an array-rank diagnostic. They are not calls
  or row views. A scalar pointer or unsized one-dimensional ARRAY still takes
  one index; consecutive applications are not multidimensional syntax.
- New multidimensional indexes must have integer types under the existing
  integer/enum conversion policy. Reject REAL and pointer indexes. Preserve
  the current one-dimensional index policy rather than changing it incidentally.
- Reject a statically known negative or out-of-range coordinate for the new
  form, checking each axis separately. For example, `values(0,3)` is invalid
  even though its flattened offset lands inside the object. Do not add runtime
  bounds checks: callers remain responsible for dynamic coordinates.

For dimensions `d0,d1,...,dn`, normalize the element index as:

```text
linear = (...((i0 * d1 + i1) * d2 + i2)...) * dn + in
address = captured_base + linear * element_stride
```

Evaluate the base once, then coordinates once from left to right. Finish
evaluating each coordinate under its ordinary source arithmetic rules before
converting it to the target's index/address calculation width. Perform the
compiler-generated products and sums at that width, not at BYTE width because
the loop variables happen to be BYTE. A conversion cannot repair overflow
inside an explicitly written coordinate expression.

An indexed assignment captures the complete destination before its RHS;
compound assignment retains the existing load/RHS/store ordering. Calls inside
coordinates can change the array descriptor, other coordinates or memory, but
cannot change an already captured base or duplicate an earlier evaluation.
Out-of-range dynamic addresses retain the existing unchecked wrapping address
arithmetic; dimensions are not proof that a runtime pointer is in bounds.

### Storage, initialization and queries

Support globals, routine/lexical locals and embedded fields using their current
storage models. Atari static locals and native automatic locals remain distinct.
Use the existing fixed-array element eligibility, complete record layout,
alignment and volatility rules. This adds no new scalar element type or backend
support for an element type it currently rejects.

Check each dimension, total element count, element stride, byte extent and
fixed-address endpoint before allocation. Use checked arithmetic; no wrapping,
saturating result or truncated 16-bit descriptor count is a successful layout.
Apply the selected target's SIZE/address limits and the existing compiler layout
limits. Native frame displacement and executable placement limits still produce
their existing explicit diagnostics; a valid source shape does not promise that
every object fits on the stack or into an Atari program image.

Flat initializer lists follow row-major element order, reusing existing leaf
typing, partial initialization, zero filling and relocation rules. Nested
records use their existing recursive scalar-leaf ordering. Reject excess values
and nested row-list syntax in this milestone. Audit scalar arrays as well as
record arrays so their accepted partial lists initialize the full declared
extent. New fixed-address multidimensional bindings use the existing explicit
fixed-array address syntax and checked extent rules.

`ELEMENTS(matrix)` returns 12 and `SIZEOF(matrix)` returns 48. Queries on inline
fields use their full extent; `ALIGNOF` and enclosing `OFFSETOF` remain governed
by the target layout. Query operands are unevaluated. Do not add a dimension
query intrinsic yet; named constants can express the dimensions.

`@matrix(row,column)` produces an element pointer. Static addresses of constant
coordinates resolve to the existing storage identity plus checked byte addend,
including addresses inside record fields and initializer relocation leaves.
Enclosing record copies include every byte of an embedded multidimensional
field; this does not introduce whole-array assignment or array return values.

### Descriptor and parameter policy

Dimensions describe an array place's declared interpretation, not a new runtime
array value. Named arrays retain their existing mutable pointer-descriptor
behavior and element-pointer conversion/rebinding rules. Rebinding changes the
base, never the declared dimensions; it neither copies elements nor adopts the
source declaration's shape. Matching element types are still required wherever
the existing conversion rules require them. No runtime allocation-size or shape
check is implied. Inline fields remain non-rebindable storage.

A shaped array may decay to a compatible element pointer or an existing flat
ARRAY parameter. The recipient then has its existing one-dimensional view.
Do not infer dimensions from an initializer, a caller, or a pointer's previous
assignment. Test descriptor mutation and shape erasure explicitly; optimizers
must not assume that a declared backing remains the current base.

**Shaped parameters are deferred:** reject rank-two-or-higher formal ARRAY
dimensions, including callable-pointer prototypes, with an explicit unsupported
diagnostic. `CallableType.params` and `CallableParamTypeRef` currently erase array
shape, so checked shaped calls need their own source-signature design covering
direct calls, indirect calls and raw-pointer escapes. Existing one-dimensional
parameters are unchanged. The matrix/DCT acceptance variants can use shaped
global storage and existing flat helpers without that ABI work.

Other non-goals: variable-length or jagged arrays, partial indexing, slices,
pointer-to-array types, a new array value/copy ABI, runtime bounds traps,
dimension reflection in native artifacts, a 65816 VM, or new optimizer passes.

## Inspected implementation and ownership

| Area | Current code and required treatment |
| --- | --- |
| Declaration syntax | `src/ast.rs` has `DeclEntry.size: Option<Expr>`; `src/parser.rs::parse_decl_entries` reads one size expression. Add a structured dimension list without confusing declaration commas, callable prototypes or initializer lists. Migrate visitors and synthetic declarations explicitly. |
| Application syntax | `ExprKind::Call` already holds all arguments. `src/semantic.rs` and `src/semantic/ir.rs` recognize indexing only when `args.len()==1`. Resolve the callee's meaning before rank validation; never route a malformed array access into routine-call lowering. |
| Canonical shape | `ArrayType.length`, `SemanticModel.array_lengths`, `SemanticArrayLayout` and inline `RecordFieldStorage` currently hold a single bound. Establish one authoritative shape; derive flattened count for existing consumers. Reuse SymbolId, FieldId and ArrayLayoutId rather than name-based side tables. |
| Semantic places | `src/semantic/array_places.rs`, `subject.rs`, `ir.rs` and their walkers must retain resolved coordinates, element type and shape. Cover reads, lvalues, compound stores, address-of, module-qualified symbols and nested/indexed fields. |
| Initialization | `src/semantic/initializers.rs` and `static_addresses.rs` own leaf order and constant subobject addresses. Extend the canonical layout walk; do not reparse initializer text in a backend. |
| NIR | `src/nir/lowerer.rs` currently lowers one index to `NirPlaceKind::Index`. Normalize multiple coordinates to ordinary typed temps, casts, arithmetic and one element address. `verifier.rs` checks the resulting storage, widths and use-def facts. |
| Classic 6502 | `src/codegen/semir.rs` projects typed SemIR; `array_place.rs`, `array.rs` and `storage.rs` implement storage/addressing. Consume canonical flattened layout and ordered computations through that bridge. Do not teach legacy AST codegen to resolve source dimensions independently. |
| MIR targets | MIR6502 and MIR68K consume verified normalized computation and complete element strides. 65816 participates in existing target-layout/NIR checks; this plan makes no new executable 65816 claim. |
| Symbols/artifacts | Native `ArrayInfo` and version-2 artifacts already expose element width, stride, flattened count and descriptor/backing information. Preserve that format and export the total element count. Per-axis reflection is not required to execute or test the new form. |

SemIR owns rank, source legality, shape and evaluation order. NIR owns normalized
typed computation and conservative storage/effect facts. Each MIR owns target
addressing and register choices. Emission owns final bytes and relocations.

Prefer a canonical shape type with an explicit existing rank-one/unknown-bound
case and fixed dimensions, plus checked count/stride calculations. Avoid keeping
an independently mutable dimension vector and product. Existing `length`
projections must derive from, or be validated against, that shape. Preserve
rank-one printed output unless an intentional contract change requires otherwise.

Share the typed index-normalization rules between NIR lowering and the classic
SemIR projection. Classic must not recreate `i*columns+j` as an ordinary narrow
source expression or re-evaluate a descriptor during a coordinate call. Reuse
its existing prepared-expression/address-staging mechanisms where appropriate.

Verifier-clean executable NIR should need no multidimensional opcode or source
dimension strings. Any new non-executable shape facts must have checked rank,
nonzero dimensions, consistent count/stride/extent and stable storage identity.
Do not weaken alignment or alias rules based on an array's initial descriptor.

## Implementation slices

### 1. Parse dimensions and establish checked canonical shape

Add a private semantic rollout capability, following the embedded-array
precedent; expose no temporary CLI flag. Parse dimension expressions with their
own spans, and diagnose the feature in public profiles while implementation is
incomplete. Preserve rank-one declarations, STRING sizes, callable signatures,
initializer syntax and declaration groups.

Resolve fixed dimensions through existing constant/dependency analysis. Migrate
named/inline shape and layout consumers to checked products, using the actual
target stride. Audit `ArrayType::total_width_bytes`, which currently saturates,
and avoid making it an authority for accepted multidimensional layouts.

Acceptance: parser and semantic tests for ranks two/three, grouped declarations,
imported/shadowed CONSTs, target-dependent SIZEOF bounds, cycles, wrong types,
missing bounds, zero/negative dimensions, overflow and compatibility diagnostics.
Layout tests cover Atari and both 65816 layouts plus MC68000. Existing rank-one
fixtures remain unchanged.

### 2. Resolve element places, initialization and pointer behavior

Classify array application by resolved identity before argument-count checks.
Carry typed coordinate lists and canonical shape into SemIR. Update visitors,
effect discovery, address-of, compound destinations, static subobject resolution,
initializers and layout queries together. Preserve display names as metadata.

Specify the capture order in the typed representation shared by its consumers.
Keep ordinary named-array rebinding and inline-field restrictions explicit.
Reject partial indexing, excess coordinates and shaped parameters before they
can reach code generation. Cover real multi-argument functions, casts, layout
intrinsics and arrays of callable pointers so classification remains unambiguous.

Acceptance: semantic/SemIR fixtures for direct, local, absolute, qualified,
pointer-record and nested field cases; partial initialization and relocations;
unevaluated queries; record elements/copies; and each unsupported form. Public
profiles still reject the extension under the rollout gate.

### 3. Normalize indexing and tighten NIR verification

Capture base and coordinates in order, explicitly convert the captured indexes
to the target calculation width, and construct the row-major index with normal
typed operations. Emit one element load/store/address using the complete stride.
Carry full flattened extents into global/local backing facts, static writes and
automatic storage. Keep descriptor size words separate from canonical counts;
never silently truncate a large native count to a legacy 16-bit field.

Add negative verifier tests for malformed retained shape/storage facts and
incorrect arithmetic/index widths. Existing use-def, alignment and effect
verification must remain at least as strict. There must be no executable raw
application, coordinate text or unresolved shape reaching either MIR backend.

Acceptance: raw/optimized NIR snapshots, all four target-layout projections,
base/index/RHS ordering, volatile coordinates, pointer changes during calls,
static relocations, offsets over 255 and native offsets over 65535. A compile-
time out-of-bounds check must not become a false optimizer proof for dynamic
coordinates or rebound descriptors.

### 4. Complete all executable backend paths

Project canonical total counts and typed index computations into modern classic,
including local storage, fixed addresses and inline record fields. Reuse existing
6502 effective-address capture and protected compound destinations. MIR6502 and
MIR68K should normally reuse their one-dimensional normalized index paths; fix
general missing behavior only when a focused regression demonstrates it.

Acceptance: equivalent flat and shaped programs agree in modern classic and
MIR6502 under cart/standalone runtimes, and in raw/optimized MIR68K. Include
mixed widths, record strides, unaligned addresses where supported, pointer
rebinding, pointer decay, nested effectful coordinates and recursive native
locals. Check ABI/stack guards and exact volatile-access order. Validate a HUNK
program at different section bases as well as the bare native image path.

### 5. Enable the modern feature and document its boundaries

Enable it through the public modern compiler API/CLI once slice 4 passes.
Retain the explicit compatibility and shaped-parameter diagnostics. Add public
tests that use LF/CRLF source/includes and imported constants, and confirm that
native emitted count/stride metadata describes all flattened elements without
changing version-2 serialization. Existing artifacts remain loadable.

Update the language/storage documentation, semantic invariants, NIR contract and
target execution notes. Add a small readable example with rectangular dimensions
and an embedded field. Label the examples as actionc modern extensions.

Acceptance: public profile matrix, meaningful diagnostics before emission,
existing one-dimensional fixtures/benchmarks, symbol-based native inspection,
and documented example builds. Commit this usable language slice independently
of the benchmark adaptations.

### 6. Exercise matrix1 and DCT without replacing their baselines

Add dedicated shaped variants; retain the existing flat/pointer algorithms,
pinned C sources, reference generators and measurement baselines. Share reference
parsing/execution helpers where practical. Avoid broad textual substitutions that
silently change a different access; new instrumentation must assert its anchors.

For matrix1 preserve its actual memory ordering:

| Reference buffer | Shaped declaration | Access in multiplication |
| --- | --- | --- |
| A, row-major | `matrixA(Rows,Inner)` | `matrixA(row,k)` |
| B, column-major | `matrixB(Columns,Inner)` | `matrixB(column,k)` |
| C, column-major | `matrixC(Columns,Rows)` | `matrixC(column,row)` |

This retains the reference byte order without transposing vectors or changing
the multiply/add sequence. Check all 252 existing cases: LONGINT, BYTE, INT and
CARD across 10x10x10, 3x7x5 and 2x129x1 configurations. Preserve narrowing after
each accumulation and signed checksum behavior. A known rectangular fixture must
independently distinguish row-major from column-major indexing.

For jfdctint use `LONGINT ARRAY block(8,8)`, with `block(row,k)` in the row pass
and `block(k,column)` in the column pass. Preserve every arithmetic operation,
wrapping conversion, Descale rule and pass boundary. Compare all 181 cases,
including initialized input, row-pass state, final block and checksum/status.
Update capture code to use two coordinates or an explicit existing flat view.

Run each full reference corpus for the new variant in the supported modern 6502
configurations and raw/optimized 68K. Compile once per source/type/shape/mode,
then reuse the artifact in isolated VMs for vector cases. Address native state
through symbols and serialize numerical values in target byte order. Reuse the
existing 6502 harness boundary; do not introduce 6502 addresses into shared
algorithm source or start a repository-wide harness migration in this slice.

Record code size, instruction counts and stack traffic for the ordinary new
variants alongside the retained flat versions, using existing measurement tools.
Investigate unexpected regressions but do not turn this milestone into a new
optimization campaign or promise GCC parity.

## Validation and completion

During semantic/NIR slices, run the repository-required checks after the affected
focused tests:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Run the new targeted 6502/native VM tests when their backend slices change.
After benchmark integration:

```sh
python3 tools/generate_matrix1_vectors.py --check
python3 tools/generate_jfdctint_vectors.py --check
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test matrix1 --test jfdctint
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --test matrix1 --test jfdctint
```

Include any separately named multidimensional targets introduced by the slices.
Backend-only and benchmark-only commits use their affected checks; do not repeat
the full compiler suite for every fixture edit. Preserve complete cross-platform
CI coverage and report a dispatched run without waiting for it to finish.

Normalize host text before newline-sensitive instrumentation, exercising LF and
CRLF through actual compilation. Guest outputs, memory and binaries remain exact.
Explain every changed fixture as a semantic/IR contract change, printer change
or bug fix. Existing rank-one snapshots should normally stay byte-for-byte stable.

Completion requires public modern execution on both 6502 backends and MIR68K,
checked shape/layout diagnostics, exact reference states for both new benchmark
variants, conservative effects and descriptor handling, unchanged original
fixtures, updated documentation and separately reviewable commits. A fresh
manual vAmiga run is useful additional evidence; it is not a replacement for
the compiler, ABI, HUNK and reference-vector checks.
