# Oscar64 behavioral test ports

These eleven Action! fixtures adapt Oscar64 autotests into the existing
[isolated VM harness](../../../tools/vm-runtime-tests/README.md). They test
observable results, not a preferred instruction sequence or agreement between
backends. Expected results are calculated independently in
[`oscar64_conformance.rs`](../../../tools/vm-runtime-tests/tests/oscar64_conformance.rs).

## Provenance

Adapted on 2026-09-05 from Oscar64 by drmortalwombat and contributors:

- Upstream: <https://github.com/drmortalwombat/oscar64>.
- Inspected fork: <https://github.com/mkur/oscar64>.
- Source revision: `8deb94c4d762bab3aa60c9565412691f01021bbb`.
- Original paths: `autotest/<fixture-stem>.c`, listed below, except
  `shiftbyteaddconst.cpp`.
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

All modes use both ActionCart and Standalone runtime linking. The first eight
ports retain **258 VM cases in fourteen passing tests**. Both MIR6502
copy/increment regressions remain active with their original loops and
independent expected values.

The [second-batch plan](../../../docs/OSCAR64_TEST_PORTING_PLAN.md) has stages
1 and 2 ported. Their current results are:

| Fixture | Host cases | VM cases (three modes, two runtimes) | Result |
| --- | ---: | ---: | --- |
| `shiftbyteaddconst` | 256 | 1,536 | All pass |
| `testsigned16mul` | 33 | 198 | All pass |
| `arraytest` | 30 | 180 | All pass |

Overall: **2,172 passing VM cases in 18 active passing tests**. No test is
ignored or expects a panic. The classic reverse-copy regression was repaired
without changing fixture expressions or oracles. See
[OSCAR-CLASSIC-COMPUTED-INDEX](#oscar-classic-computed-index--fixed).

## Action! semantics and harness contract

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
- Oscar64's compile-time `#for` in `maskcheck` is expanded to literal-mask
  branches. Automatic C zero-initialized arrays become explicit initialization
  on each Action! call.
- Original results begin at `$0600`, host inputs at `$06F0`, and completion is
  `$A5` at `$06FF`. All result/configuration bytes start poisoned. Fixed test
  buffers are surrounded by checked guards; inputs/guards cannot overlap a
  loaded object segment. Each case gets a fresh VM.
- New arithmetic tables use guarded `$6001/$6403/$6805/$6C07` backing (signed
  multiplication) or `$7001/$7403/$7805/$7C07` (shift composition). Host offset
  inputs exercise different destination indexes. The reverse-copy test uses
  five disjoint external buffers plus three independently guarded original
  buffers. All new host input words/tables must remain unchanged.
- Fixtures finish in `DO OD`. The harness requires completion within a bounded
  instruction budget, then checks memory. These budgets are watchdogs, not
  benchmark timings. Optimizer selection counts are deliberately not asserted.

## Running

From the repository root:

```sh
cd tools/vm-runtime-tests
cargo test --locked --test oscar64_conformance
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
```

The existing VM CI job runs `cargo test --locked`, so the active tests need no
new runner, dependency, or workflow. Root `cargo test` does not execute this
isolated crate; it does include the new fixtures in the broad NIR corpus sweep.

## Compiler regressions

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
