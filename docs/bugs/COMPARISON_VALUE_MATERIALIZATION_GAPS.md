# Comparison values: modern extension and shared comparison repairs

Status: implemented, 2026-09-05. Initially exposed by stage 4 of the Oscar64
behavioral ports, after the independent classic accumulator-return fact repair.

## Language boundary

Comparison-as-value is a modern Action! extension, not missing cartridge
compatibility. The original manual restricts relational expressions to
conditions (§4.3, p. 63) and explicitly forbids assigning them (§5.1.1, p. 67).
See the [Action! Reference Manual](https://www.noniandjim.com/Jim/atari/ACTION_Reference_Manual-Def_Ed.pdf).

Direct probes against the bundled Action! 3.6 cartridge rejected
`result=(x<j)`, `result=x<j`, `result=(x>=lo AND x<hi)`, and `RETURN(x<j)`
with error 17. Equivalent IF assignments compiled and ran correctly.

The agreed policy is:

- Modern classic and MIR6502 support all six comparisons as BYTE values,
  false = 0 and true = 1. Widening to INT/CARD zero-extends that result.
- Values work in assignments, arguments, returns, arithmetic/bitwise composition,
  indexes and modern scalar CONST expressions, using ordinary operand promotion.
- Value-context AND/OR/XOR remain eager bitwise operations. In particular,
  `(x<y) OR 2` is 2 or 3, not a normalized Boolean or a short-circuit operator.
  Ordinary conditional AND/OR retain their existing behavior.
- Compatibility reports `comparison values require the modern profile` during
  semantic analysis, before NIR or backend code generation. Conditional
  comparisons remain legal. The gate also applies inside call arguments,
  casts, indexes and arithmetic nested within a condition.

## Implementation and reuse

The existing semantic-options boundary owns the profile choice. Subject
classification distinguishes predicate from value context; only conditional
AND/OR propagate predicate context to their operands. Existing typed comparison
facts and NIR Compare operations remain unchanged.

Classic materializes 0/1 around its existing branch emitter. Comparisons now
participate in the existing expression-temporary predicates, so composed
operations and computed destinations use normal staging. The protected
comparison-operand path now also handles modern recursive operands and applies
the common promoted signedness. Operands are evaluated into the public value
frame; the captured left value is stacked and restored to pointer scratch only
after right evaluation, avoiding overlap with an indexed source pointer.
Modern binary temporary staging saves its
left result across recursive right evaluation rather than trusting a scratch
pair to survive calls or another comparison.

MIR6502 splits value-producing comparisons at their original operation site,
reusing the byte/word comparison branch expansion. True and false edges pass
1/0 into a BYTE continuation parameter. Existing block-argument lowering and
home assignment materialize that result. Later operations/effects stay in the
continuation; neither operand evaluation nor later calls are hoisted or skipped.
Single-use, branch-only comparisons retain the existing selection path.
Value expansion runs after that branch selection, preserving its existing
short-circuit and addressing choices. Branch selection also checks for numeric
uses in successor blocks before deleting a comparison definition.
No unresolved MIR is accepted by emission, and verifier requirements are intact.

## Signed subtraction overflow

Once the ports could run, their INT-boundary cases exposed a separate existing
classic bug: signed subtract-and-branch checked N without correcting for V.
For example, comparisons involving -32768 and positive operands could reverse.

The shared scalar, staged-slot, call/constant and indirect-word comparison
paths now branch on N xor V, correcting the high subtraction result with
`BVC; EOR #$80` before the sign branch. This correctness repair applies to
both classic profiles. It intentionally changes the old cartridge-shaped
instruction sequence; reproducing signed overflow miscomparisons is not a
compatibility requirement. Direct sign tests against zero remain unchanged
where subtraction cannot overflow.

The CIRCLE execution test had a classic-specific expected result encoding this
overflow bug. Its source is unchanged; both backends now use the existing
mathematically correct result, including the wrapped INT value of ABS(-32768).

## Coverage and test partition

| Category | Host cases | Modes | VM cases |
| --- | ---: | --- | ---: |
| Original/extended signed intervals, explicit IF results | 41 | All three | 246 |
| Original mixed comparison counts | 1 | All three | 6 |
| Mixed INT/BYTE branch grid | 26 | All three | 156 |
| Modern interval values | 40 | Optimized, MIR6502 | 160 |
| Modern mixed comparison values | 26 | Optimized, MIR6502 | 104 |

Every row uses both ActionCart and Standalone linking. The same independent
Rust truth/count oracles are retained; the source split is an intentional
profile distinction, not an ignored failing backend. The original 3,708
Oscar64 cases are supplemented by these 672 cases (4,380 total in 24 tests).

The separate `comparison_values.act` consumer fixture adds 24 execution cases:
BYTE/CARD/INT boundaries and equality, six predicates, nested comparisons,
bitwise/arithmetic composition, arguments and multiple returns, word
zero-extension, array/index boundaries, captured pointers, exactly-once calls,
call order, eager composed calls, unchanged inputs and full-buffer guards.
Semantic/API tests check early Compatibility rejection and modern acceptance.
Structural MIR tests check BYTE merges, preservation of later effects and a
result shared between a branch and a successor's numeric consumer.

From `tools/vm-runtime-tests`:

```sh
cargo test --locked --test oscar64_conformance oscar64_signed_intervals
cargo test --locked --test oscar64_conformance oscar64_mixed_signed_comparison
cargo test --locked --test comparison_values
```

Full validation and the next record-operation category are tracked in the
[porting plan](../OSCAR64_TEST_PORTING_PLAN.md). This extension does not start
stage 5 or add comparison-specific optimizations.
