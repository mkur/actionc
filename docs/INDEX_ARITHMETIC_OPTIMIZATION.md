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

Reproduce with:

```sh
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --example code_quality -- matrix1 matrix1-multidimensional jfdctint jfdctint-multidimensional
```

Local validation runs the compiler suite, NIR snapshots and the 51-fixture
sweep, plus full shaped/flat matrix1 and DCT corpora in both VM workspaces.
The sample parser check runs against a tracked source snapshot with the current
compiler library when unrelated untracked samples are present in the workspace.
