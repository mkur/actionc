# Choosing values with IF and CASE

Modern Action! supports IF and CASE as integer or enum expressions. Use them
in LET initializers, assignments, returns, arguments, indexes, arithmetic and
runtime conditions. Only the selected result expression executes.

## IF values

```action
LET smaller=IF a<b THEN a ELSE b FI
LET category=IF n=0 THEN 0 ELSEIF n<10 THEN 1 ELSE 2 FI
PrintBE(1+(IF ready THEN 2 ELSE 3 FI))
```

Every IF expression requires ELSE, even with a constant condition. Tests run
in order until one succeeds; later tests and unchosen results do not execute.
FI closes the expression, so a surrounding comma, operator or closing
parenthesis can follow it. Nested IF and CASE expressions are allowed.

## CASE values

```action
LET amount=CASE health OF
WHEN 0 THEN
  0
WHEN 1 TO 16 THEN
  1
ELSE
  IF health<32 THEN 3 ELSE 4 FI
ESAC
```

Keep WHEN headers on lines ending in THEN, with their results on following
lines. ELSE occupies its own line. A multiline CASE expression inside a guard
may end with `ESAC THEN`. Each arm contains exactly one expression; it cannot
contain assignments, LET, RETURN, a PROC call or a BEGIN/END statement block.
At least one WHEN is required. Mixed expression nesting is limited to 64.

The selector is evaluated once. Labels and ranges use the existing statement
CASE rules: `WHEN 1,3 THEN`, `WHEN 4 TO 8 THEN`, or enum member labels. A guard
such as `WHEN 1 TO 8 IF ready THEN` runs only after its labels match. A false
guard continues to later arms; only the first accepted result runs.

Integer and enum CASE expressions require ELSE, including a CASE listing all
named enum members. Enum labels must belong to the selector's enum; enum ranges
are unavailable. Statement CASE keeps its existing optional ELSE.

## Variant selectors and local USE

```action
TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]

BYTE FUNC ValueOrZero(MaybeByte item)
  USE ALL FROM MaybeByte
RETURN(CASE item OF
WHEN SOME(n) THEN
  n
WHEN NONE THEN
  0
ESAC)
```

An exhaustive set of unguarded variant patterns needs no ELSE. Guarded patterns
do not establish unconditional coverage. Nested patterns, alternatives, `_`
and immutable arm-local binders follow the
[variant matching rules](VARIANTS.md). Qualified patterns such as
`MaybeByte.SOME(n)` and generic patterns such as `Option<BYTE>.SOME(n)` also work.
Local USE opens named, non-generic variant types and keeps its existing scope
and collision rules; it does not accept `USE ALL FROM Option<BYTE>`.

The CASE retains a snapshot even if a guard or result call mutates its source.
An invalid outer or active nested tag invokes terminal Error(105) on Atari
before exposing payloads, running guards or selecting user ELSE. Returning from
Error does not resume the match. A known outer tag does not prove nested tags
valid. See the [validation contract](../VARIANT_CASE_VALIDATION.md).

## Exact result types and conversions

All results must independently have the same canonical integer type or the
same nominal enum identity. Ordinary literal typing still applies. There is
no implicit common-width search or C conditional-operator promotion.

```action
; health is BYTE; damage is CARD.
LET applied=IF CARD(health)<damage THEN CARD(health) ELSE damage FI
LET BYTE narrowed=applied
```

The CARD conversion inside the first arm makes both results CARD. The outer
BYTE annotation converts the completed value. An annotation or enclosing cast
cannot reconcile mismatched arms: `BYTE(IF flag THEN health ELSE damage FI)`
is still an error. Unselected arms are checked before optimization too.

IF tests and CASE guards use condition semantics, including short-circuit
logical conditions. Result expressions always use value semantics, including
eager bitwise AND/OR/XOR, even when the completed selection becomes a condition.
A comparison result has source type BYTE and can join another BYTE arm.

## Evaluation and support limits

Earlier operands and call arguments survive later selection effects. Indexed
assignment destinations are captured before the RHS; compound assignments
retain their existing load/store order. Loop conditions re-evaluate selections
on every test. FOR evaluates its start once and its end on each test. MIR6502
also supports selections in a runtime STEP, evaluated after each body; the
classic backend retains its constant-only STEP requirement. Unused results
still preserve calls, volatile reads, guards and selected-arm effects.

Selections are rvalues: they cannot be assigned to or have their storage
address taken. They are runtime expressions, so evaluated uses in CONST,
static initializers, array bounds and CASE labels are rejected, even for
literal conditions. Existing unevaluated layout queries do not execute them.

| Path | Supported results |
| --- | --- |
| Modern classic, Atari cartridge or standalone | BYTE, CARD, INT, LONGCARD, LONGINT, enums |
| Modern MIR6502, Atari cartridge or standalone | Same integer and enum results |
| Front end and verified NIR, all four target layouts | Integer/enum joins; native runtime fault support remains target-dependent |
| Compatibility profile | Selection expressions rejected |

REAL, pointer, record, union and variant **results**, plus statement blocks
that yield values, remain separate extensions. Variants are supported as
selectors. Existing target/type limitations continue to apply.

## Runnable example

[`samples/if-case-expressions.act`](../../samples/if-case-expressions.act)
prints `17`, `3`, `42`, `0`, `1` on separate lines. It combines explicit
widening, nested scalar selections, variant constructor arguments,
exhaustive matching and enum results.

```sh
cargo run --bin actionc -- --profile modern --backend classic --runtime standalone samples/if-case-expressions.act
cargo run --bin actionc -- --profile modern --backend mir6502 --runtime cart samples/if-case-expressions.act
```

The sample catalog builds both backends with both runtimes, and the VM suite
checks its printed numeric values through a memory capture procedure. See the
[implementation plan](../Action_2027/IF_CASE_EXPRESSIONS_IMPLEMENTATION_PLAN.md)
and [statement/expression codegen audit](../Action_2027/IF_CASE_EXPRESSIONS_CODEGEN_AUDIT.md).
