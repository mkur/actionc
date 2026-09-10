# Oscar64 behavioral test ports

These twenty-five Action! fixtures adapt Oscar64 autotests and a fractal sample into the existing
[isolated VM harness](../../../tools/vm-runtime-tests/README.md). They test
observable results, not a preferred instruction sequence or agreement between
backends. Expected results are calculated independently in
[`oscar64_conformance.rs`](../../../tools/vm-runtime-tests/tests/oscar64_conformance.rs)
and [`oscar64_mandelbrot.rs`](../../../tools/vm-runtime-tests/tests/oscar64_mandelbrot.rs).

## Provenance

Adapted on 2026-09-05, 2026-09-06 and 2026-09-10 from Oscar64 by drmortalwombat and contributors:

- Upstream: <https://github.com/drmortalwombat/oscar64>.
- Inspected fork: <https://github.com/mkur/oscar64>.
- Source revision: `8deb94c4d762bab3aa60c9565412691f01021bbb`.
- Original paths: `autotest/<fixture-stem>.c`, listed below, except
  `shiftbyteaddconst.cpp` and `rolrortest.cpp`. Both `rolrortest.act` and its
  `rolrortest_wide.act` companion adapt `rolrortest.cpp`.
  The additional `mbfixed.act` adapts `samples/fractals/mbfixed.c`; its shared
  source and provenance live in [the sample project](../../../samples/graphics/mandelbrot/README.md).
- The source repository supplies GNU GPL version 3 in its `LICENSE`; the
  selected files have no additional per-file license notice. These adaptations
  retain GPL-3.0 attribution. The license text is in this repository's
  [LICENSE](../../../LICENSE).

The ports change syntax, expose results in memory instead of C assertions or
process return codes, and add the cases listed below. They are not literal C
translations or tests of C-only language behavior.

## Coverage

| Source / Action! fixture | Original check retained | Additional coverage |
| --- | --- | --- |
| `mbfixed` (fractal sample) | 160x100 coordinate mapping, 32-iteration recurrence, wide radius check, floor-rounded cross product | Every axis position, 128 seeded pixels, rounding-sensitive pixels, direct radius-boundary inputs and repeated calls in all six lanes; probe output, selected Atari rows in all lanes and full standalone MIR6502 image |
| `byteindextest` | Fill 20 bytes with their indexes; byte sum is 190 | Fixed odd-base storage and a descriptor viewing the same backing; lengths 0, 1, 20, 127, 128, 255, 256, 257 |
| `arrayindexintrangecheck` | Get/Put calls on ten INT elements; sum minus 45 is zero | Fixed odd-base words at indexes 0, 1, 127, 128, 255, 256, with runtime-supplied values |
| `arrayoffsetindex` | Four stores through `p(x+3)` through `p(x+6)`; sum minus 10 is zero | Two runtime pointer bases, including `$50F1`, and starting arguments 4, 123, 124, 252 |
| `copyintvec` | Copy 100 INT elements; sum minus 4950 is zero | Runtime pointer arguments; lengths 0, 1, 100, 127, 128, 129, 255, 256, 257; unchanged source and destination guards |
| `incvector` | Increment 100 INT elements; sum minus 5050 is zero | The same extended lengths; odd pointer base; carry, signed-boundary, and wraparound values |
| `loopboundtest` | All 16 sums: lengths 50/100, strict/inclusive ascending/descending loops, INT/CARD accumulators | Odd fixed base, independently computed sums, final table contents and guards |
| `cmprangeshortcuttest` | Signed/unsigned induction over 5..14; exact counts for six comparison predicates | All five thresholds 4, 5, 10, 14, 15 for every predicate: 60 counters |
| `maskcheck` | Every byte input and eight literal masks; four masked equality/inequality forms | Independently checks every truth value, not just equality of paired results: 8,192 result bytes |
| `shiftbyteaddconst` (`.cpp` source) | Shift a widened byte by 0..15 and add 15/16/111/4096/13421 or subtract 15/16/4096 | All 256 byte inputs; all four literal/runtime shift/amount combinations; explicitly unsigned word wrapping; guarded indexed destinations, including 127/128 |
| `testsigned16mul` | Multiply runtime INT by every coefficient -16..15, contrasting literal expansion with the runtime loop | Both operand orders; 33 representative inputs in -1024..1024 (not the original exhaustive outer sweep); guarded destination indexes up to 255 |
| `arraytest` (16-bit portions) | All six sum/copy/reverse checks on 100 words with BYTE/INT indexes; original sums are 450 | Exact forward/reversed contents and unchanged sources; disjoint buffers at three base layouts; lengths 0, 1, 2, 100, 127, 128, 129, 255, 256, 257; BYTE-count variants only through 255 |
| `fastcalltest` | Nested `P1(5,P2(C2(2),C2(4)))-13` is zero | 16 representative signed word triples with nonzero high bytes; repeated calls, exactly-once argument counters, and a private byte-leaf companion across all 256 byte values; unchanged inputs/full-page guards |
| `testinterval` (INT portions) | Four counts of 500 for equivalent signed interval predicates | Runtime intervals across zero and near INT limits, strict/inclusive endpoints, reversed predicate order, empty/reversed bounds, and explicit IF endpoint-probe results |
| `mixsigncmptest` | Four signed-word/unsigned-byte sweeps with exact true/false counts | 26 signed word inputs against all 256 BYTE values; all six predicates in both operand orders, branch results, odd-base guarded tables |
| `testinterval_values`, `mixsigncmptest_values` | Modern-only companions to the two preceding ports | The same interval and mixed-comparison truth oracles, using numeric comparison values instead of IF assignments |
| `structarraycopy` | Eight Point record-array copies following three conditional calls; sum plus 144 is zero | Exactly 12 original calls; twice-repeated seven-byte mixed-width record copies; runtime lengths 0, 1, 2, 8, 100, 127, 128, 129, 255, 256, 257 at aligned, odd and page-boundary bases; complete images and guards |
| `structmembertest` | The original record with inline `INT x(100),y(100)` and 100 three-INT vectors; pointer-based field-copy loops | Modern-only; additional inline arrays of 257 INTs and seven-byte tagged vectors; runtime lengths 0, 1, 2, 100, 127, 128, 129, 255, 256, 257 at three base layouts; all fields, source members, unused elements and guards |
| `testsigned16div` | Runtime INT divided by each nonzero coefficient -16..15; actual literal expansion contrasted with the original runtime loop | 39 representative signed inputs, including inexact quotients; guarded odd-base literal/runtime tables and unchanged inputs |
| `divmodtest` | Quotient/remainder identities with byte divisors 1..255 and wrapping CARD power-of-three divisors | Every original outer-loop value supplied by the host; exact quotient and remainder checked independently, full CARD high-bit range, guarded odd-base row table |
| `mixedwidthternary` | Player health 17, maximum 17, severity 0 produces damage 3 and health 14; empty inventory stays NONE | Modern IF values with explicit CARD arms and outer BYTE narrowing; 336 health/maximum/severity inputs, repeated damage and consumption, companion clamping amounts through 65535, complete-page guards |
| `enumswitch` | E1/E2/E3 return 10/20/30 and E4 takes the default 100; four-call sum minus 160 is zero | Statement and expression CASE with INT results; all 256 enum representations, repeated calls with `255-tag`, unchanged input and complete-page guards |
| `rolrortest` (8/16-bit portions of `.cpp` source) | Original `$12`/`$1234` seeds, inverse rotations and full-cycle right tables checked against left rotations | Every BYTE input, 42 representative CARD inputs; each left/right table entry checked independently, odd-base word tables, unchanged inputs and complete-page guards |
| `rolrortest_wide` (32-bit companion) | Original `$12345678` seed, inverse rotation and full 32-bit rotation cycle | LONGCARD in all modes; 76 seeds including every single-set/single-clear bit, alternating patterns and byte boundaries; independent left/right tables with odd bases and guards |

Cartridge-compatible sources run in all three modes; modern comparison-value
companions, `structmembertest`, `mixedwidthternary` and `enumswitch` run in
Optimized and MIR6502. `rolrortest_wide` runs in all three modes. All use both ActionCart and Standalone runtime linking.
The first eight ports retain **258 VM cases in fourteen passing tests**. Both MIR6502
copy/increment regressions remain active with their original loops and
independent expected values.

The [second-batch plan](../../../docs/OSCAR64_TEST_PORTING_PLAN.md) has stages
1 through 5 ported. Results, including the later focused ports, are:

| Fixture | Host cases | VM cases | Result |
| --- | ---: | ---: | --- |
| `shiftbyteaddconst` | 256 | 1,536 | All pass |
| `testsigned16mul` | 33 | 198 | All pass |
| `arraytest` | 30 | 180 | All pass |
| `fastcalltest` | 256 | 1,536 | All pass |
| `testinterval` | 41 | 246 | All pass |
| `mixsigncmptest` | 27 | 162 | All pass |
| `testinterval_values` | 40 | 160 | Both modern backends/runtimes |
| `mixsigncmptest_values` | 26 | 104 | Both modern backends/runtimes |
| `structarraycopy` | 33 | 198 | All six mode/runtime combinations |
| `structmembertest` | 30 | 120 | Both modern backends/runtimes; Compatibility rejection checked separately |
| `testsigned16div` | 39 | 234 | All six mode/runtime combinations |
| `divmodtest` | 1,190 | 7,140 | All six mode/runtime combinations |
| `mixedwidthternary` (expression integration port) | 336 | 1,344 | Both modern backends/runtimes; Compatibility rejection checked separately |
| `enumswitch` (CASE integration port) | 256 | 1,024 | Both modern backends/runtimes; Compatibility rejection checked separately |
| `rolrortest` (BYTE/CARD rotations) | 257 | 1,542 | All six mode/runtime combinations |
| `rolrortest_wide` (LONGCARD rotations) | 76 | 456 | All six mode/runtime combinations |

The conformance target covers **16,438 VM cases in 32 active tests**: the previous 14,440 cases plus
1,542 BYTE/CARD and 456 LONGCARD rotation executions. Profile/backend rejection
checks for member arrays, enums and selection expressions are
not VM cases.
No test is ignored
or expects a panic. Both the nested-call and classic reverse-copy regressions were repaired
without changing fixture expressions or oracles. See
[OSCAR-CLASSIC-COMPUTED-INDEX](#oscar-classic-computed-index--fixed).
Stage 4 now distinguishes the modern comparison-value extension from cartridge
syntax and repairs signed-subtract overflow in both classic profiles; see
[the contract and repairs](../../../docs/bugs/COMPARISON_VALUE_MATERIALIZATION_GAPS.md).

The separate `oscar64_mandelbrot` target adds 2,424 numerical executions (404
pixel cases across six lanes, each also checking a direct raw-coordinate case)
and six probe-output runs. Another 2,904 cases verify dimension-aware display
coordinates. Six selected-row renders and one full native-resolution 160x192
standalone MIR6502 image bring its total to 5,341 executions in five tests. All share the
maintained kernel with the sample project. Graphics assertions check the VM's
modeled CIO pixels and palette against an independent image oracle, not ANTIC
scanout or OS screen-memory packing. These counts are separate from the
conformance table above.
The integer model finds 180 viewport pixels where floor and truncation differ;
the VM corpus includes explicit examples of that difference. No Oscar64 binary
or floating-point Mandelbrot implementation is used as the oracle.

## Action! semantics and harness contract

- `mbfixed` preserves separate square rescaling, pre-update escape checks,
  signed floor multiplication, the original constant truncation, and the
  32-iteration cap. Q4.12 SqrWide returns the complete Q8.24 LONGCARD square;
  the radius threshold is 0x04000000. The fixture imports the shared kernel
  with the sample project on its module path. A host-supplied NOP at $0700
  provides an explicit execution endpoint after the completion marker, so a
  watchdog expiration cannot count as success. Full-page guards cover inputs,
  outputs, and gaps. Pure numerical standalone cases load no ROMs.
- `rolrortest` and `rolrortest_wide` adapt the
  [pinned C++ source](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/autotest/rolrortest.cpp).
  They retain its shift-plus-add function bodies, right-rotation tables,
  left-after-right checks and inclusive reverse-table comparisons. BYTE, CARD
  and LONGCARD supply the original unsigned 8/16/32-bit widths; logical right
  shifts and wrapping left shifts require no C promotion rules. Assertion
  failures increment an observable counter that must remain zero. The host
  also checks every left/right table entry using Rust's `rotate_left` and
  `rotate_right`, plus the final value after width+1 left rotations, so matching
  errors in the two directions cannot hide behind an inverse-only check.
  The narrow port has one original seed pair and 256 byte cases cycling through
  42 word seeds. The wide companion has 76 distinct seeds, including the
  original and every single-set/single-clear bit. Word coverage is sampled;
  only the byte input domain is exhaustive. Odd-base tables, unchanged inputs
  and poisoned gaps detect storage damage. The source's `__noinline` attribute
  is omitted; these tests check behavior without constraining instruction
  selection. The 32-bit portion uses LONGCARD directly in MIR6502, with separate
  rejection checks in both classic modes.
- `enumswitch` adapts the
  [pinned C source](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/autotest/enumswitch.c)
  as statement and expression CASE functions. Each retains the original
  E1/E2/E3/E4 calls and sum-minus-160 check. Action! enums have nominal identity
  and BYTE storage; the host supplies all 256 representations via an explicit
  `E(tag)` cast. E4 and the unnamed values 4..255 must take ELSE and return 100.
  Expression arms explicitly produce INT, preserving the signed-word return
  type without relying on implicit arm widening. Each VM invocation also calls
  both functions with `255-tag`, checking repeated dispatch after a changed
  argument. The host independently checks every result word, the unchanged
  input and all remaining poisoned page bytes. This covers Action!'s enum
  representation domain, without assuming C enum layout or promotion rules.
- `mixedwidthternary` adapts the
  [pinned C source](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/autotest/mixedwidthternary.c)
  as the focused [IF/CASE integration port](../../../docs/Action_2027/IF_CASE_EXPRESSIONS_IMPLEMENTATION_PLAN.md).
  It preserves the original 3-damage/14-health check on every invocation.
  Each IF arm is explicitly CARD before an enclosing BYTE annotation or cast
  narrows the result; no C ternary promotion is assumed. The host crosses 14
  health values with eight maximum-health values and three severities. It
  cycles eight companion amounts through 65535 and both valid inventory enum
  values, checking repeated damage and consumption plus complete-page guards.
  The original damage formula reaches at most 102; the companion exercises
  high-word clamping independently. The original null-inventory diagnostic
  branch is outside its valid-input main case and is omitted: the port supplies
  valid one-element arrays. This isolated port does not resume the deferred
  volatile batch.
- Arithmetic ports use the [modern integer contract](../../../docs/MODERN_INTEGER_ARITHMETIC_IMPLEMENTATION_PLAN.md)
  in every profile/runtime. `testsigned16div` retains all 31 nonzero literal
  divisors explicitly; its 39 host inputs are a sample, not the original
  exhaustive signed outer loop. `divmodtest` retains all original outer grids:
  0..255 step 11, 0..6999 step 11, and 0..63999 step 121 (24+637+529 inputs).
  The host supplies those outer values so each VM invocation has a bounded
  watchdog; original inner loops remain in Action! code. Quotient/remainder
  rows are checked directly, not only through a possibly wrapping identity.
  Division-by-zero and signed overflow tests live separately in
  `modern_arithmetic.rs`, not in these valid C-domain translations.
- Oscar64's unsigned byte `char` maps to `BYTE`, signed word `int` to `INT`,
  and unsigned word to `CARD`. There is no invented signed-byte Action! type.
- C `for` predicates become explicit `WHILE` predicates. In particular,
  descending signed loops preserve the termination at -1, and unsigned-sum
  variants retain signed induction variables as in the originals.
- Unsigned accumulators in `loopboundtest` still return INT, as in the C
  functions. `INT(x)` makes that conversion explicit. All these sums fit INT.
- The single-array C struct in `loopboundtest` is flattened to an INT array
  and an INT pointer parameter; this batch does not test record layout.
- Typed pointer indexing scales by element size. Runtime addresses are loaded
  from fixed CARD input cells, then assigned to declared INT pointers. A
  pointer declaration initializer would set its value, not its storage address.
- Explicit `BYTE(...)` narrowing preserves byte wrapping, and CARD operands
  keep the extended indexes/offsets word-sized. No C integer-promotion rules
  are assumed for dynamic Action! BYTE expressions.
- `shiftbyteaddconst` uses `CARD(value)` before shifting and CARD amounts.
  Rust computes in a wider unsigned type and truncates to 16 bits. The four
  result tables each have their own expected words, so identical wrong
  constant/runtime implementations cannot mask a failure. Literal expressions
  explicitly replace Oscar64's templates and compile-time loops. This larger
  fixture uses `ORG $2000` to keep its expanded code below its host workspace.
- `testsigned16mul` explicitly expands literal coefficients in both operand
  orders. Its 33 host inputs cover zero, signs, and neighbors of byte/word
  boundaries within -1024..1024; all 4,224 products per mode/runtime fit INT.
  This deliberately samples the original 2,049-input outer sweep rather than
  claiming exhaustive signed multiplication coverage.
- `arraytest` retains `d(i)=s(n-i-1)` with no scalar-index workaround. Host
  patterns distinguish reversal from copying, which the original sum-only
  check could not do. BYTE-count routines are not invoked for 256/257-element
  external buffers. These are disjoint copies, not `memmove` semantics.
- `fastcalltest` maps C's signed words to INT, with host-selected products and
  sums constrained to representable INT values. Its original word/multiply
  expression is outside the current byte-only leaf-inliner subset; separate
  private byte leaves exercise that path without requiring inlining. Byte
  arithmetic wraps explicitly, and the host oracle checks exactly-once counts
  without asserting C argument order. Word inputs are at `$06E0..$06E5`;
  byte inputs occupy `$06F0/$06F2/$06F4`. All other host-page gaps stay poisoned.
- `structarraycopy` retains `Point=[INT x,y]`, the two eight-element record
  arrays, conditional calls, whole-record assignment and the original signed
  sum. `BmLine` increments an observable counter instead of being empty.
  An additional seven-byte `Mixed` record contains BYTE, INT and CARD fields;
  two pointer-based copy loops exercise runtime lengths and addresses without
  replacing the original direct-array loop. Every destination byte starts
  different from its source. Rust independently derives all record bytes and
  call counts, including unchanged unused records, source buffers, guards and
  the host configuration page. These are disjoint copies, not overlap tests.
- `structmembertest` retains both original record layouts and their 100-iteration
  loops, including local declarations, typed pointer arguments and direct
  `a.x(i)=a.y(i)` / `v(i).x=v(i).y` assignments. Local objects use fixed host
  RAM for complete observation; inline members are never replaced by pointer
  fields or separate arrays. The added `Wide` record has two 257-element inline
  INT arrays; `Marked` has a BYTE tag and three INT fields. Rust derives every
  expected word independently and checks untouched `y`, `z`, tags, unused `x`
  elements and guards. The original scalar failure counter is supplementary,
  not the oracle. No comparison-as-value extension is used in this fixture.
- `testinterval` omits Oscar64's signed-byte section. Rust derives interval
  counts by intersecting bounds in a wider signed type; endpoint probes check
  each truth value independently. `mixsigncmptest` uses Action!'s
  INT precedence for mixed INT/BYTE comparisons, so BYTE values stay unsigned
  when widened. Both operand orders have independently calculated results.
  Its original counts are `7893/13017`, `7899/13011`, `13017/7893`, and
  `13011/7899`; the extended branch table uses `$A5/$5A`, while the value table
  requires `1/0`. A runtime flag chooses original or extended branch checks,
  with full-page guards and unchanged inputs. Numeric comparison expressions
  live in separate `_values` companions because the original cartridge rejects
  them. Compatibility uses explicit IF results, not a silent backend workaround;
  both forms retain the same independent truth/count oracles. Unused tables and
  result slots remain poisoned and checked.
- Oscar64's compile-time `#for` in `maskcheck` is expanded to literal-mask
  branches. Automatic C zero-initialized arrays become explicit initialization
  on each Action! call.
- Original results begin at `$0600`, host inputs occupy fixture-declared cells
  near the end of the `$0600` page, and completion is
  `$A5` at `$06FF`. All result/configuration bytes start poisoned. Fixed test
  buffers are surrounded by checked guards; inputs/guards cannot overlap a
  loaded object segment. Each case gets a fresh VM.
- New arithmetic tables use guarded `$6001/$6403/$6805/$6C07` backing (signed
  multiplication) or `$7001/$7403/$7805/$7C07` (shift composition). Host offset
  inputs exercise different destination indexes. The reverse-copy test uses
  five disjoint external buffers plus three independently guarded original
  buffers. Mixed-comparison tables use `$5001/$6003`, with guards checked
  throughout `$4F00..$6CFF`. All new host input words/tables must remain unchanged.
- Fixtures finish in `DO OD`. The harness requires completion within a bounded
  instruction budget, then checks memory. These budgets are watchdogs, not
  benchmark timings. Optimizer selection counts are deliberately not asserted.

## Running

From the repository root:

```sh
cd tools/vm-runtime-tests
cargo test --locked --test oscar64_conformance
```

Run the fixed-point Mandelbrot port separately with:

```sh
cargo test --locked --test oscar64_mandelbrot
```

Run just the MIR6502 word-vector regressions:

```sh
cargo test --locked --test oscar64_conformance oscar64_mir_word_vector
```

Run the new categories separately:

```sh
cargo test --locked --test oscar64_conformance oscar64_shift_add_sub
cargo test --locked --test oscar64_conformance oscar64_signed_multiply
cargo test --locked --test oscar64_conformance oscar64_mir_reverse
cargo test --locked --test oscar64_conformance oscar64_classic_reverse
cargo test --locked --test oscar64_conformance oscar64_nested_calls
cargo test --locked --test oscar64_conformance oscar64_signed_intervals
cargo test --locked --test oscar64_conformance oscar64_mixed_signed_comparison
cargo test --locked --test oscar64_conformance oscar64_record_array_copies
cargo test --locked --test oscar64_conformance oscar64_record_members
```

The existing VM CI job runs `cargo test --locked`, so the active tests need no
new runner, dependency, or workflow. Root `cargo test` does not execute this
isolated crate; it does include the new fixtures in the broad NIR corpus sweep.

## Compiler regressions

### OSCAR-RECORD-ARRAY-BACKING / COPY / COMPARE — fixed

The member port exposed record-array decay being mistaken for single-record
addressing, lost local fixed backing in classic and NIR, MIR allocation using
element width for a descriptor, a scratch-conflicting MIR copy fusion, and
classic comparisons recomputing field addresses across live accumulator state.
All 120 modern cases now pass with unchanged source loops and memory oracles.
The general fixes and focused cross-profile coverage are described in the
[record-array regression note](../../../docs/bugs/RECORD_ARRAY_PORTING_GAPS.md).

### OSCAR-CLASSIC-INDIRECT-SCALAR-COPY — fixed

The original `structarraycopy` checksum exposed a word load that overwrote its
own `$AE/$AF` source pointer between byte reads. Both classic profiles returned
`$00FC` instead of zero despite correct record copies; MIR6502 passed. Shared
scalar-copy staging now protects overlapping pointer destinations, including
byte-to-word loads and fixed absolute zero-page aliases. All 198 port cases
pass without changing their expressions or oracles. See the
[diagnosis and focused coverage](../../../docs/bugs/CLASSIC_INDIRECT_SCALAR_COPY_POINTER_BUG.md).

### OSCAR-COMPARISON-VALUE — modern extension implemented

The original cartridge rejects comparison values. Modern classic and MIR6502
now support BYTE 0/1 results using existing comparison/branch machinery, while
Compatibility rejects numeric uses during semantic analysis. All branch/count
checks remain enabled across three modes; value checks use both modern backends.
Execution exposed signed subtraction overflow in classic, now repaired through
the shared N xor V branch path. See the
[profile boundary and regression coverage](../../../docs/bugs/COMPARISON_VALUE_MATERIALIZATION_GAPS.md).

### OSCAR-COMPAT-NESTED-CALL — fixed

Compatibility formerly failed `fastcalltest` under both runtimes. The second
`C2` call overwrote the first result in `$A0/$A1`, so the original expression
computed `5+4*4-13=8`, not zero. Protective stack staging is now shared across
classic profiles, and call detection traverses casts. Each argument is staged
at the ABI base before being pushed, avoiding an overlapping word-result copy
for mixed-width signatures. All 1,536 cases now pass with the original nested
expressions and independent oracle. Focused compiler tests check both profiles,
BYTE/INT/CARD and mixed-width arguments, casts, repeated calls, counts, guards,
and stack balance. See the
[diagnosis and repair](../../../docs/bugs/CLASSIC_NESTED_CALL_ARGUMENT_BUG.md).

### OSCAR-CLASSIC-COMPUTED-INDEX — fixed

Both classic modes and both runtimes formerly failed `arraytest`'s original reverse-copy
shape. In `d(i)=s(n-i-1)`, the source base is loaded into `$AC/$AD`, then the
inner subtraction materializes `n-i` into those same bytes. Address scaling
therefore added the final index to a corrupted base. This affected BYTE and INT
indexes and is distinct from the already-fixed scale-carry loss below.

The fallback now uses the existing materialization predicate to preserve the
captured base on the stack around computed-index evaluation. All 180
reverse-copy cases pass, including the original-size checks; their source
expressions and memory oracles are unchanged. Focused compiler tests cover
all four pointer scratch pairs, byte/word element sizes, BYTE/INT/CARD indexes,
nested addition/subtraction, load/store/copy consumers, and base-before-index
evaluation across an effectful call. See the
[diagnosis and reproduction commands](../../../docs/bugs/CLASSIC_COMPUTED_POINTER_INDEX_BUG.md).

### OSCAR-CLASSIC-WORD-INDEX — fixed

Both Compatibility and Optimized, with both runtimes, previously lost the scale
carry when an INT-valued pointer index crossed 127/128. For example, the copy fixture
with 129 elements left `$5503` unchanged, and the increment fixture updated
element zero twice. The offset fixture with base `$5000` and argument 123 wrote
3 at `$5000` instead of writing the third value at `$5100`.

The shared fallback in `src/codegen/expr.rs::pointer_index_slot_with_addr`
formerly emitted `ASL; CLC; ADC low; ... ROL; ADC high`: `CLC` discarded the
scale carry, and the later ROL consumed the low-address-add carry instead.
It now uses the existing array word-index schedule: PHP saves the scale carry,
the low-address-add carry enters ROL, and PLP restores the scale carry for the
final high-byte ADC. This changes only the general word-pointer fallback;
byte-element and proven byte-index fast paths remain unchanged.

All three `oscar64_classic_*` boundary tests are enabled with their original
memory oracles. Focused compiler tests additionally cover all four scale/base
carry combinations, both scratch pointer pairs, unchanged X/Y and stack balance,
INT/CARD indexes, `i+3`, and load/store/increment/copy consumers. Address-only
checks cover full 16-bit wrapping without dereferencing wrapped addresses.

### OSCAR-MIR-SELF-INDEX-STORE — fixed

MIR6502 previously failed the original 100-element copy and increment checks in both
runtimes, even with the extra transfer length set to zero. This is not merely
an extended boundary-case failure. Both residuals are `$ED0D` (-4851), not zero:
the copy sum is 99 rather than 4950, and the increment sum is 199 rather than
5050.

The initialization shape is `INT i; ... words(i)=i`. The post-home
`word-array-store-value-staging` rewrite removed both staged value stores but
kept an indexed-address operation reading those same spill bytes. Subsequent
coloring of the already-invalid MIR made both index lanes share one spill;
coloring was not the original defect.

The shared structural-plan builder and rewrite driver now check surviving
replacement home reads against disappearing definitions, including address
bases, indexes, and pointer-pair bytes. Unsafe staging elimination is rejected;
independent-index staging elimination remains available. Routine-wide deadness
still protects uses after the window and on backedges. The post-home verifier
also rejects compiler-private bytes that may be read without a definition on
some entry path, independently of rewrite selection.

Both `oscar64_mir_word_vector_*` tests retain the original zero-residual checks
and extended pointer-buffer checks. To inspect the emitted code:

```sh
cargo run --bin actionc -- --mode mir6502 --runtime standalone \
  --listing /tmp/oscar-copy.asm -o /tmp/oscar-copy.xex \
  fixtures/runtime/oscar64/copyintvec.act
```

Run that command from the repository root. Neither fixture loops nor expected
results were changed to enable these tests.
