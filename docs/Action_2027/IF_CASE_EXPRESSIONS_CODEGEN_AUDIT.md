# IF/CASE statement and expression codegen audit

Measured on 2026-09-10 against slice 3 (`e9c7176`) with the slice-4 integration
tests. This slice changes tests and documentation, not compiler lowering or
optimization. [Raw measurements](IF_CASE_EXPRESSIONS_CODEGEN_AUDIT.csv) include
both Atari runtimes and both modern backends.

## Reproduce

From `tools/vm-runtime-tests`:

```sh
cargo test --locked --test if_case_codegen -- --nocapture
```

The [test](../../tools/vm-runtime-tests/tests/if_case_codegen.rs) generates the
complete sources and compiles the public pipeline at origin `$3000`. It runs
384 VM executions: four cases, two source forms, two backends, two runtimes,
six input bytes (`0,1,127,128,254,255`) and two flags (`0,1`). Every execution
checks the entire poisoned `$0600..$06FF` result/input page against an
independent Rust oracle. The VM is locked to revision
`7ec0cc454ebf43b088b7bcd11515533085ea1964`.

XEX bytes are the complete emitted object length, including segment headers,
data and any linked runtime support. Cycles run from object entry through the
completion-marker store, excluding the following infinite loop. Min/max are
over the twelve inputs for that form/backend/runtime. They are measurements,
not performance golden assertions or isolated per-join instruction counts.

## Programs and results

| Case | Statement form | Expression form |
| --- | --- | --- |
| `if` | Return `n` below 128, otherwise return wrapping `BYTE(n+1)` | Return the equivalent IF value |
| `case` | Return 7 for zero, `BYTE(n+1)` for 1..127, otherwise `n` | Return the equivalent CASE value |
| `known_some` | Construct SOME(42), match into a mutable BYTE local, store it | Construct SOME(42), bind a CASE result with LET, store it |
| `dynamic_variant` | Assign SOME(input) or NONE from a runtime flag; match/return in a function | Same setup, return the CASE value |

ActionCart results below use **statement → expression**:

| Case | Backend | XEX bytes | Min cycles | Max cycles |
| --- | --- | ---: | ---: | ---: |
| IF | Classic | 50 → 60 | 37 → 48 | 46 → 54 |
| IF | MIR6502 | 53 → 42 | 24 → 36 | 26 → 38 |
| Scalar CASE | Classic | 80 → 93 | 45 → 56 | 69 → 80 |
| Scalar CASE | MIR6502 | 59 → 59 | 39 → 42 | 50 → 53 |
| Known SOME | Classic | 152 → 156 | 178 → 182 | 178 → 182 |
| Known SOME | MIR6502 | 37 → 37 | 24 → 24 | 24 → 24 |
| Dynamic variant | Classic | 326 → 336 | 325 → 333 | 342 → 350 |
| Dynamic variant | MIR6502 | 209 → 209 | 126 → 129 | 155 → 156 |

Standalone cycle measurements are identical. Standalone XEX sizes are identical
except for the linked fault support in known-SOME classic (174 → 178 bytes),
dynamic-variant classic (348 → 358), and dynamic-variant MIR6502 (227 → 227).

Classic expressions add 4–13 bytes here and increase measured min/max cycles.
MIR6502 IF saves 11 bytes but adds 12 cycles; scalar CASE keeps its size and
adds three cycles. Dynamic variant keeps its size, with three extra minimum
cycles and one extra maximum cycle. These small programs include ordinary
join, return, inlining and layout decisions, so the results are not a universal
cost model for selecting a value. No example-specific optimization was added.

Known-SOME has identical MIR6502 size and cycles. The test independently
verifies raw NIR and optimizes through the verified pipeline, requiring both
known-SOME forms to lose all dispatch comparisons and fault calls. The dynamic
variant must retain its invalid-tag fault path. Execution checks still prove
the result on every input; equality between backends is not the oracle.

## Integration coverage

[`if_case_expressions.rs`](../../tools/vm-runtime-tests/tests/if_case_expressions.rs)
checks the public classic/MIR pipelines and raw/optimized NIR→MIR on both
runtimes, with MIR-only paths for wide integers and dynamic FOR steps:

| Consumer or invariant | Observable check |
| --- | --- |
| Indexed stores and compound updates | Captured pointer/index survive RHS calls changing both pointer and destination contents |
| Function and constructor arguments, returns | Earlier arguments survive nested selections and repeated calls; payload results match |
| IF/CASE guards and results | Source-ordered calls, skipped results/faults, condition short-circuiting versus eager result operators |
| Loop starts, ends, steps and tests | Start once, end on each FOR test, dynamic STEP per iteration in MIR, WHILE/UNTIL repeated selection effects |
| Unused values and volatile input | Required reads/calls remain, with observed bus access order |
| Signed and width boundaries | BYTE/CARD/INT conversions, full LONGCARD/LONGINT MIR joins and call preservation |
| Variant safety and snapshots | Mutating guards, nested payload validity and terminal invalid-tag faults even when Error returns |
| Published sample | Captured printed values are exactly 17, 3, 42, 0, 1 |

Module alias/local-USE identity is also checked in
[`local_variant_use.rs`](../../tests/local_variant_use.rs), including an imported
aggregate function result selected with opened and differently qualified
patterns. The [Oscar64 port](../../fixtures/runtime/oscar64/README.md) adds
1,344 independent mixed-width IF executions. These checks preserve the existing
constant-only classic FOR-step restriction and Atari MIR requirement for wide
integer execution.
