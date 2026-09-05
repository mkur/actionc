# Oscar64 behavioral ports: second batch

Status: stages 1 and 2 ported, 2026-09-05; stages 3 through 5 planned.
Stage 1 passes all six mode/runtime combinations. Stage 2 passes MIR6502 but
exposes an active classic-backend regression; its correctness acceptance gate
is not green. See [the regression note](bugs/CLASSIC_COMPUTED_POINTER_INDEX_BUG.md).

Validation against compiler `37ee296` plus these ports:

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

## Stage 4: signed intervals and mixed comparisons

Sources: the INT portion of `testinterval.c`, and `mixsigncmptest.c`.

Cover intervals crossing zero, reversed predicate order, strict/inclusive
endpoints, and INT/BYTE comparisons in both operand orders. Extend existing
comparison coverage with broader runtime grids and branch/materialized-result
forms. Derive exact counts and truth values in Rust using established Action!
promotion rules. Omit signed-byte cases.

## Stage 5: record operations inside loops

Sources: `structarraycopy.c`, `structmembertest.c`.

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
