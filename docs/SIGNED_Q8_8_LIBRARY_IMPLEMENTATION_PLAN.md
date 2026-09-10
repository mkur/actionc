# Signed Q8.8 library and tests

Status: slice 1 complete; slices 2 and 3 pending. Created 2026-09-10.

## Scope

Add an embedded Action! library for signed Q8.8 arithmetic using existing INT
storage and LONGINT intermediates. Support Compatibility, Optimized classic,
and MIR6502 with both ActionCart and Standalone. Deliver three independently
tested slices and commit after each slice.

The library is ordinary Action source. SemIR, verified NIR, and the backends
continue to see existing integer types, casts, operations, and calls. Fixed
point scaling belongs to the library's API contract. No new language type,
literal syntax, IR operation, ABI, or compiler arithmetic helper is required.

This builds on [classic LONG support](CLASSIC_LONG_INTEGER_IMPLEMENTATION_PLAN.md),
[wide integer semantics and ABI](LONG_INTEGER_INTEGRATION_AND_MIR6502_PLAN.md),
and the [integer arithmetic contract](MODERN_INTEGER_ARITHMETIC_IMPLEMENTATION_PLAN.md).
Any compiler defect exposed by the library needs a general regression and a
separate repair, preserving those contracts and the verifier guarantees.

## Representation and numeric contract

- A value is an INT containing a signed, two's-complement 16-bit raw number.
  Its mathematical value is `raw / 256`. The eight integer bits include the
  sign; total storage is two bytes, not four.
- Range is exactly -128 through 127.99609375, in steps of 0.00390625.
  Raw 256 means 1, raw 384 means 1.5, and raw -384 means -1.5.
- Arithmetic and conversion results wrap modulo 65536 and are interpreted as
  signed INT. Narrow only after rescaling. There is no saturation or overflow
  status in this first API.
- Multiplication, division, ratio construction, and integer extraction
  truncate toward zero when discarding a fractional remainder. Negative
  values must follow the same rule in all modes and runtimes.
- Division by a zero raw value, including 0/0, uses the existing non-returning
  arithmetic fault, delivered as Error(101) on Atari. There is no result,
  destination store, or subsequent caller effect. Reuse ordinary division;
  do not implement a separate library error path.
- A literal zero passed to a library routine is a valid call expression;
  execution must fault. Do not require interprocedural compile-time rejection.
  Direct statically invalid arithmetic expressions retain existing diagnostics.
- Plain INT addition, subtraction, negation, equality, and signed ordering
  already operate correctly on raw values at the same scale. Document their
  wrapping behavior, including negation of raw -32768. They need no wrappers.
- Raw values are not a distinct type: the compiler cannot prevent mixing
  integer values, Q8.8 values, or other fixed point scales. Conversion and
  arithmetic APIs must make the interpretation explicit.

Use signed LONGINT operands before every multiplication that needs a wide
intermediate and before scaling a division numerator. `LONGINT(a*b)` would
already have lost the high product bits when a and b are INT. Use division by
256 for truncating signed rescaling; Action RSH is logical and is not the
specified rounding operation.

Every required intermediate fits LONGINT for every INT input. The largest
product is `(-32768)*(-32768) = 1073741824`; a scaled numerator lies between
-8388608 and 8388352. This library needs neither 64-bit storage nor a wider
multiply/divide kernel.

## Module and public API

Add `embedded/modules/math/q8_8.act`, declaring `MODULE MATH.Q8_8`.
Applications import it with `USE MATH.Q8_8 AS Q`. The existing embedded VFS
recursively collects module files; no manual registry or generated source edit
should be needed. Importing this module must not import the parent MATH REAL
facade or ATARI.REAL. Qualified imports and existing selective linking rules
apply; no names are added to the compatibility prelude.

Export INT constants `One=256`, `Half=128`, `Epsilon=1`, `MinValue=-32768`,
and `MaxValue=32767`. These are raw Q8.8 constants, not integer conversions.
Use existing typed CONST syntax, with explicit INT conversion where needed to
represent the minimum. Constants must not become mutable storage.

All five functions return INT:

| Function | Parameter interpretation | Raw result before final INT wrapping |
| --- | --- | --- |
| `FromInt(INT value)` | Ordinary integer | `value * 256` |
| `Trunc(INT value)` | Raw Q8.8 | `trunc_zero(value / 256)`, an ordinary integer |
| `FromRatio(INT numerator, denominator)` | Ordinary signed integers | `trunc_zero(numerator * 256 / denominator)` |
| `Mul(INT left, right)` | Both raw Q8.8 | `trunc_zero(left * right / 256)` |
| `Div(INT left, right)` | Both raw Q8.8 | `trunc_zero(left * 256 / right)` |

FromRatio and Div have the same raw calculation but communicate different
input meanings. They may share an implementation using ordinary calls.
Stage computations in explicitly typed locals so the module and its basic
examples stay within Compatibility's source-expression restrictions. Use
routine locals and existing scratch conventions; add no shared mutable
library state. This does not introduce an interrupt-safety or reentrancy
guarantee beyond the existing Action calling convention.

Illustrative pseudocode, not additional language syntax:

```text
Mul(left: INT, right: INT) -> INT:
    wide_left: LONGINT = sign_extend(left)
    wide_right: LONGINT = sign_extend(right)
    product: LONGINT = wide_left * wide_right
    scaled: LONGINT = product / LONGINT(256)   // truncates toward zero
    return wrap_to_INT(scaled)

Div(left: INT, right: INT) -> INT:
    wide_left: LONGINT = sign_extend(left)
    wide_right: LONGINT = sign_extend(right)
    numerator: LONGINT = wide_left * LONGINT(256)
    scaled: LONGINT = numerator / wide_right // existing zero-divisor fault
    return wrap_to_INT(scaled)
```

## Slice 1: module, constants, and integer conversions

1. Add the embedded module with constants, FromInt, and Trunc, plus concise
   API comments specifying representation, rounding, and wrapping.
2. Add `tests/fixed_q8_8.rs` for public `compile_file` coverage. Check qualified
   and aliased imports, typed constants, and conversion calls in all six
   mode/runtime combinations. Check that Q8.8 loads independently of REAL.
3. Add `fixtures/runtime/fixed_q8_8.act` and
   `tools/vm-runtime-tests/tests/fixed_q8_8.rs`. Feed raw signed inputs from the
   host and compare conversion results against an independent Rust oracle.
   Exercise negative fractions, integer boundaries, out-of-range FromInt
   inputs, and the two raw extrema with guarded output storage.
4. Add the initial API documentation at `docs/FIXED_POINT_Q8_8.md`, clearly
   identifying the implemented subset while subsequent slices remain pending.

Acceptance: a consumer imports the shipped module without a module search path,
and both conversions agree with the numeric contract across all six lanes.
Standalone numerical execution needs no OS or cartridge ROM. Commit this slice.

## Slice 2: multiplication, division, and ratio construction

1. Implement Mul, Div, and FromRatio with explicit signed LONGINT intermediates
   and final INT conversion. Extend public compilation coverage for every API.
2. Extend the VM fixture and oracle with a signed boundary cross-product,
   128 deterministic random pairs, and the targeted cases below. Include
   fractional remainders of both signs, extrema, underflow to zero, and wrapped
   outputs. Retain cases with nonzero upper product words.
3. Test Div and FromRatio zero denominators separately, including zero and
   nonzero numerators. Reuse the fault-probe pattern in `wide_integers.rs`:
   verify Error(101), unchanged caller destination and following-effect
   sentinels, and the defensive non-returning guard when an Error hook returns.
   An exhausted instruction budget alone is not evidence of the right fault.
4. Update the API document with all five functions and tested examples.

Acceptance: arithmetic matches independent mathematical expectations in all six
lanes; zero division follows the existing runtime contract. Commit this slice.

## Slice 3: composition, sample, and final acceptance

1. Add repeated and composed function-call cases, array and pointer stores at
   odd/page-crossing addresses, and neighboring-byte guards. Count effectful
   operand calls to check that each is evaluated once and earlier results
   survive later calls. Use staged calls for Compatibility where required by
   its existing grammar; cover nested expressions in supported modern modes.
2. Verify ordinary raw addition, subtraction, negation, and signed comparisons
   in a small motion calculation that uses FromRatio, Mul, Div, and Trunc.
   Feed dynamic inputs so constant folding cannot replace the numerical path.
3. Add `samples/fixed-point/q8_8.act` and its README with exact expected output,
   raw-value examples, and build commands. Keep the sample usable in all six
   lanes; use existing integer output and explain its scale instead of adding
   decimal formatting or REAL conversion. Exercise the maintained sample in
   the VM test target against its documented output.
4. Link the API guide from `docs/README.md`, the module usage reference, and
   sample indexes. Update the runtime fixture and VM harness READMEs with the
   focused test command. Record actual coverage and mark this plan complete
   only after final checks pass.

Acceptance: the public sample, composed uses, existing LONG tests, and full
compiler/VM suites pass. Commit this slice. Optimization and performance
comparisons remain separate work after correctness is established.

## Oracle and regression requirements

Use host i64 arithmetic and explicit signed-16-bit wrapping. Compute expected
values independently of compiler evaluators, helpers, and generated output;
do not use floating point or compare backends as the sole oracle. Rust signed
integer division supplies the chosen truncation rule for nonzero divisors.

The boundary set should include raw -32768, -32767, -1024, -513, -512, -257,
-256, -129, -128, -1, 0, 1, 127, 128, 255, 256, 257, 512, 1024, 32766, 32767.
Cover unary conversions over this set and additional FromInt inputs around
-129, -128, 127, and 128. Exercise every ordered pair for binary operations;
zero-divisor cases use the separate fault fixture. Use a fixed, documented
random seed and report failing inputs with mode/runtime information.

These raw examples pin down the contract:

| Operation | Expected INT result | What it checks |
| --- | --- | --- |
| `FromInt(1)` | 256 | Integer-to-fixed scaling |
| `FromInt(128)` | -32768 | Conversion overflow wraps |
| `FromRatio(3,2)` | 384 | Exact fractional construction |
| `FromRatio(1,256)` | 1 | Smallest positive step |
| `Trunc(-384)` | -1 | Integer extraction truncates toward zero |
| `Mul(384,512)` | 768 | 1.5 times 2, preserving the wide product |
| `Mul(-1,1)` | 0 | Tiny negative product does not round down to -1 |
| `Mul(-257,128)` | -128 | Negative half-step remainder |
| `Mul(32767,512)` | -2 | Rescale before wrapping |
| `Div(-256,768)` | -85 | Negative non-integral quotient |
| `Div(-32768,-256)` | -32768 | Positive quotient overflow wraps |
| `Div(0,0)` | Error(101), no return | No invalid zero-numerator shortcut |

Compile fixtures once per mode/runtime and reuse them with externally supplied
inputs. Batch cases where practical; do not compile a separate program per
pair or exhaustively enumerate all 2^32 operand pairs. Check complete guarded
output regions and completion/effect markers, not only a checksum. Keep
conversion, binary arithmetic, faults, and integration failures distinguishable.

## Validation commands

For each implementation slice, run the relevant new tests from the repository
root and the isolated VM harness respectively:

```sh
cargo test --test fixed_q8_8
```

From `tools/vm-runtime-tests`:

```sh
cargo test --locked --test fixed_q8_8
```

At final acceptance, run the root checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
cargo check --all-targets
```

Then run `cargo test --locked --no-fail-fast` from `tools/vm-runtime-tests`.
If a slice changes semantic lowering, NIR, verification, printing, or related
compiler code, run the required root checks before committing that slice as
well. Existing IR snapshots should remain unchanged for a library-only change;
explain any compiler-regression fixture changes explicitly.

## Progress

Slice 1 is complete. The embedded module exports typed constants, FromInt,
and Trunc without importing MATH or ATARI.REAL. Both public compiler tests
pass, including 12 import/mode/runtime compilations. The VM conversion test
passes 150 host-fed executions (25 signed inputs across six lanes), checking
complete guarded output regions. Standalone runs load no ROMs. No compiler
code or IR snapshots changed.

## Deferred

Native fixed point types and literals; unsigned Q8.8; Q4.12, Q16.16, Q24.8;
implicit mixed-scale arithmetic; saturating/checked variants; floor and
round-to-nearest APIs; decimal parsing/formatting; REAL conversion; square root,
trigonometry, and graphics ports; specialized 6502 kernels and codegen audits.
This is a first-party library, not an Oscar64 or Mad Pascal source translation.
Any later upstream port needs its own pinned provenance and semantic mapping.
