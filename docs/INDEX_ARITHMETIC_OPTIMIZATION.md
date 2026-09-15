# Index arithmetic optimization

Fixed multidimensional indexes are already normalized into typed NIR casts,
integer arithmetic and ordinary indexed places. Optimize those facts; never
recover dimensions or source syntax in a target backend.

## 68K constant multiplication

MIR68K selects bounded shift/add/subtract sequences for constant factors of the
form `2^a`, `(2^a +/- 1) * 2^b`, and their modular negatives. The calculation
uses the resolved byte/word/long width, so signed and unsigned low products
have identical overflow behavior. Both constant operand positions are accepted
only after source operands have been captured. Dense or costly constants keep
the general multiply implementation. The existing `select_instructions` option
controls this choice independently of NIR optimization.

The selection budget estimates original MC68000 instruction cost, rather than
minimizing instruction count alone: a short shift/add sequence can execute more
instructions than one word MUL while costing fewer cycles. Measurements below
are VM instruction counts, not cycle or wall-clock benchmarks.

## Shared NIR index reuse

The backward slice of ordinary Index coordinates identifies integer casts and
add/subtract/multiply computations. Equal typed expressions may share a captured
result within 64 operations of the same block. This bounded exception to the
general GVN lifetime policy does not reuse loads or array descriptors, reassociate
source arithmetic, or move computations between blocks. Calls, foreign code,
volatile accesses and real operations end the reuse region. Replacements include
all successor uses and edge arguments; verification remains mandatory.

## Loop-invariant arithmetic

Shared NIR recognizes reducible natural loops with a dedicated existing
preheader. It moves only total integer casts, unary operations and add/subtract/
multiply operations in the index dependency graph. Every operand must already
dominate the preheader or be another proven invariant. Inner loops are processed
first; no CFG edges are split. At most 64 operations move per loop.

Loads, descriptors, division and other potentially faulting operations never
move. Loops containing calls, foreign code, real operations or volatile accesses
(including volatile copies) are excluded. Computing total integer expressions on
a zero-trip path is unobservable. All source widths remain explicit.

## Incremental offsets

Shared NIR can replace generated ADDRESS multiplication by a non-power-of-two
constant stride with an additional header parameter. The preheader computes the
initial product; the sole backedge advances it by `step * stride`, modulo the
target ADDRESS width. At most four products become carried values per loop.
Power-of-two strides retain their cheap shift form, and ordinary source
multiplications keep their existing policy.

The counter must already be in SSA, increase by a positive constant, and have a
header comparison against a constant upper bound. That bound must prove that
even the final source-width update cannot wrap. A single integer widening cast
is allowed; narrowing or nonlinear coordinate calculations are excluded. The
product must dominate the backedge. Unsupported loop shapes keep recomputation.

This transform replaces a computation proved equal to the carried value; it does
not move any source evaluation. Its only inputs are the counter and constants,
so calls cannot change the recurrence. Descriptor reads and RHS calls retain
their original order, including calls that rebind the array. Signed extension
and zero-trip behavior are preserved.

## Validation and measurements

The constant multiplication oracle exercises all five scalar integer types,
powers of two, positive/negative sparse and dense constants, both operand
positions, boundary and deterministic random inputs, and selection on/off.
Existing call/capture tests, instruction selection, multidimensional execution,
and complete matrix1 and DCT C corpora also cover the change.

Same uninstrumented workloads and options as
[MULTIDIMENSIONAL_ARRAYS_VALIDATION.md](MULTIDIMENSIONAL_ARRAYS_VALIDATION.md).

| Stage | Matrix1 shaped instructions | DCT shaped instructions | Matrix1 code bytes | DCT code bytes |
| --- | ---: | ---: | ---: | ---: |
| Initial, optimized NIR | 223833 | 50499 | 1550 | 8044 |
| Constant multiplication | 175833 | 44739 | 1370 | 6952 |
| Local index reuse | 164833 | 43723 | 1344 | 6646 |
| Loop-invariant arithmetic | 144207 | 43279 | 1326 | 6658 |
| Incremental offsets | 144051 | 43279 | 1332 | 6658 |

Reproduce with:

```sh
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --example code_quality -- matrix1 matrix1-multidimensional jfdctint jfdctint-multidimensional
```

Local validation runs the compiler suite, NIR snapshots and the 51-fixture
sweep, plus full shaped/flat matrix1 and DCT corpora in both VM workspaces.
The sample parser check runs against a tracked source snapshot with the current
compiler library when unrelated untracked samples are present in the workspace.

The final shaped frames are 88 bytes for matrix1 and 886 bytes for DCT, down from
124 and 1170. The incremental slice has a small additional benefit after
hoisting has already removed the hot inner-loop products. It deliberately
retains DCT's power-of-two strides: carrying those offsets increased stack
traffic in measurement. Against the original shaped versions, final instruction
counts improve about 36% for matrix1 and 14% for DCT. There is still substantial
room between the shaped matrix and its flat/pointer baseline.

Focused regressions cover counter wrap rejection, nonlinear/narrowing casts,
16/24/32-bit ADDRESS verification, and 6502 execution with a promoted word
counter. Native execution additionally checks signed negative coordinates,
indices beyond 64 KB, exact volatile traces, zero-trip loops, and array rebinding
during RHS calls. The 65816 targets have NIR verification coverage, not VM
execution coverage.
