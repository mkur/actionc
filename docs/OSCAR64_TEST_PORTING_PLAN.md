# Oscar64 behavioral ports: second batch

Status: stages 1 through 4 ported, 2026-09-05; stage 5 pending.
Stage 4 now separates cartridge-compatible comparisons from the agreed modern
comparison-value extension. The 408 branch/count cases run in all modes; 264
value cases run in modern classic and MIR6502. Compatibility rejects numeric
comparison uses during semantic analysis. Both classic profiles also correct
signed-subtract overflow exposed by the boundary grid. See
[the contract and repairs](bugs/COMPARISON_VALUE_MATERIALIZATION_GAPS.md).

Current validation after the comparison-value implementation:

- Root `cargo test --quiet --no-fail-fast`: 2,662 passed, zero failed,
  22 existing ignored.
- Isolated VM `cargo test --locked --no-fail-fast --quiet`: 100 passed,
  zero failed or ignored. All 24 Oscar64 tests and 4,380 VM cases pass.
- The separate comparison-value consumer fixture passes another 24 VM cases
  across both modern backends and both runtimes.
- NIR snapshots and the 33-fixture sweep pass. The broad corpus contains
  315 valid roots and the same five declared module-only semantic failures.

Stages 1 through 3 now pass all six mode/runtime combinations. The classic computed
pointer-index regression exposed by stage 2 was fixed separately, without
changing the ports' expressions or oracles. See
[the regression note](bugs/CLASSIC_COMPUTED_POINTER_INDEX_BUG.md).

Stage 3 retains `fastcalltest.c`'s original nested call shape plus runtime
word inputs, repeated calls, per-argument evaluation counters and a byte-only
leaf companion. All 1,536 cases pass after generalizing protected argument
staging to Compatibility, recognizing calls through casts, and materializing
each stacked argument at the ABI base. The original 512 Compatibility failures
are repaired without changing port expressions or oracles. See
[the diagnosis and repair](bugs/CLASSIC_NESTED_CALL_ARGUMENT_BUG.md).

The separate classic word-return regression is now fixed. Return-fact inference
uses the existing register/value proof instead of treating two `Unknown`
descriptions as equal bytes. Before adding stage 4, root tests passed 2,656
checks (22 existing ignored), all 94 VM tests passed, and the NIR snapshot test
and 33-fixture sweep passed. Assignment, argument and pointer-index consumers,
multiple return paths, and the public return bytes have focused coverage.

Historical validation of the initial stage-4 translation, before checking
the cartridge language boundary and implementing modern comparison values:

- Root `cargo test --quiet`: 2,656 passed, zero failed, 22 existing ignored.
- Isolated VM `cargo test --locked --no-fail-fast --quiet`: 94 passed, three
  failed, none ignored. Only the three new stage-4 tests fail, at compilation.
- Oscar64 subset: 22 active tests, 19 passing; all existing 3,708 VM cases
  pass. The 408 intended stage-4 cases have not executed.
- Broad NIR corpus: 312 valid roots and the same five declared module-only
  semantic failures. Both new fixtures pass NIR validation. NIR snapshots
  and the 33-fixture sweep also pass.
- No failing mode was ignored. The subsequent cartridge probe established that
  numeric comparison values are an extension, motivating the profile split below.

Historical validation after the nested-call repair:

- Root `cargo test --quiet`: 2,654 passed, zero failed, 22 existing ignored.
- Isolated VM `cargo test --locked --no-fail-fast --quiet`: 94 passed, zero
  failed or ignored; all 19 Oscar64 tests and 3,708 VM cases pass.
- NIR snapshots and the 33-fixture sweep pass. The broad corpus remains at
  310 valid roots and the same five declared module-only semantic failures.
- Three focused execution tests cover byte/word/mixed arguments, casts,
  repeated calls, exactly-once counts, guarded inputs/results and stack balance
  in both classic profiles. Existing port expressions and oracles are unchanged.
- The separately documented optimized word-return accumulator-fact issue is
  outside this repair and remains a follow-up.

Historical validation including stage 3, before the nested-call repair:

- Root `cargo test --quiet`: 2,651 passed, zero failed, 22 existing ignored.
- Focused broad NIR corpus test passes: 310 valid roots and the same five
  declared module-only semantic failures; no lowering/verification/optimization
  failures.
- Isolated VM `cargo test --locked --no-fail-fast --quiet`: 93 passed, one
  failed, none ignored. The only failure is the new nested-call test.
- Oscar64 subset: 19 active tests, 18 passing and one failing; 3,708 VM cases,
  3,196 passing and 512 Compatibility failures. All prior 2,172 cases pass.
- No further compiler edits, ignored regressions, or weakened existing oracles.

Validation after the separate pointer-preservation repair (`590ef47`), before
adding stage 3:

- Root `cargo test --quiet`: 2,651 passed, zero failed, 22 existing ignored.
- Isolated VM `cargo test --locked --quiet`: 93 passed, zero failed or ignored;
  all 18 Oscar64 tests and 2,172 VM cases pass.
- NIR snapshots pass; the 33-fixture NIR sweep has no failures. The broad
  corpus expectation now includes the three added fixtures (309 valid roots
  and the same five declared module-only semantic failures).
- Original ported expressions and memory oracles remain unchanged.

Historical validation against compiler `37ee296` plus the initial ports:

- Root `cargo test --quiet`: 2,648 passed, zero failed, 22 existing ignored.
- Focused `cargo test --test nir_corpus --quiet`: passed.
- Isolated VM `cargo test --locked --no-fail-fast --quiet`: 92 passed, one
  failed, none ignored. The sole failure is the new classic reverse-copy test.
- Oscar64 subset: 18 tests, 17 passed and one failed; 2,172 executed VM cases,
  2,052 passing and 120 failing. The first 258 cases remain green.
- No compiler implementation changes, ignored regressions, or weakened oracles.

## Scope and baseline

Extend the existing eight fixtures and 258 passing VM cases with five
category-sized stages. Keep each category independently reviewable; compiler
fixes uncovered by a port are separate changes, not part of test translation.

Source: Oscar64 revision `8deb94c4d762bab3aa60c9565412691f01021bbb`,
`autotest/`, by drmortalwombat and contributors (GPL-3.0). Record provenance and
semantic adaptations in each fixture and in
[`fixtures/runtime/oscar64/README.md`](../fixtures/runtime/oscar64/README.md).

## Shared harness contract

- Extend `tools/vm-runtime-tests/tests/oscar64_conformance.rs`; do not introduce
  another runner or dependency.
- Run Compatibility, Optimized, and MIR6502 with ActionCart and Standalone.
- Derive expected values independently in Rust. Agreement between two compiled
  implementations is not the oracle.
- Use host-supplied inputs where constant folding could erase the operation.
- Check completion, complete result buffers, unchanged sources and inputs, and
  surrounding guards. Poison unwritten results, and reject buffers overlapping
  object segments before execution.
- Preserve Action! semantics explicitly: BYTE, INT, CARD, typed indexing,
  widening before shifts, and deliberate narrowing. Do not import C promotion,
  signed-byte, signed-overflow, or 32-bit assumptions.
- Compile once per fixture/mode/runtime and reuse the image across fresh VMs.
  Partition large grids into bounded tables/cases, with watchdog step limits,
  not benchmark thresholds.
- Preserve the first eight source fixtures and their existing case coverage.
- If a compiler failure appears, retain the triggering expression and correct
  oracle, isolate and document it, and keep its fix separate. Do not silently
  skip a mode, ignore a failure, or rewrite the fixture to conceal it.

## Stage 1: arithmetic composition

Sources: `shiftbyteaddconst.cpp`, `testsigned16mul.c`.

Add corresponding Action! fixtures and host-side tests:

- Shift explicitly word-widened BYTE inputs, then add/subtract an amount.
- Cover all 256 BYTE inputs and shifts 0 through 15; compare each literal and
  runtime operand form independently with the oracle.
- Retain addition amounts 15, 16, 111, 4096, 13421 and subtraction amounts
  15, 16, 4096. Use modulo-65536 CARD expectations, not C signed overflow.
- Expand the source's compile-time generated expressions into literal Action!
  expressions; removing a pragma alone would lose the constant/runtime axis.
- Multiply runtime INT values by each coefficient -16 through 15. Cover both
  operand orders and literal/runtime coefficients. Start with a bounded
  representative input grid in -1024 through 1024 so every product fits INT;
  document the reduced grid instead of claiming the original exhaustive sweep.
- Store results through indexes, including indexes around the word-element
  scale boundary at 127/128, with guarded backing buffers.

Acceptance: every expression result matches the independent oracle and no
source/input/guard bytes change.

## Stage 2: reverse indexing

Source: the 16-bit portions of `arraytest.c`.

- Retain the original 100-element sum/copy/reverse checks in BYTE- and
  word-index variants; omit the unsupported 32-bit variants.
- Exercise `destination(i)=source(n-i-1)` without staging away the two-index
  expression under test.
- Add lengths 0, 1, 2, 100, 127, 128, 129, 255, 256, 257. BYTE-count variants
  run only for representable lengths; do not truncate 256/257 into BYTE tests.
- Use aligned, odd, and near-page-boundary pointer bases and nonuniform word
  patterns with high bytes and signed boundaries.
- Keep source and destinations disjoint. This is not an overlap-safe copy test.
- Check every destination element, unchanged source, and guards: the original
  sum-only assertions cannot distinguish reversal from a forward copy.

Acceptance: both index widths preserve exact element order, including zero
length and page/scale crossings. A forward-copy mutant must fail the oracle.

## Stage 3: nested calls and argument preservation

Source: `fastcalltest.c`.

Retain nested calls with runtime inputs, nonzero word high bytes, and repeated
calls using different values. Check earlier arguments across later nested
calls and add observable counts for exactly-once argument evaluation.

The original word/multiplication shape tests call staging, not selection by the
current byte-only leaf inliner. Add a byte-only leaf companion for that path.
VM tests check results independently of whether a profitable call is inlined;
selection assertions remain in the focused compiler tests.

Implemented coverage uses all 256 byte values and a rotating set of 16 signed
word triples; every word product/sum fits INT, including the repeated sum.
Explicit BYTE narrowing supplies wraparound for the byte companion. A full
`$0600..$06FF` oracle checks outputs, poisoned gaps, unchanged inputs and three
exactly-once counters. It does not impose C's unspecified argument order or
require a particular inlining decision. Compatibility's former stack-staging
profile gate left earlier results vulnerable to later calls; the generic repair
is documented above.

## Stage 4: signed intervals and mixed comparisons

Sources: the INT portion of `testinterval.c`, and `mixsigncmptest.c`.

Cover intervals crossing zero, reversed predicate order, strict/inclusive
endpoints, and INT/BYTE comparisons in both operand orders. Extend existing
comparison coverage with broader runtime grids and branch/materialized-result
forms. Derive exact counts and truth values in Rust using established Action!
promotion rules. Omit signed-byte cases.

Implemented: `testinterval.act` retains the four 500-count checks and adds
runtime intervals, empty/reversed bounds, endpoint probes and eight explicit IF
forms (41 host cases). `mixsigncmptest.act` retains all four original sweeps
(one host case) and adds a 26-word by 256-byte grid for all six predicates in
both operand orders, with branch result tables. INT/BYTE
promotion follows Action!'s INT precedence; no signed BYTE is invented.
Full host-page/result-table guards and unchanged inputs are checked.

The original cartridge rejects comparison-as-value. `_values` companion
fixtures retain those expressions as modern-only coverage: 40 interval cases
and 26 mixed-comparison grids across two backends/two runtimes (264 VM cases).
The same independent truth/count oracles serve the branch and value tests.
This intentional profile split replaces the initial translation's assumption
that cartridge Action! supports numeric comparison results.

Modern classic and MIR6502 now reuse their comparison/branch machinery for
BYTE 0/1 results. Compatibility diagnoses value uses in semantic analysis.
The wider grid also repaired classic signed-subtract overflow; both profiles
now correct V before branching on N. Oscar64 totals are 4,380 VM cases in 24
tests. The separate modern consumer fixture adds 24 cases covering width,
composition, calls and indexed/pointer destinations. Stage 5 remains pending.

## Stage 5: record operations inside loops

Sources: `structarraycopy.c`, `structmembertest.c`.

Paused pending [fixed-length embedded array fields](EMBEDDED_RECORD_ARRAYS_IMPLEMENTATION_PLAN.md).
The `x[100]`/`y[100]` members in `structmembertest.c` must remain inline;
pointer fields or flattened backing tables would change the structure under
test. No stage-5 fixtures have been added yet.

Keep record structure: repeated record-array copies after conditional calls,
field-to-field copies through pointers, and array members within records.
Include mixed-width fields, page-crossing strides, and checks for unchanged
neighboring fields/source records/guards. Give conditional calls an observable
counter effect so they cannot disappear as empty routines.

## Validation and rollout

After each category, from `tools/vm-runtime-tests`:

```sh
cargo test --locked --test oscar64_conformance
```

From the repository root:

```sh
cargo test --test nir_corpus
```

Before handoff, run full root `cargo test` and, from `tools/vm-runtime-tests`,
full `cargo test --locked`. Any NIR/lowering/verifier or related compiler
changes also require the NIR snapshot test and 33-fixture sweep prescribed by
`AGENTS.md`. Update the coverage tables and distinguish passing cases from
newly exposed regressions; never report unexecuted cases as passing.

Implement stages 1 and 2 first, then reassess before stages 3 through 5. Use one
test-port commit per category when committing is requested, with any compiler
repairs separate. No optimizer changes or instruction-selection requirements
are part of these ports.

## Deferred batch

Signed division, full-range CARD division/remainder, volatile-device tests, and
additional unrolling cases remain follow-ups. CARD division/remainder first
needs the existing arithmetic compatibility audit. Volatile tests need
observable access counts or a deterministic test device, not only RAM's final
contents. Most named cases in Oscar64's `loopunrolltest.cpp` are disabled in
its entry point and must be assessed rather than copied wholesale.
