# Untagged unions: implementation plan

Status: accepted; implementation started on `main`, 2026-09-09.
Inspected baseline: `2994212`, after the algebraic-data-types delivery.
Public UNION support remains disabled until the acceptance matrix below passes.

## 1. Objective and boundaries

Add nominal, untagged unions: multiple typed views of the same storage. They
complement checked VARIANT values; they are not variants with checking disabled.
Reuse aggregate identities, target layout, field access, copies, LET and call
boundaries. No allocator, new arithmetic rules, active-member tracking, union
runtime helper, or source-specific optimizer is included.

Modern classic and MIR6502 must support the declared common Atari subset with
both cartridge and standalone runtime. Compatibility rejects the syntax. Native
68k/65816 receive layout/access/ABI/frame canaries within existing capabilities,
not an unsupported execution claim. Runtime selection does not choose semantics.

Commit each verified major slice separately. Keep incomplete capabilities
internal and disabled in public profiles; do not add temporary CLI switches.

## 2. Source contract

```action
TYPE BytePair=[BYTE first,second]
TYPE WordView=UNION [
  CARD word
  INT signedWord
  BYTE ARRAY bytes(2)
  BytePair parts
]
WordView view

PROC Main()
  view.word=$1234
  view.bytes(0)=$78
  ; view.word is now $1278 on Atari
RETURN
```

- UNION is contextual after `TYPE name=`. Reuse bracketed record-field syntax;
  do not reserve UNION globally or introduce `UNION name ... ENDUNION`.
- Every declared member starts at byte offset zero, including each entry of a
  grouped declaration. `BYTE first,second` declares two overlapping bytes; use
  a record member for sequential fields. No anonymous-member name injection.
- Size is the largest member extent, rounded to maximum member alignment.
  Width, alignment, array stride and endianness come from TargetLayout. Reject
  empty unions, zero-sized/incomplete members, duplicate names, inline layout
  cycles and checked-extent overflow. Pointer recursion remains finite.
- Each declaration/concrete generic instance is nominal. Equal layouts do not
  permit implicit whole-value assignment or mismatched call signatures. Imports
  and aliases preserve the defining identity.
- A member read interprets the stored representation as that member's type.
  Reading another member is intentional type punning, not a numeric conversion
  or an inactive-member error. Ordinary typed arithmetic widths are unchanged.
- A member assignment writes only its selected extent. It does not clear the
  remainder, select an active alternative, or validate other members. Compound
  assignment uses the selected scalar member's ordinary semantics.
- Whole-union assignment copies every byte, including padding, using existing
  overlap-safe value semantics. Evaluate addresses/arguments exactly once in
  the existing order. Whole-union arithmetic, comparison, scalar casts and CASE
  selectors are not introduced; `CASE view.word OF` is ordinary scalar CASE.
- Storage duration/initialization follows ordinary aggregates. There is no
  implicit clear on a member change and no promise that unwritten bytes are
  zero. Initialize every byte intended for inspection. No active-member fact
  gives the optimizer permission to ignore an overlapping write.
- LET and pattern bindings are immutable whole-value snapshots with existing
  write/address/alias/machine-code restrictions. Pointer values share pointees.
  Direct and exactly typed indirect value parameters/results use callee-private
  parameters and caller-owned results; Atari activation/reentrancy is unchanged.
- SIZEOF/ALIGNOF use the complete union layout; OFFSETOF is zero for direct
  members. Target-native layouts are not a portable serialization or foreign
  aggregate call ABI. No packing/alignment override is included.

### Initial member set and composition

Support integers, enums, data pointers, ordinary records, fixed-length array
members and nested unions. Arrays are inline storage, not new owning array
values. Reuse existing backend type limits, including classic wide-integer limits.

Initially reject inline variants, REAL and callable pointers, transitively through
records, arrays and generic applications. They need additional rules for reading
potentially invalid representations. Ordinary data pointers to such types remain
pointer views; this does not establish pointee validity or memory safety.

A supported union may itself be a VARIANT payload. The outer variant retains
its normal validation and its union binder is an immutable value snapshot.
There are no union constructors or union payload patterns.

Generic definitions reuse the existing finite specialization mechanism:

```action
TYPE Overlay<T,U>=UNION [T first U second]
```

Check member eligibility after substitution. No generic routines or inference
of omitted type arguments are part of this work.

### Initializers and low-level storage

The current scalar-leaf initializer walker traverses every record field. It
must not silently initialize all overlapping union views.

- Reject positional initializer lists for unions and aggregates containing
  inline unions. Runtime member assignment and whole-value copies suffice.
- Do not invent "initialize the first member" behavior. Named static union
  initialization is a separate language extension.
- Reuse ordinary RAM-backed absolute declarations, storage aliases and typed
  pointer access; distinguish these storage bindings from initializer lists.
- Reuse object-level VOLATILE handling after selected-width and ordering tests.
  Whole-value volatile copies explicitly access the complete extent. They are
  not atomic and do not promise safe bulk reads of arbitrary hardware registers.
- Preserve existing restrictions on volatile pointer declarations and member
  qualifiers. Do not silently drop qualifiers through union views or snapshots.

## 3. Architecture and reuse

| Existing component | Union work |
| --- | --- |
| `ast.rs`, TYPE parser | Union definition using existing member declarations |
| `semantic/declarations.rs`, `semantic.rs` | Shared dependency resolver and field validation; sequential versus overlapping placement |
| `semantic/layout.rs`, aggregate identity/field IDs | Canonical aggregate kind, extent, alignment and field ownership |
| `semantic/initializers.rs`, aggregate validation | Explicit union boundaries; no walk over all overlapping representations |
| `semantic/ir/aggregate.rs` | Existing snapshots, copies, argument/result preparation |
| NIR places/effects/storage optimization | Ordinary typed fields, loads/stores, CopyBytes and conservative overlapping regions |
| Classic SemIR projection, MIR backends | Consume resolved offsets/widths; no AST union reconstruction |
| `semantic/generics.rs`, module interfaces | Existing finite nominal instance cache and defining identities |

Do not rename record-named modules merely because they also carry aggregates.
Source meaning stays in semantic analysis/SemIR. NIR needs no union-specific
executable operation or backend-specific ABI. Distinct FieldIds are identities,
not proof that storage is disjoint. Preserve verifier-clean NIR before and after
optimization and conservative behavior around calls, pointers, absolute storage,
volatile access and machine blocks.

## 4. Delivery slices

### Slice 1 — Syntax, identity and canonical layout

Add TypeDefinition::Union and an independent internal capability gate. Generalize
field placement without duplicating validation. Preserve explicit aggregate kind
where semantic walkers need it. Test nominal identity, layout queries, grouped
members, packed/natural layouts, pointer widths, recursion and invalid declarations.
Public profiles remain closed; parser acceptance is not executable support.

### Slice 2 — Typed access and alias correctness

Lower member reads/writes, compound assignments and addresses through existing
typed field places. Audit existing storage/alias optimizers immediately: writes
through one member invalidate overlapping views, regardless of field identity.
Preserve pointer/call/machine/absolute effects. Generalize existing region facts
only where necessary; do not introduce a union optimizer.

### Slice 3 — Aggregate composition and snapshots

Exercise records/arrays/pointer-backed union storage, complete copies, self-copy,
both overlap directions, immutable LET, direct/typed-indirect value arguments and
results. Verify independent parameter copies, captured argument values, exactly-once
destination/callee evaluation and unchanged Atari storage lifetime.

### Slice 4 — Initialization and low-level storage

Make positional initializer rejection transitive and explicit. Reuse RAM-backed
absolute declarations and aliases, without changing `=` storage-binding semantics.
Verify object-level volatile member widths, alias qualifier propagation, skipped
reads, compound-access ordering and complete-copy behavior. Preserve existing
volatile pointer/member restrictions and target alignment limits.

### Slice 5 — Generics, modules and variant composition

Extend the existing generic cache, exports and type applications. Check substituted
member eligibility, exact signatures, import identity, finite recursion and limits.
Verify unions as variant payloads/bindings. Reject prohibited members hidden in
records, arrays or generic instances; pointer barriers remain explicit.

### Slice 6 — Backend/runtime acceptance

Run independent memory oracles across modern classic/MIR6502, both Atari runtimes,
and raw/optimized NIR paths where supported. Cover alternating views, signed bits,
preserved trailing bytes, nested arrays, pointer aliases, calls, volatile accesses,
copies crossing pages and both overlap directions. Check native layout/access and
aggregate ABI/frame lowering, including differing pointer widths and endianness.
Compatibility and unsupported backend cases must fail before emission.

### Slice 7 — Documentation, costs and public enablement

Publish examples, representation rules, restrictions and the support matrix.
Compare simple union programs with equivalent existing memory aliases: direct
access must not add tag checks, implicit clears or a union runtime helper. Record
aggregate-copy costs separately; the outstanding ADT initialization/copy optimizer
work is not part of this delivery. Enable public modern support only after gates.

## 5. Verification

For relevant implementation slices:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test --no-fail-fast
cargo check --all-targets
```

For executable slices and final enablement, also run the pinned suite:

```sh
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --no-fail-fast
```

Use independent host/guarded-memory expectations, not one backend as another's
oracle. Include 1/2/3/4-byte members, 31/32/33 and 255/256/257-byte extents, invalid
generic substitutions, read-after-overlapping-write, inactive-byte preservation,
mutation after LET capture, module-only uses and exact signature mismatches.
Keep existing records, variants, generics, arithmetic, CASE, LET and scalar ABI
tests green. Explain any fixture contract changes and update only relevant counts.

## 6. Progress

- Plan saved in `4e3ac9f` against baseline `2994212`; gated slice 1 is complete.
- Slices 1–5 are complete; slices 6–7 remain pending. No public UNION support
  is claimed yet.

### Slice 1 — Syntax, identity and canonical layout (complete)

- Added contextual UNION definitions, canonical nominal identities and explicit
  aggregate layout kind. Shared field placement now supports sequential records
  and overlapping union members; variant payload layout is unchanged.
- Reused target widths/alignment, layout queries, pointer recursion barriers and
  module identity. Internal declaration/layout canaries lower through ordinary
  SemIR/NIR forms on all four target layouts; raw AST codegen rejects union syntax.
- Added transitive member eligibility and initializer guards now, before any
  consumer could traverse overlapping representations. This brings forward the
  safety rejection from slice 4, not its absolute/volatile execution acceptance.
  Generic unions and all public union capability gates remain closed.
- Ten focused tests cover parsing/contextual names, gates, layout/queries,
  grouped members, recursion/identity, invalid declarations, transitive member
  and initializer restrictions, module aliases and AST-only rejection.
- Acceptance: all 2,938 compiler tests pass, including the ten new layout/gate
  tests and existing NIR snapshots. The 44-source NIR sweep, 167-source MIR6502
  sweep and all-target cargo check pass. All 14 focused pinned VM regressions
  for variants, nested patterns and CASE guards pass after the shared inline-type
  walk refactor. The full VM suite/public enablement remain later-slice gates.
  No existing fixture, count or public capability changed in this slice.

### Slice 2 — Typed access and alias correctness (complete)

- Ordinary typed field places already preserve union offsets, widths, signed
  interpretation and selected-member writes. Added semantic rejection of legacy
  aggregate-to-scalar arithmetic/casts, scalar assignment and truth conversion
  for inline-union-containing values. Explicit typed addresses remain available.
- Audited NIR storage facts and forwarding: aggregate homes remain addressable
  and untrackable; field/indirect stores and CopyBytes invalidate cached scalar
  memory. Structured effects intersect byte ranges, not FieldIds. No union pass
  or new executable IR form was necessary.
- Added a reusable classic SemIR entry with explicit runtime selection and an
  internal-capability VM harness. It reuses the existing cartridge/standalone
  linkers without exposing a public source capability or CLI switch.
- Two compiler tests cover typed places and native lowering on all four targets,
  plus rejected scalar/nominal operations. Three guarded-memory VM tests pass in
  classic and raw/optimized MIR for both Atari runtimes: alternating signed/enum/
  byte/word views, trailing-byte preservation, pointers, calls, machine writes,
  branch joins, high-offset arrays and exactly-once index/RHS evaluation.
- Acceptance: all 2,940 compiler tests and NIR snapshots pass; NIR 44/44 and
  MIR6502 167/167 sweeps pass. The full pinned VM result is recorded under the
  combined slice-5 gate below; focused executable tests pass. Public enablement
  remains reserved for slices 6–7. Existing fixture contracts are unchanged.

### Slice 3 — Aggregate composition and snapshots (complete)

- Reused aggregate copies, LET captures and direct/typed-indirect argument/result
  lowering without backend special cases. Three compiler tests cover immutable
  subobject/address restrictions, exact nominal callable signatures, full capture
  extents (including native tail padding), native private frames and unchanged
  Atari routine-static parameter lifetime.
- Three six-lane VM tests cover nested records/unions/arrays, copied pointer
  values with shared pointees, mutation after capture, private parameters, live
  results, ignored results, forwarding, and destination/callee/argument ordering.
  A host memmove oracle covers self-copy and both overlap directions for extents
  1/2/3/4, 31/32/33 and 255/256/257, with guarded page-crossing storage.
- All focused tests pass. These exercise the slice-2 implementation already
  passing the full compiler suite/sweeps; no production compiler change or fixture
  update is needed. The combined full pinned VM result is recorded below.

### Slice 4 — Initialization and low-level storage (complete)

- The transitive initializer guard introduced in slice 1 already rejects list/
  string initialization through nested records, arrays and generic wrappers, at
  module and local scope. RAM absolute addresses, ordinary storage aliases and
  static subobject-address relocations remain distinct from initialization.
- Extended the shared VM harness with watched bus events. Three six-lane tests
  prove volatile alias propagation, selected byte/word widths, retained repeated
  reads, skipped conditional reads, read-before-write compound order (no dummy
  writes), immutable full-width snapshots and complete volatile copies. Multi-byte
  access is not atomic; classic and MIR may use different byte access orders.
- Three compiler tests preserve qualifier rejections, target layout/alignment
  facts, and 1/2/4-byte volatile operations plus CopyBytes flags before/after NIR
  optimization. Absolute bindings retain existing semantics; this slice does not
  introduce a new raw-address alignment policy.
- Focused compiler and VM tests pass, as do the access, indirect-call and CASE-
  guard harness regressions (13 VM tests). No production compiler or fixture
  change was necessary. The combined full pinned VM result is recorded below.

### Slice 5 — Generics, modules and variant composition (complete)

- Enabled internal generic union definitions through the same union capability
  gate. Concrete instances use the existing nominal cache and overlapping layout
  resolver. Member eligibility is checked after substitution; REAL/callable/
  variant restrictions follow inline records, arrays and generic instances while
  data pointers remain traversal barriers. Public profiles remain closed.
- Five compiler tests cover cached identity, target layout, exact signatures,
  mutually recursive pointers, rejected inline/expanding recursion, the existing
  depth/instance budgets, imported aliases and immutable variant binders. Three
  six-lane VM tests cover module-only generic definitions and callbacks, guarded
  union-payload snapshots, and retained outer-variant Error(100) validation.
- Module callback tests exposed a shared SemIR bug: qualified routine values
  missed the existing bare-name address conversion and became aggregate scalar
  loads. Generalized that path to use the existing canonical direct-symbol
  resolver, with a non-union record regression on all four targets. No generic-
  union-specific lowering, new IR operation or backend workaround was added.
- All 2,952 compiler tests pass, including NIR snapshots, the new union tests
  and the qualified ordinary-record callback regression. NIR 44/44, MIR6502
  167/167 and all-target cargo check pass. All 187 pinned VM tests pass, including
  the twelve new six-lane union tests. No existing fixtures were changed.
- This completes slices 2–5, not public enablement. Slices 6–7 retain their
  explicit backend-limit/representation acceptance, documentation, cost baselines
  and release-gate work. Native canaries do not claim native runtime execution.
