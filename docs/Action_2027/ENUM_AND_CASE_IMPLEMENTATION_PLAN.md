# ENUM and CASE Implementation Plan

Status: implementation in progress. This plan incorporates the
language review in [ENUM and CASE design](ENUM_AND_CASE_DESIGN.md).

Inspected baseline: `5d4d164` (`Route arithmetic faults through Atari Error`).
Oscar64 porting remains
suspended; this work neither resumes it nor depends on deferred volatile work.

## 1. Delivery Contract

The first complete delivery includes:

- `TYPE Name=ENUM [...]`, with whitespace-separated members and optional commas.
  TYPE remains the named-type declaration entry point; existing record syntax
  and existing RECORD declarations retain their behavior.
- BYTE-only nominal enums: 0 through 255, no representation annotation, no
  signed/wider form, no implicit widening, and no numeric member aliases.
- First implicit member zero; each subsequent implicit member is the preceding
  member plus one. Check overflow and duplicates; do not skip occupied values.
- Qualified members, exact enum assignment/argument/return typing, typed CONST,
  explicit scalar conversions, and same-enum comparisons.
- Enum variables, arrays, record fields/embedded arrays, pointers, direct
  parameters/results, and the existing FUNC POINTER result-type surface.
- `CASE selector OF`, `WHEN labels THEN`, optional final ELSE, and ESAC only.
  Integer singleton/multi-label/range arms and same-enum constant labels work.
- One selector evaluation, no fallthrough, no implicit arm scopes, and unchanged
  nearest-loop EXIT and routine RETURN behavior.
- Modern classic and modern MIR6502, each with standalone and cartridge-linked
  runtimes. Compatibility rejects the new constructs explicitly.

```action
TYPE ResultCode=ENUM [OK=0 FAILED=1 BUSY=10 RETRY]

ResultCode FUNC TryStart(BYTE busy)
  IF busy THEN
    RETURN(ResultCode.BUSY)
  FI
RETURN(ResultCode.OK)

PROC Main()
  BYTE result

  CASE TryStart(0) OF
  WHEN ResultCode.OK THEN
    result=1
  WHEN ResultCode.BUSY, ResultCode.RETRY THEN
    result=2
  ELSE
    result=3
  ESAC
RETURN
```

This is proposed syntax, not a currently runnable sample. The final runtime
fixture must use observable output rather than a dead local result.

All 256 enum representation values remain defined, including unnamed values
from memory or explicit casts. No membership trap or new runtime helper is
introduced. Named-member coverage alone cannot remove the no-match path.

Guards, guarded wildcards, unions/variants, payload binding, enum ranges, and
jump tables are follow-ons. The separate guard phase below records the agreed
extension direction without making it a prerequisite for this delivery.

## 2. Implementation Boundaries

### TYPE and enum facts

`ast::TypeDecl` currently stores record fields directly. Replace that assumption
with a type-definition discriminator, initially Record and Enum. Do not add a
new parallel top-level enum declaration mechanism or refactor every existing
type into a speculative algebraic-type framework.

Give each enum a nominal identity backed by its resolved type `SymbolId` or a
small typed wrapper around it. Keep member names/values/spans in semantic facts.
Imports refer to the same identity; same-spelled local types remain distinct.
Do not encode enums as record-only `ValueTypeBase::Named(String)` or add them to
fundamental `ScalarType` in a way that enables implicit integer promotions.

The two typed-expression layers in `semantic/subject.rs` and `semantic/ir.rs`,
their literals, materialization, and constant facts must agree on enum identity.
Reuse the integer constant evaluator for payload bits, but retain the nominal
type until all language checks are complete. In particular, inferred
`CONST Value=ResultCode.OK` retains its enum type, and
`CONST ResultCode Value=0` is not an implicit enum conversion.

At the NIR boundary, enum values/storage lower to U8. Classic projection uses
BYTE forms. Both consume canonical semantic facts; neither re-resolves enum
names or re-evaluates declarations. Preserve debug/source type names as metadata.

### CASE representation and dispatch

Add one source CASE node with ordered arms. Distinguish singleton/range labels
from an explicit ELSE catch-all. An absent ELSE is not an empty explicit ELSE.
Retain label/arm spans so duplicate diagnostics can identify both declarations.

SemIR owns the selector type, constant normalization, interval/domain checks,
initial overlap policy, source ordering, and control-flow facts. Use typed
intervals, not lists expanded across a range. Represent ELSE as a final
unguarded catch-all, without accepting source `WHEN _ THEN` as an alias.

Add focused validation of the new SemIR facts through the existing semantic
boundary and tests; do not first build a general-purpose SemIR verifier.

NIR receives a captured value and ordinary compare/Branch/Goto blocks. It needs
no Switch opcode, enum-specific arithmetic opcode, or runtime dispatch helper.
Preserve signedness for INT CASE even though enums themselves are byte-only.
Use direct lower/upper comparisons, not overflow-sensitive subtraction tricks.

Classic projection collects compiler-owned capture storage and expands CASE
into an assignment plus existing IF-style branches. Use collision-free storage
identity and assign on each CASE entry. Distinct nested captures must not alias.
The projection's statement-list builder already flattens lexical blocks; extend
it to collect the additional statements/declarations rather than generating and
re-parsing source. Modern classic already takes the SemIR route for both
runtimes; verify that route instead of redesigning compiler selection.

Preserve arms as an ordered sequence, not an unordered map from value to body.
Disjoint labels are an initial-language validation policy, not a permanent
requirement of the dispatch architecture. This is the extension point for guards.

### Routine results and ABI

`RoutineKind::Func` and several parser predicates currently assume `FundType`.
`ConstDeclaredType` likewise lacks named enum types. Generalize the relevant
source result/constant annotations to accept qualified type names.

Use a nonrecursive result syntax such as fundamental-or-named type, not an
unboxed full `TypeRef` inside its own callable branch. Semantic callable result
facts must carry the resolved enum type. Audit consumers that recover result
meaning from the old fundamental-only routine kind and move those decisions to
the canonical resolved result facts; adapters may project BYTE for codegen.

Direct enum parameters/results and `ResultCode FUNC POINTER reader` must work.
Do not add parameterized function-pointer declaration syntax, aggregate returns,
or a new calling convention. Source callable compatibility must distinguish
different enums even when their machine signatures use identical bytes.

Extend signature identity handling deliberately: use resolved nominal identity,
not just width or an unqualified spelling. Preserve existing non-enum signature
behavior and deterministic snapshots; do not derive persistent identities solely
from allocation-order-dependent numeric IDs.

## 3. Slices and Exit Gates

Implement in numbered order. Each major slice should be a focused commit once
implementation is authorized, with its tests and documentation changes together.
Do not bundle optimization work or unrelated cleanups into these commits.

### Slice 0 — Baseline and capability gates

Status: complete. CASE and enum capabilities are independent internal semantic
options, initially false in every public profile and runtime provider. Baseline:
2,766 compiler tests, 115 VM tests, snapshots, and all 33 sweep fixtures passed.
After the gate change, all 2,767 compiler tests, the focused gate test, snapshots,
and all 33 sweep fixtures passed. Existing snapshots/output remain unchanged.

- Run and record the existing compiler/NIR/VM baselines before changing code.
  Do not treat a pre-existing failure as an accepted new expected result.
- Add independent internal semantic capabilities for CASE and enums, disabled
  in public profiles while incomplete. Follow the existing semantic-options
  pattern; do not add experimental CLI switches.
- Tests can enable a capability and use existing lower-level semantic/codegen
  entry points until the public gate opens. Incomplete operations must produce
  focused diagnostics, not backend panics or implicit BYTE fallbacks.
- Establish focused test modules and fixture naming before broad changes.

Exit gate: unchanged public behavior and baseline output; internal gates are
explicit and cannot accidentally be enabled by runtime or backend selection.

### Slice 1 — Integer CASE vertical slice

Status: complete. Four focused semantic/NIR tests and four internal execution/
linking tests pass. The execution tests cover every BYTE input, INT/CARD word
labels, single selector evaluation, nested CASE/IF/loops, lexical blocks, EXIT,
RETURN, and missing ELSE across both backends and runtimes. These use the
existing test-only 6502 executor, not the separate VM harness; public VM tests
follow in slice 2. The full compiler suite, snapshots, and all 33 NIR sweep
fixtures pass unchanged. Public CASE remains gated in this slice.

Primary files: `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`,
`src/semantic/ir.rs`, `src/nir/lowerer.rs`, `src/codegen/semir.rs`.

- Parse contextual CASE/OF/WHEN/ESAC, singleton and comma-separated constant
  labels, THEN, and optional ELSE. Preserve historical lexer token IDs.
- Follow the design's initial physical-line header rules. Recognize the complete
  `CASE ... OF` shape before named-variable declaration detection; otherwise
  `CASE state OF` can be mistaken for a type and variable declaration.
- Update expression stops, declaration/body boundaries, and nested terminator
  handling together. Preserve `Case()`, `When()`, `Esac()`, assignments, and
  fields using those spellings outside structural contexts.
- Validate BYTE/CHAR/CARD/INT selectors and compile-time label conversion without
  implicit truncation. Diagnose unsupported selectors, duplicate labels, malformed
  arms, ENDCASE, and deferred guards/wildcards.
- Implement both lowering paths with one selector capture, ordinary comparisons,
  and a shared continuation. Include nested CASE/IF/loops and RETURN/EXIT now.
- Update every affected walker: linking/reachability, standalone restrictions,
  side effects, lexical declaration collection, flow analysis, materialization,
  source mapping, and printing. A routine referenced only inside a WHEN or ELSE
  must remain linked and still undergo runtime-availability checks.

Exit gate: internal end-to-end tests execute correct singleton/multi-label CASE
in all four modern backend/runtime combinations, including a counted selector
call. Public CASE remains gated until slice 2.

### Slice 2 — Integer ranges, flow/effect hardening, CASE enablement

Status: complete. Integer CASE is publicly enabled in modern only. Eight focused
integration tests, five internal execution/linking tests, both new VM tests, the
full 2,779-test compiler suite, snapshots, and all 35 NIR sweep fixtures pass.
The VM tests execute all 256 byte inputs with signed/unsigned word boundaries,
pointer selectors, canaries, and selector-call counts in all four combinations.
CHAR ranges pass the internal executor matrix. Volatile NIR has exactly one
selector read before and after optimization. Selected faults reach Error(100)
without continuation; unselected calls/stores/faults have no effects. Fault tests
install an explicit returning Error observer; other cart execution uses real ROMs.

The capture-name canary found and fixed a collision with ordinary routine locals;
capture reservation now includes globals, parameters, locals, and lexical names.
Two new lowered/optimized NIR fixture pairs document ordinary typed dispatch;
existing snapshots and the feature inventory remain unchanged. The broad-corpus
inventory grows from 321 to 325 successful fixtures, with its same five declared
non-entrypoints. Integer CASE also passes independent 65816/68k lowering canaries
(not native execution). Guards and enums remain separately unavailable.

- Normalize inclusive `low TO high` intervals without expanding them. Reject
  descending ranges and all initial unguarded overlaps, including within one arm
  and duplicates created by constant folding or explicit casts.
- Compare normalized numeric values with the selector's signedness; do not sort
  INT endpoints as unsigned storage bits. Cover 0/255, 0/65535, and -32768/32767.
- Retain the no-match continuation when ELSE is absent. Combine explicit arm
  flow using IF-style facts; all returning arms plus a returning ELSE may prove
  a FUNC returns. CASE does not change loop depth or consume EXIT.
- Prove that unselected arm calls/stores/faults do not execute and that fixed,
  escaped, pointer, and volatile selectors keep the existing effect contract.
- Add lowered/optimized NIR fixtures and inspect representative classic/MIR
  output. Any optimizer fixes must address a general compare/CFG/effect defect,
  not add a CASE-specific sample optimization.
- Enable integer CASE in the modern profile after the complete gate passes.
  Compatibility still rejects it; enums and guards remain separately gated.

Exit gate: full integer CASE syntax and semantics work through public compilation
in all four combinations. Unrelated code and existing IF/loop behavior remain
unchanged. Document only this completed surface at this stage.

### Slice 3 — TYPE definition representation migration

Status: complete. TYPE now discriminates Record and Enum definitions. ENUM
member parsing delegates complete expressions to the existing precedence parser,
independently of line breaks. Four focused parser/gating tests, all 2,783 compiler
tests, snapshots, the 35-fixture NIR sweep, and the record-array/lexical-block VM
tests pass. Existing record layouts and snapshots are unchanged. Enum definitions
remain rejected by semantic analysis in public profiles until the remaining
type, routine, and storage slices are complete.

- Change `TypeDecl` from a record-only field list to a named type definition.
  Migrate all existing record consumers to the Record branch without changing
  layout, name resolution, initializers, emitted bytes, or RECORD syntax.
- Add the enum source definition/member nodes and parse
  `TYPE Name=ENUM [...]`, including explicit expression values and optional commas.
  Newlines do not delimit enum members semantically. Parse a complete initializer
  expression before recognizing the next member; do not split blindly at every
  identifier or equality token.
- Reject empty lists, unsupported representation annotations, and malformed
  definitions cleanly. Recognize ENUM only in its type-description context.
- Keep enum semantic capability disabled in public profiles.

Exit gate: record regression tests and non-enum NIR/object baselines are unchanged;
enum parsing is covered but cannot reach unsupported public code generation.

### Slice 4 — Nominal BYTE enum core, end to end

Status: complete behind the enum capability. Nine focused integration tests,
three internal execution/materialization tests, all 2,791 compiler tests,
snapshots, and the 35-fixture NIR sweep pass. All 256 byte values execute across
both backends/runtimes; signed/wide casts, zero extension, unnamed values,
comparisons, CASE, and no-match continuation have independent expectations.
The RETRY=11 collision, 256-member overflow, invalid nominal mixing, typed/
inferred CONST, and shadow-safe materialization are covered. Enum storage is U8
in all four target layouts and passes independent 65816/68k lowering canaries.
Enums remain unavailable publicly until routine and complete storage integration.

- Build enum/member semantic facts in declaration order. Implement checked
  previous-member-plus-one numbering and duplicate name/value diagnostics.
- Bind qualified members, typed/inferred enum CONST, and explicit enum casts.
  Preserve nominal identity through constant folding and both typed-expression
  layers; member constants have no storage and cannot be addressed or assigned.
- Enforce same-enum assignment/comparison and reject implicit integer mixing,
  enum arithmetic, truthiness, loop-counter use, and numeric indexing. Explicit
  integer conversion remains available where appropriate.
- Support scalar objects, loads/stores, BYTE lowering, and enum CASE using the
  existing dispatch path. Reject bare numeric/cross-enum labels and enum ranges.
- Cover unsigned byte ordering, explicit conversion from signed/wider integers,
  and unnamed values produced both at compile time and from raw memory.

Required numbering regression:

```action
TYPE ResultCode=ENUM [OK=0 FAILED=11 BUSY=10 RETRY]
```

This must diagnose RETRY=11 duplicating FAILED=11, not choose 12. Also test a
valid explicit RETRY=12 and all 256 distinct byte values followed by overflow.

Exit gate: internal scalar enum tests execute on both backends/runtimes; NIR
uses U8 but semantic misuse still fails before lowering. Public enums remain
gated pending complete routine/storage integration.

### Slice 5 — Enum parameters, function results, and callable results

Status: complete. All 2,796 compiler tests, unchanged NIR snapshots, and all 35
NIR sweep fixtures pass. Four
focused parser/signature tests and four internal execution/materialization tests
pass. Enum results, nested direct calls, indirect calls, and counted CASE
selectors execute for all 256 byte values in both backends and runtimes.
This uncovered and fixed incorrect call-return flag assumptions in classic and
stale MIR N/Z exit summaries during rewriting. General BYTE ABI and signature
formats remain unchanged; enum signatures retain their nominal result identity.

- Generalize routine/result annotations and all associated lookahead predicates:
  routine start/end, named declarations, FUNC POINTER, PUBLIC/EXTERNAL forms,
  module-qualified result names, and CONST annotations. Do not patch only
  `parse_routine` while leaving body-boundary detection fundamental-only.
- Resolve direct parameters and results to exact enum identities. Cover enum
  return values in assignments, expressions, nested calls, and CASE selectors.
- Extend existing function-pointer result syntax and compatibility checking;
  different enum result types and BYTE results are not interchangeable by width.
- Update NIR signatures, BYTE result projection, ABI selection, and any routine
  serialization/printer consumers without introducing new calling conventions.
- Check aliases/calls around result capture, including nested enum-returning
  calls and an enum selector function that increments an observable counter.
- Reject bare RETURN, returning a raw BYTE, returning a different enum, and
  missing-return paths. Accept an explicit enum conversion under the open-value
  contract, including unnamed byte results.

Exit gate: enum FUNC returns and existing callable-pointer results work through
both backends/runtimes using the BYTE ABI. Original built-in function and
callable-pointer behavior remains covered. This slice is mandatory for release.

### Slice 6 — Storage, scopes, modules, and metadata integration

Status: complete. All 2,804 compiler tests and the 35-fixture sweep pass against
the staged slice in an isolated checkout, including unchanged NIR snapshots.
Six storage/module tests and six internal execution/materialization tests pass;
the latter cover all 256 bytes, offsets above 255, zero-fill, and link selection.
65816 and 68k enum storage lowering canaries also pass. Enum
scalar/array/record initializer leaves are typed before encoding, with explicit
numeric bridges for sizes and addresses. Both import forms, private names,
shadowed types, and named-module layout dependencies have focused tests. Enum
metadata survives selective linking; executable values remain ordinary bytes.

- Reuse scalar/record/array layout and initializer walkers for enum globals,
  locals, parameters, arrays, record fields, nested records, and embedded arrays.
  Validate enum initializer types before encoding bytes. Preserve zero-fill and
  existing declaration backing/address semantics; do not add constructors.
- Support enum pointers and array decay through existing legal pointer contexts.
  Exact enum element identity must survive pointer/array compatibility checks;
  matching element widths alone are insufficient.
- Cover routine/block-local enum declarations and shadowing, qualified module
  types/members, PUBLIC exports and both import forms. Members stay scoped under
  the enum; imports retain the same type identity.
- Audit selective linking/type-fact retention, SIZEOF/ALIGNOF and existing layout
  queries, map/source names, static data, and constant-materialization consumers.
  Include a storage-boundary test through a neighboring-byte guard and a record
  field offset above 255; enum fields still have one-byte extent.
- Keep assembly/data interfaces explicit about numeric conversion. Do not add
  automatic enum name strings or silently broaden assembly expression syntax;
  existing supported scalar constant bridges remain the compatibility boundary.

Exit gate: complete enum storage/visibility behavior agrees across both backends
and runtimes; layout canaries pass for Atari6502, both 65816 targets, and 68k.
No new aggregate-return or parameterized callable syntax is required.

### Slice 7 — Public enum enablement and complete-feature validation

- Enable enums in the modern profile only after slices 4–6 pass together.
- Add a small runnable enum-returning state-machine example using CASE and
  observable outputs, plus a sample-build entry under both linking modes.
- Run the full acceptance matrix below, including public API/CLI behavior,
  default and explicit classic codegen-source routes, and compatibility errors.
- Check independent NIR/MIR lowering canaries for 65816 and 68k. The current
  public compiler does not emit executables for those targets; do not claim
  native runtime validation from lowering alone.
- Update syntax/name-resolution/profile references and relevant semantic/NIR
  boundary contracts. Mark this plan complete only when enum returns, storage,
  CASE, and the tests actually work. Leave deferred features explicitly deferred.

Exit gate: documented v1 ENUM and CASE are usable through public modern compiler
entry points, with no unsupported path hidden by type erasure or backend choice.

## 4. Validation Matrix and Commands

Use independent host-side expectations, not generated assembly or the original
cartridge compiler, as the oracle for these new language constructs.

| Layer | Required coverage |
| --- | --- |
| Parser | TYPE/ENUM and ESAC grammar, contextual identifiers, complete expression boundaries, nesting, malformed headers, qualified FUNC/CONST/callable results |
| Semantic | Enum identity/numbering/casts, constants, label conversion/overlaps, return flow, unsupported profiles/features, scopes and module visibility |
| SemIR/linking | Typed enum facts, ordered CASE arms/catch-all, all nested calls/effects/type facts retained, no unresolved labels |
| NIR | Lowered and optimized snapshots, single capture, signed interval comparisons, valid CFG/uses, no executable enum names or CASE syntax |
| Classic/MIR6502 | Identical observable results in modern classic and modern MIR6502, with ActionCart and Standalone runtimes |
| Cross-target | Enum U8 layout/stride and integer CASE lowering on Atari6502, Wdc65816Small, Wdc65816Native, and Motorola68000 |
| Regression | Existing records, BYTE FUNC, CONST, comparisons, loops, lexical blocks, pointer signatures, ABI baselines, and error handling |

New test homes (proposed, not present yet):

- `tests/case_statements.rs`, `tests/enum_types.rs`, and
  `tests/enum_routines.rs` for focused semantic/lowering/public API coverage.
- `fixtures/nir/case_dispatch.act`, `case_ranges.act`, `enum_values.act`, and
  `enum_calls.act`, with lowered/optimized snapshots registered in
  `tests/nir_fixture_support/mod.rs` and any relevant sweep inventory.
- `fixtures/runtime/case_statements.act` and `enum_types.act`, exercised by
  new `case_statements` and `enum_types` targets in `tools/vm-runtime-tests/tests`.

Reuse existing in-memory/temp-source helpers, snapshot support, and VM loading
patterns rather than building a new harness. Feed runtime test inputs externally
so constant folding cannot make all dispatch tests vacuous. Exercise all 256
byte inputs for representative byte/enum cases and independently chosen signed
and unsigned word boundaries. Test both branch outcomes and explicit ELSE.

Critical negative/effect tests include duplicate member 11, member overflow,
numeric labels narrowing to a byte without a cast, nominal mismatches after
folding, duplicate/overlapping intervals, absent-ELSE return paths, nested EXIT,
unselected faults/stores, and exactly one selector call/read. Non-returning fault
tests must distinguish the expected error path from a watchdog hang.

For each semantic/IR/codegen slice, run relevant focused tests and the required
root checks from the repository root:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Run VM commands with `tools/vm-runtime-tests` as the working directory so Cargo
uses that harness's configuration and pinned VM dependency. Once the new test
targets exist:

```sh
cargo test --locked --test case_statements
cargo test --locked --test enum_types
cargo test --locked --no-fail-fast
```

The four positive execution combinations are `CompileMode::Optimized` and
`CompileMode::Mir6502`, each with `Runtime::ActionCart` and `Runtime::Standalone`.
Compatibility rejection is a separate compile-time check, not a successful VM
case. Record actual results per slice, identify intentional snapshot changes,
and distinguish cartridge-entry test doubles from real ROM-backed execution.

## 5. Separate Follow-on: Guards and Guarded Wildcards

After the initial delivery, add `WHEN labels IF condition THEN` and
`WHEN _ IF condition THEN`. Keep ELSE as the unique final unconditional fallback;
do not introduce bare `WHEN _ THEN` or ENDCASE as synonyms.

1. Extend arm syntax/facts with ordinary IF-condition guards. Pattern matching
   precedes guard evaluation; a false guard continues searching. Do not evaluate
   a guard for a nonmatching arm or re-evaluate the captured selector.
2. Replace the initial blanket overlap rejection with ordered coverage analysis.
   Permit repeated labels after guarded arms and diagnose arms fully shadowed by
   earlier unconditional coverage. Partial interval overlap is not by itself
   proof that the whole later arm is unreachable. Enum member duplicates remain
   illegal independently of this change.
3. Lower guard evaluation into ordinary conditional CFG on both backend paths.
   Guard calls/stores/faults preserve source order and normal IF semantics. A
   guard may change the source selector variable, but later arms still compare
   the original captured value. Capture storage must remain valid across guards.
4. Cover guarded fallback, failed/matched/unevaluated guards, side effects,
   intervals, and effectful selector mutation in all four execution combinations.

No selectorless CASE, payload patterns, new implicit scalar-arm scope, or mutable
payload aliases are required for this follow-on. Those belong to a separate
variant/algebraic-type design and implementation effort.

## 6. Completion and Non-Goals

Finish the base delivery only when slices 0–7 and their gates pass. Do not claim
completion based only on parsing enum declarations or compiling a scalar CASE.
Do not run the original compiler as an oracle for syntax it does not support.

This plan does not authorize an implementation or commit in the design-only turn.
When implementation begins, update slice status and actual validation results
after each major change. Keep optimizations, wider enums, guards, variants,
runtime diagnostics, and resumed Oscar64 porting out of the base implementation.
