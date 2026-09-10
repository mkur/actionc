# IF and CASE expressions

Status: slices 0 through 2 implemented. Modern integer/enum IF and CASE values
execute end to end. Variant CASE values remain gated; slices 3 and 4 are pending.
Baseline: `3afeb08`, inspected on 2026-09-10.

## Objective

Let modern Action! choose a value using IF or CASE wherever a runtime integer
or enum expression is accepted. Evaluate only the selected result expression,
preserve existing evaluation order, and reuse verified NIR control flow.

```action
LET smaller=IF a<b THEN a ELSE b FI

USE ALL FROM MaybeByte
LET value=CASE item OF
WHEN NONE THEN
  0
WHEN SOME(n) THEN
  n
ESAC
PrintBE(value)
```

The second example assumes `SOME` has a BYTE payload. The opening follows the
existing [local USE rules](../tutorials/VARIANTS.md). Qualified and generic
constructor patterns remain
available without an opening.

Deliver integer and enum results first. REAL, data/callable pointer results,
aggregate results, and statement blocks that yield a final expression are
separate extensions. A variant **selector** is in scope; a variant **result** is
not. Existing statement IF/CASE behavior remains compatible.

## Source contract

### Syntax and placement

- IF expression: `IF condition THEN expression [ELSEIF condition THEN expression]* ELSE expression FI`.
  ELSE is required, including when the first condition is constant. Conditions
  use the existing Action! condition rules. An ELSEIF condition runs only after
  all earlier conditions were false.
- CASE expression: `CASE selector OF`, followed by existing WHEN headers,
  optional ELSE, and ESAC. Each arm contains exactly one result expression.
  Preserve the current multiline CASE header style: WHEN headers end with THEN;
  ELSE occupies its own line. The expression opening can follow `=`, `(`, or
  another expression introducer on the same line, as in the example above.
- FI and ESAC close the expression itself. They may be followed by a surrounding
  `)`, argument comma, index delimiter, or binary operator. Nesting must pair
  each closer with its own opener, independently of physical lines.
- Selection expressions are delimited primary expressions. Parentheses may
  clarify composition, such as `1+(IF ready THEN a ELSE b FI)`. Preserve existing
  arithmetic precedence within conditions and result expressions.
- Support nesting in LET initializers, assignment RHSs, RETURN values, call
  arguments, arithmetic/comparisons, indexes, and runtime condition/bound
  expressions. Follow each enclosing consumer's existing evaluation rules.
- Arms contain expressions, not statements. Reject assignments, LET, local USE,
  BEGIN/END blocks, RETURN, EXIT, and value-less PROC calls in result positions.
  Such syntax requires a later block-expression design; there is no implicit
  "last statement yields a value" rule.
- Keep CASE contextual: ordinary identifiers such as `Case()` and `case=1`
  remain valid. Statement-start IF/CASE retains its existing statement grammar.
- These are modern runtime expressions. Reject their evaluated use in CONST,
  static initializers, array bounds, CASE labels, and other constant-only
  contexts, even with literal conditions. Existing unevaluated layout-query
  semantics must not accidentally execute a selection or its arms.
- Diagnose malformed/missing branches and closers at their source spans. Apply
  an explicit nesting limit, initially 64, across mixed IF/CASE expression
  nesting, with bounded recovery at enclosing statement/routine boundaries.

### Result types

Infer every arm independently, then require the same canonical integer type or
the same nominal enum identity in all arms. Use existing literal typing; no new
literal coercion, C ternary promotion, common-width search, or expected-type
inference is introduced. Check all source arms before optimization, including
arms excluded by constant conditions. Ordinary semantic errors remain errors.

Analyze result arms in value context even when the enclosing selection is used
as a condition. Only IF tests and CASE guards inherit condition semantics; this
keeps an arm's type and bitwise evaluation independent of its eventual consumer.

Different types require conversions inside the arms:

```action
; health is BYTE; damage is CARD
LET applied=IF damage>CARD(health) THEN CARD(health) ELSE damage FI
LET BYTE narrowed=applied
```

An enclosing LET annotation, assignment, argument, return conversion or explicit
cast applies to the completed selection value using existing rules. It does
not change intermediate arithmetic or reconcile mismatched arm types. Thus
`LET CARD x=IF flag THEN byteValue ELSE wordValue FI` is still a type error.

The result is a value, never an assignable place. Reject assignment to a
selection and taking its storage address, even if both arms name variables.
Keep enum identities distinct before NIR erases their representation to U8.

### Coverage and matching

| Selector | Initial expression rule |
| --- | --- |
| Integer | Require an explicit ELSE. Reuse current constant/range, overlap and guard rules. |
| Enum | Require an explicit ELSE, even when all declared members are listed. Reuse exact enum-label identity checks and the existing prohibition on enum ranges. |
| Variant | Reuse current pattern usefulness/exhaustiveness checking. ELSE is optional when unguarded patterns cover every valid value. |

The integer/enum ELSE requirement avoids introducing a separate full-domain
coverage proof or an enum runtime-validity policy. Guarded arms do not establish
unconditional coverage. Variant alternatives, nested patterns, `_`, immutable
binders and ordered guards retain their existing meanings. Opened nullary names
in payload patterns remain constructors, as established by local USE.

Evaluate the selector once. Preserve the current snapshot boundary when calls
or guards can mutate its source. Guards run in source order only after their
pattern tests and required binder initialization succeed; the selected result
runs only after its guard succeeds. A false guard continues matching.

Invalid variant tags never produce a value or enter a user ELSE. Preserve the
existing terminal InvalidVariantTag fault, including when Error returns. Flat
dispatch may retain its current validation fusion; inline nested variants keep
early deep validation before tests, binders and guards. A known outer tag does
not prove nested validity. Statement matching must use the same rules. See the
existing [validation contract](../VARIANT_CASE_VALIDATION.md).

## Existing foundations and implementation boundaries

| Component | Reuse and required work |
| --- | --- |
| `src/ast.rs`, `src/parser.rs`, `src/parser/case.rs` | Add distinct expression forms and result-arm nodes. Current token collectors track parentheses/brackets, while CASE statements depend on physical-line headers. Teach collectors and expression parsing balanced IF/FI and CASE/ESAC boundaries; do not recover expressions by parsing `Expr.text`. |
| `src/semantic/subject.rs`, `src/semantic.rs` | Add typed value subjects, result-type checking, modern capability gates and value/place legality. All expression visitors must traverse conditions, selectors, guards and results. |
| `src/semantic/case.rs`, `variants.rs`, `patterns.rs` | Extract reusable checked dispatch/pattern facts from statement-body analysis. Reuse label checks, canonical constructor/field IDs, binder scopes, guards and coverage for expression bodies. |
| `src/semantic/ir.rs`, `src/semantic/ir/aggregate.rs` | Represent resolved value selection and preserve selector capture, tests, binder preparation and fault ordering. Update all dependency, declaration, effect, printing and flow visitors. |
| `src/nir/ir.rs`, `src/nir/lowerer.rs`, `src/nir/verifier.rs` | Reuse `NirBlockParam`, `NirEdge.args`, ordinary branches and existing faults. Add lowering helpers for a typed value join; preserve edge arity/type and dominance/use-def checks. |
| `src/codegen/semir.rs`, classic expression/call lowering | Project resolved selection to expression-local control flow and a compiler-owned result temporary where needed. Reuse the existing compiler-only `Prepared` mechanism, keeping preparation inside the expression's evaluation boundary. |

AST and semantic subjects may retain distinct IF/CASE expression nodes. Shared
SemIR should carry typed value-selection nodes with resolved conditions and
dispatch facts; provisional names are `IfValue` and `CaseValue`. CASE value arms
carry the same ordered tests/binder preparation/guard facts used for statements,
plus a typed yielded expression. Factor that common contract instead of creating
a second pattern compiler. No source pattern or constructor lookup belongs in
NIR, MIR or classic emission.

Do not desugar through synthetic source variable names, synthetic RETURNs, raw
AST statement strings, or an unchecked AST `Prepared` expression. Compiler-owned
temporaries must have explicit internal identity and storage ownership, not
pretend to be Action! declarations. The classic projection may choose storage
for a resolved result; it must not decide typing, coverage or source name lookup.

## NIR value joins and evaluation order

Conceptually, lower a scalar IF expression as:

```text
entry:
  condition = evaluate_condition()
  branch condition -> selected_left, selected_right
selected_left:
  a = evaluate_left_result()
  goto join(a)
selected_right:
  b = evaluate_right_result()
  goto join(b)
join(result: T):
  consume(result)
```

Actual CFG identity uses BlockId and TempId. Each normal incoming edge supplies
one value of the result representation type; compiler-generated terminal fault
paths have no edge to the value join. This is ordinary NIR, with no executable
IF-expression, CASE-expression, select, phi-string or source-scope instruction.

CASE follows the existing ordered dispatch and supplies its selected arm value
to the same kind of join. Keep the result in typed NIR values without requiring
scalar promotion to repair an initially undefined result home. Backend edge
copy/spill decisions remain target strategy.

The surrounding expression must preserve earlier operand values across later
branch effects. For `Consume(ReadFirst(), IF Test() THEN Left() ELSE Right() FI)`,
retain existing argument order and protect the first result across Test and
the selected call. Likewise, preserve the language's current destination/index
evaluation order for assignments and compound assignments. Never hoist both
arm preparations into a common prelude.

Keep condition-context AND/OR behavior and eager bitwise value-context AND/OR/XOR
unchanged. A selection inside a loop condition executes on each test; a selection
in a loop body executes on each reached iteration. An unused selection may lose
its result storage but must retain required condition, selector, guard, volatile
and selected-arm effects.

Optimize only after verification. Reuse constant/copy propagation, branch/CFG
cleanup, dead-temp elimination and known constructor-tag propagation. No new
alias-sensitive optimizer, branch speculation, or eager evaluation is needed.

## Commit slices

Commit each slice only after its relevant checks pass. Syntax may be recognized
behind disabled semantic capabilities before a form's complete lowering lands;
do not expose a form that can escape into verifier-rejected executable NIR.

### 0. AST and parser contract

- Add IF/CASE expression and value-arm AST nodes with source spans and stable
  CASE binder-scope syntax IDs. Retain statement nodes unchanged.
- Share the syntax-ID allocator with surrounding LET, local USE and BEGIN
  parsing; embedded expression parsers must not restart IDs and alias scopes.
- Implement balanced expression collection and nested parsing, including
  ELSEIF, parent delimiters, multiline CASE, contextual names and diagnostics.
- Add traversal coverage and explicit semantic rejection while capabilities
  are disabled. Do not add successful executable fixtures for gated forms.
- Acceptance: parser/diagnostic tests pass; all existing fixtures are unchanged.

### 1. Integer/enum IF expressions end to end

- Add independent arm inference, exact result-type checking and value-only
  legality. Enable IF expressions in modern mode when all consumers are ready.
- Introduce resolved SemIR value selection, NIR typed joins and classic
  projection. Cover nested expressions, ELSEIF and the runtime consumer sites
  listed above; preserve existing target/type limitations.
- Add raw/optimized NIR fixtures and VM tests for both choices, nested calls,
  volatile reads, skipped runtime faults and repeated loop evaluation.
- Acceptance: every normal result path is typed and defined; neither backend
  evaluates the unchosen arm or changes surrounding evaluation order.

### 2. Integer and enum CASE expressions

- Factor checked scalar dispatch from statement-body handling and reuse it for
  value arms, with required ELSE and matching result types.
- Support current integer ranges, multiple labels, enum labels and ordered
  guards. Keep statement CASE's optional ELSE behavior unchanged.
- Cover selectors with calls/volatile reads, guard fallthrough, unmatched values,
  nested IF/CASE results, label diagnostics and enum identity mismatches.
- Acceptance: one selector evaluation; a defined result for every normal input;
  matching classic/raw-NIR/optimized-NIR behavior on both Atari runtimes.

### 3. Variant CASE expressions

- Share checked pattern/coverage facts and lowering with statement CASE. Support
  existing qualified/generic and locally opened constructors, nested patterns,
  arm-local immutable binders, guards and scalar/enum results.
- Preserve snapshot and invalid-tag behavior, including mutation during guards,
  discarded nested payloads, ELSE/wildcard fallbacks and returning Error handlers.
- Add a fixture for the MaybeByte example and a dynamic-selector companion so
  tests exercise both folded and live dispatch/value joins.
- Acceptance: exhaustive matches need no ELSE; incomplete or guarded-only
  coverage is diagnosed; invalid tags cannot yield a result or expose payloads.

### 4. Integration, behavioral port and publication

- Complete a combined consumer/effect matrix: indexed and compound assignments,
  function/constructor arguments, returns, nested conditions, repeated calls,
  loop bounds/tests, unused results, signed values and integer-width boundaries.
- Port Oscar64 `mixedwidthternary.c` from the repository's pinned upstream
  revision as a focused behavioral test. Adapt branch conversions explicitly to
  Action!'s same-type join rule; retain the behavioral oracle and the existing
  provenance/attribution process in `fixtures/runtime/oscar64/README.md`. This
  slice does not resume the separately deferred volatile port batch.
- Compare emitted bytes/cycles for equivalent statement and expression programs,
  including known-SOME and unknown-selector cases. Check that known tags still
  fold dispatch and join values. Record regressions; any fixes must be general
  compiler behavior, not example-specific optimizer cases.
- Publish syntax/type/coverage rules, update semantic and NIR boundary documents,
  and add an example plus sample-catalog coverage. Run final checks and commit.

## Validation and completion criteria

Focused compiler tests should cover parser nesting/recovery, profile gates,
type mismatch diagnostics (including dead arms), constants versus runtime
contexts, value/place legality, module/local-USE identity, coverage, and source
spans. Test expressions in every supported consumer, not only LET initializers.

NIR tests must verify raw and optimized forms, exact join arity/types,
dominance, retained ordered effects and terminal faults. Add malformed-join
regressions where new lowering exposes a verifier gap; never weaken validation.
New snapshots establish the expression lowering contract. Explain any existing
snapshot change as a deliberate lowering change, printer change or bug fix.

VM checks cover modern classic and raw/optimized MIR6502 with cartridge and
standalone runtimes. Use counters and bus observations for ordering, and dynamic
faulting operands in unchosen arms so the tests distinguish runtime laziness
from existing compile-time diagnostics. Wide integer results retain MIR6502's
existing Atari requirement. Verify supported front-end/NIR joins for all four
target layouts; native lowering checks do not imply native runtime fault support.

Run per applicable slice:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
cargo check --all-targets
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked \
  --test if_case_expressions
```

The named VM test is proposed and must be added by slice 1. Include existing
variant, guard, LET, local-USE, known-tag and arithmetic tests when affected;
run the full locked VM suite for final acceptance. Update fixture inventories
and sample catalogs when adding entries.

Completion requires documented public syntax, explicit diagnostics for deferred
forms, successful execution on the supported backend/runtime matrix, verified
typed NIR joins with no new executable source semantics, and preservation of
existing statement behavior and runtime fault ordering.

## Implementation progress

Slice 0 adds structured selection expressions and CASE value arms, balanced
operand collection, ELSEIF and mixed nesting, parent-expression delimiters,
shared lexical syntax IDs, and source diagnostics for malformed or over-nested
forms. AST visitors include the new expression children. CASE statement syntax
and ordinary contextual identifiers retain their existing behavior.

At slice 0, semantic analysis explicitly rejected IF/CASE value expressions in
both profiles until their typed lowering was implemented. That slice changed no
executable SemIR/NIR contract and introduced no successful expression runtime fixture.
Five focused parser/gating tests cover nesting, scope identity, runtime consumer
syntax, malformed input, the 64-level limit and identifier compatibility.
Existing NIR snapshots remain unchanged.

Slice-0 validation: 3,053 compiler tests passed with 22 pre-existing ignored;
the dedicated NIR snapshot command, all 45 NIR sweep fixtures, and
`cargo check --all-targets` passed. Runtime tests begin with slice 1, when the
first expression form becomes executable.

Slice 1 enables integer/enum IF expressions in the modern profile. Semantics
checks every arm independently, rejects mismatched/deferred result types and
static evaluation, and retains rvalue legality. Conditions are checked once per
expression so nested IF tests do not multiply semantic work. Shared SemIR adds
resolved `IfValue`; NIR uses existing typed join parameters/edge arguments;
classic projection uses expression-local preparation and a private result home.
Classic comparison materialization includes prepared operands, including FOR
bounds. The new raw/optimized `if_expressions` snapshots establish this lowering
contract without changing existing fixture expectations.

Focused coverage includes all integer types across four target layouts, enum
identity, conversion placement, dead-arm diagnostics, static contexts, layout
queries, nesting, and runtime consumer syntax. VM oracles cover both Atari
runtimes, classic and raw/optimized MIR, selected calls, skipped division faults,
operand/argument preservation, eager result operators, short-circuit tests,
loops, indexed/compound assignments, signed/wide values, narrowing, and observed
volatile read order even when a result is unused. Wide execution retains the
existing MIR-only Atari restriction.

Slice-1 validation: 3,058 compiler tests passed with 22 pre-existing ignored;
the dedicated NIR snapshot command, all 46 NIR sweep fixtures, and
`cargo check --all-targets` passed. Four expression VM tests and 11 existing
guard/LET/known-tag VM regressions passed. A 64-level nested IF condition also
parsed and lowered successfully through the public CLI. The broader corpus
inventory increases from 337 to 338 successful sources for the new fixture;
its eight existing module-aware sweep exclusions are unchanged.

Slice 2 enables scalar/enum CASE expressions with mandatory ELSE, independently
matching integer/enum result types, existing label/range checks, and ordered
guards. Statement and expression bodies share semantic header validation,
resolved SemIR arms and both backend dispatch implementations. `CaseValue`
carries `SemCaseArm<SemExpr>` bodies; scalar expressions introduce no binders or
binding initialization. Each selected result supplies the same typed NIR join
used by IF. Classic projection keeps selector capture and selected-arm
preparation inside the expression.

Execution tests exposed a classic destination-preservation gap around prepared
expressions containing indexed writes. Preparation now conservatively triggers
pointer/index and operand staging. This is a general bug fix covering IF and
CASE, with regressions for indirect and fixed-array destinations. New raw and
optimized `case_expressions` fixtures establish the value-dispatch contract;
existing NIR snapshots remain unchanged. The broader corpus inventory grows
from 338 to 339 successful sources.

The focused checks cover required ELSE versus statement fallthrough, enum
identity and unnamed enum fallback values, ranges and shadowed guards, static
contexts, result/place legality, nested IF/CASE and runtime consumers, and joins
across all four target layouts. VM oracles cover selector mutation by guards,
selected-call and volatile-read order, skipped dynamic faults, eager result
operators versus short-circuit guards, unused results, loops, array stores,
signed ranges, narrowing and wide MIR labels/results.

Slice-2 validation: 3,061 compiler tests passed with 22 pre-existing ignored;
the dedicated snapshot command, all 47 NIR sweep fixtures, and
`cargo check --all-targets` passed. Nine IF/CASE VM tests and 14 existing
guard, nested-pattern, LET and known-tag VM regressions passed. The broader
corpus check passes with 339 successful sources and its eight unchanged
module-aware exclusions. Variant CASE values remain gated for slice 3.
