# Composed layout queries shared cached values

Status: fixed, 2026-09-05. Discovered while implementing named-module
constant/record-layout dependencies for embedded fixed-length array fields.

## Failure and cause

For `TYPE Pair=[BYTE tag CARD word]`, this Atari constant must be 5:

```action
CONST Count=SIZEOF(Pair)+OFFSETOF(Pair,word)+ALIGNOF(Pair)
```

It could instead become 9. The expression parser discarded nested token
ranges and gave every nested expression the enclosing expression's span.
Semantic layout queries are cached by scope and span, so the three distinct
queries reused the first result. SemIR could also mistake an ordinary runtime
call alongside a layout query for the cached compile-time query.

## Repair

The expression parser now carries `Expr` nodes with their actual token ranges
through prefix, postfix and binary parsing. Compound assignment expansion
preserves the repeated target's original range. The semantic cache and SemIR
continue using their existing scope/site keys; no source-text matching or
layout-specific expression evaluator was added.

Classic same-record-pointer reuse now uses the existing lvalue-equivalence
helper instead of comparing source spans. Its existing byte/word/bitwise
optimization tests remain active. The inline-string listing test now points
to the string literal's column rather than the enclosing call's column; that
expectation change is source-location metadata only.

## Regression coverage

- Parser tests verify distinct call/callee/argument ranges and preserved
  compound-assignment target/RHS ranges.
- `tests/layout_query_composition.rs` compares emitted object bytes against
  literal equivalents in all three public modes and both runtimes, covering
  repeated SIZEOF calls, mixed layout queries, casts and runtime calls on
  either side of a layout query.
- Named-module dependency tests additionally compose these queries in an
  embedded-array bound, including naturally aligned target layouts.
