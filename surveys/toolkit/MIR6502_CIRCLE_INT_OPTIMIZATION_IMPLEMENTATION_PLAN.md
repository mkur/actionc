# MIR6502 CIRCLE INT Optimization Implementation Plan

Status: in progress; Slices 0 through 6 complete

Date: 2026-07-26

Planning baseline: `94b8f82`

Primary source:
`corpora/toolkit/original/extracted/CIRCLE.DM1`

Shared library:
`corpora/toolkit/original/extracted/CIRCLE.ACT`

Secondary source:
`corpora/toolkit/original/extracted/CIRCLE.DM2`

Scope: modern profile, MIR6502 backend

## Objective

Improve MIR6502's ordinary signed `INT` arithmetic and comparison strategy,
using CIRCLE1 as the primary workload. The implementation must be general:
no rewrite may inspect the `Circle` or `Abs` routine names, Toolkit paths, or
source spelling.

The primary target is the repeated shape:

```text
ordinary memory word/byte loads
    -> signed word add or subtract
    -> A/X or Y call argument
```

The secondary targets are word-arithmetic chains, signed comparisons, and
known-callee result preservation. Together these account for essentially the
entire current CIRCLE1 gap.

## Baseline Artifacts

Generate the common comparison artifacts with:

```sh
tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/circle1-listing-audit-20260726 \
  --no-diffs \
  corpora/toolkit/original/extracted/CIRCLE.DM1

cargo run --quiet --bin actionc-listing-quality -- \
  target/circle1-listing-audit-20260726/circle/classic.listing \
  > target/circle1-listing-audit-20260726/circle/classic.quality

cargo run --quiet --bin actionc-listing-quality -- \
  target/circle1-listing-audit-20260726/circle/mir6502.listing \
  > target/circle1-listing-audit-20260726/circle/mir6502.quality

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern \
    --backend mir6502 \
    --emit-load \
    corpora/toolkit/original/extracted/CIRCLE.DM1 \
    > /dev/null \
    2> target/circle1-listing-audit-20260726/circle/mir6502.peepholes
```

The baseline load-file hashes are:

| Backend | SHA-256 |
| --- | --- |
| modern/classic | `4b768f2672959cbca58d269ded7fe6fdd812416b094edfbe269dc84d7cbce62d` |
| modern/MIR6502 | `c348c22cbdf52886a746b1cb8cb0b5916bea8139868a298dd9dd7fbbafe879d5` |

## Baseline Measurements

| Metric | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| XEX bytes | 625 | 778 | +153 |
| Recognized instruction bytes | 591 | 738 | +147 |
| Data and inline machine bytes | 22 | 28 | +6 |
| Recognized instructions | 256 | 327 | +71 |
| `LDA` | 73 | 108 | +35 |
| `STA` | 48 | 74 | +26 |
| `LDA` + `STA` instructions | 121 | 182 | +61 |
| `LDA` + `STA` instruction share | 47.3% | 55.7% | +8.4 points |
| RAM spill cells | 0 | 4 | +4 |
| RAM spill accesses | 0 | 9 | +9 |

The six extra data bytes are the two-byte `Abs.n` parameter home and four
one-byte result/compare spills. CIRCLE1 has no general spill-pressure problem;
all four spills belong to the two `Abs` results feeding one signed comparison.

### Routine concentration

The routine figures below are recognized instruction bytes. Inline SARGS data
is excluded consistently.

| Routine | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `Abs` | 21 | 45 | +24 |
| `Circle` | 496 | 620 | +124 |
| `CircleDemo` | 74 | 73 | -1 |
| **Total** | **591** | **738** | **+147** |

`CircleDemo` is already competitive. Work should remain focused on `Circle`
and `Abs`.

### `Circle` body decomposition

The following byte ranges include the common three-byte SARGS descriptor in
the initialization region. This makes the regional totals reconcile exactly
with the routine ranges even though listing-quality classifies those three
bytes as data.

| Region | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| Entry and initialization | 38 | 40 | +2 |
| `Phiy`/`Phixy` arithmetic chains | 94 | 108 | +14 |
| Eight `Plot` argument clusters | 256 | 328 | +72 |
| Copies, two `Abs` calls, comparisons, loop edge | 111 | 147 | +36 |
| **Routine range** | **499** | **623** | **+124** |

The eight `Plot` clusters are the highest-value target. Each MIR6502 cluster
is 41 bytes, versus 32 bytes in classic. The current direct call-expression
selector has already removed general temp homes, but it still stages every
ordinary-memory right operand through `$A0` or `$A1`.

For example, a byte result destined for Y currently uses:

```text
LDA y1
STA $A0
LDA y
CLC
ADC $A0
TAY
```

The ordinary-memory operand can instead remain an operand:

```text
LDA y
CLC
ADC y1
TAY
```

The same issue appears in both lanes of the word result destined for A/X.
There are 24 removable right-operand staging lanes:

- eight low-byte Y expressions;
- eight low-byte A expressions;
- eight high-byte X expressions.

At four bytes per lane, the exact gross opportunity is 96 bytes. The direct
form would make each `Plot` cluster 29 bytes, three bytes smaller than classic,
because MIR6502 already places the high result directly in X.

## IR Assessment

The optimized NIR is not the limiting factor. It contains:

- explicit typed `Int` add/sub operations;
- one-use arithmetic temps directly consumed by all eight `Plot` calls;
- the complete `Phi + y1 + y1 + 1` and `Phiy - x1 - x1 + 1` chains;
- two direct `Abs` calls feeding one signed comparison;
- a direct signed loop comparison.

The optimized program has 109 operations: 53 loads, 12 stores, 26 binary
operations, four compares, and 13 calls. It removes the redundant `+0` before
the `Abs` comparison and forwards `Phiy` into the following subtraction chain.

Although NIR reports all 11 scalar homes as promotable, wholesale scalar
promotion is not the first response here. The eight opaque OS `Plot` calls
force target register values to be reconstructed at each call, and MIR6502
already sees the required one-use arithmetic expressions. The observed loss
happens during target operand and destination selection.

No NIR change is planned. If a slice discovers a missing typed or effect fact,
stop and amend this note instead of making MIR6502 consult SemIR.

## Current MIR6502 Limitations

### 1. Direct call-argument selection still stages ordinary memory

`call-arg-expr-consumer` selects all eight `Plot` calls, so this is not a
failure to recognize the source expression. The loss is in
`materialize_binary_rhs_to_fixed_scratch_avoiding`, which unconditionally
copies every `MirValue::PointerCell` right operand into fixed scratch.

That fallback is needed for aliasing return slots, volatile memory, indirect
values, and operand shapes that can be overwritten while the other call lanes
are prepared. It is unnecessary for stable direct `Local`, `Param`, `Global`,
or `Static` storage that is proven not to overlap the destination or reserved
call staging homes.

### 2. Word chains materialize both their input and final transient

The `Phiy` chain first copies `Phi` into `$E0/$E1`, reloads it, and only then
performs the first addition. At the other end it writes the `+1` result back
to `$E0/$E1` and then copies it to `Phiy`.

Classic starts the first addition directly from `Phi` and places the final
addition directly in `Phiy`. Those two destination choices explain the full
14-byte arithmetic-chain gap. The intermediate `$E0/$E1` accumulator is
otherwise appropriate; it is only two physical ZP bytes despite representing
12 residual lanes.

### 3. Signed zero tests use the general overflow CFG

`Abs` implements `n < 0` with a full word subtraction from zero followed by
V/N dispatch. A signed word's relation to zero has a cheaper exact form:

- `< 0`: test the high lane's N flag;
- `>= 0`: test the high lane's inverse N flag;
- `> 0` and `<= 0`: combine the sign test with a word-zero test.

The existing `signed-return-word-zero-compare-branch` selector only handles a
known call result in the public return slots. It does not cover an ordinary
parameter or memory value.

### 4. Temp-backed signed relations fall into sign dispatch

The comparison `Abs(Phixy) < Abs(Phiy)` retains four RAM spill lanes. Because
the compare operands are still temp-backed when comparison lowering runs,
MIR6502 chooses three-way sign dispatch plus an unsigned word comparison.

Once operands have stable homes, a direct signed relation can use subtraction
and the 6502 overflow rule:

```text
SEC
LDA left.lo
SBC right.lo
LDA left.hi
SBC right.hi
BVC no_overflow
EOR #$80
no_overflow:
BMI less
```

The conditional `EOR #$80` converts N to `N xor V`. Predicate reversal and
branch polarity cover all four signed relational operators.

The final `y1 > x1` relation already avoids spills but still uses the larger
V/N branch scaffold. It should use the same direct selector.

### 5. The newest call result receives an unnecessary home

After the second known `Abs` call, its result is still resident in the public
`$A0/$A1` return slots. MIR6502 nevertheless stores both lanes in RAM before
the comparison.

Only the first result must survive the second call. It can use the routine's
two private virtual-ZP bytes after the arithmetic chain is dead, provided the
known-callee effect summary proves that the second call preserves those
locations. The second result should remain an alias of its return slots until
the compare consumes it.

`$A0/$A1` are result slots in this case. This plan does not treat `$A0-$A2` as
general parameter-shadow homes.

### 6. `Abs` keeps a separate static parameter home

`Abs` is a leaf. Its first word parameter is read-only, not address-taken, and
arrives in A/X. Its result leaves in `$A0/$A1`. A leaf-only home placement can
capture the input directly in the future result slots:

```text
STA $A0
STX $A1
TXA
BPL return
...
```

This removes the two-byte `Abs.n` static home and brings the routine close to
the classic 21-byte implementation without changing the external ABI.

## Decisions Fixed by This Plan

1. Signed `INT` add/sub remains modulo-16-bit arithmetic. No algebraic rewrite
   may assume absence of overflow.
2. NIR and SemIR remain target-independent. A/X/Y, flags, fixed scratch,
   return slots, and operand addressing remain MIR6502 concerns.
3. Calls, machine blocks, pointer dereferences, and absolute/hardware memory
   remain barriers unless structured effects prove a narrower contract.
4. Direct memory operands may bypass scratch only when the read is ordinary,
   stable, and non-overlapping with every destination or reserved staging
   home written before that read.
5. Subtraction operand order is never reversed. Addition may commute only
   when both reads are proven ordinary and reordering is unobservable.
6. Every compare rewrite preserves signed boundary behavior for `$8000`,
   `$FFFF`, `$0000`, `$0001`, and `$7FFF`.
7. Public result-slot residency ends at a call or write whose structured
   effects may clobber that slot.
8. Private ZP reuse requires routine liveness and known-callee memory effects;
   it is not inferred merely because two textual operations are separated.
9. Existing shared pre-home/post-home rewrite drivers, use/definition proofs,
   liveness, and effect summaries are mandatory. Do not add an isolated
   peephole pipeline.
10. Classic is a strategy comparison, not a correctness oracle.
11. SARGS redesign, range-driven high-lane removal, interprocedural inlining,
    and whole-routine NIR scalar promotion are out of scope.
12. Each behavior-changing slice is committed separately after its focused
    and repository-wide gates pass.

## Slice 0: Repair Coverage and Add an INT Runtime Oracle

### Problem

`tests/mir6502_circle_quality.rs` points at:

```text
samples/toolkit/original/extracted/CIRCLE.ACT
```

That path no longer exists. The test silently returns when the fixture is
missing, so its assertions currently exercise nothing. It also positively
asserts the `$A0/$A1` staging sequence that Slice 1 must remove.

### Implementation

- Point the test at `corpora/toolkit/original/extracted/CIRCLE.ACT`.
- Treat a missing repository fixture as a test failure, not an optional skip.
- Keep the eight-`Plot` selector and emission checks, but stop making the
  inefficient scratch sequence part of the contract.
- Record the current CIRCLE1 size, listing-quality, selector, and spill
  baseline in the test/note workflow.
- Add a VM fixture for the relevant general shapes:
  - signed word add/sub from two ordinary memory operands into A/X;
  - low-byte result into Y from two `INT` operands;
  - the `Phiy`/`Phixy` arithmetic chains;
  - signed zero tests and arbitrary signed word relations;
  - two known leaf-call results feeding a signed comparison.
- Execute the fixture under modern/classic and modern/MIR6502 and compare
  fixed memory result buffers, including wrap and signed-boundary cases.
- Keep separate expectations for the three general signed-overflow slots:
  classic's legacy N-only branch misclassifies `$8000` versus `$7FFF`, while
  MIR6502 must satisfy the signed language result. The shared arithmetic and
  non-overflow slots must agree.
- Add the ignored compatibility-test entry and runtime README command.

### Acceptance

- The CIRCLE quality test demonstrably compiles the real Toolkit source.
- The VM oracle passes both backends before optimization, with the documented
  classic signed-overflow divergence isolated to three slots.
- Removing or corrupting any intended boundary result makes the oracle fail.
- CIRCLE1 remains 778 bytes at this coverage-only slice.

Suggested commit:

```text
mir6502: add CIRCLE INT arithmetic runtime coverage
```

## Slice 1: Use Stable Direct Memory as Binary Call Operands

This is the highest-value slice.

### Implementation

- Split the current unconditional PointerCell staging rule into:
  - a direct ordinary-memory operand path;
  - the existing conservative scratch fallback.
- Initially allow only direct `Local`, `Param`, `Global`, and `Static` lanes
  whose layout/effects mark them ordinary and nonvolatile.
- Reject:
  - absolute or hardware memory;
  - indirect/indexed reads;
  - public return/fixed-argument slots that overlap a pending destination;
  - virtual/fixed scratch that may be overwritten while another lane is
    prepared;
  - any operand whose required read order is not proven safe.
- Apply the predicate consistently in:
  - byte add/sub expressions materialized to a register;
  - word add/sub expressions materialized to A/X;
  - Y:`$A3` word expressions where applicable;
  - canonical Action staging only when it has the same alias guarantees.
- Preserve the low result in `$A0` while computing the high A/X lane; only the
  right operands bypass scratch.
- Record candidate, selected, and blocked-reason telemetry by lane.
- Replace the old positive staging assertions with exact direct-operand
  assertions and absence checks.

### Expected effect

CIRCLE1 has 24 applicable lanes. Removing one load/store staging pair while
changing a two-byte ZP arithmetic operand into a three-byte ordinary-memory
operand saves four bytes per lane:

```text
24 * 4 = 96 bytes
```

The projected CIRCLE1 size is 682 bytes. The eight `Plot` clusters should fall
from 328 to 232 bytes.

### Acceptance

- All 24 CIRCLE1 lanes select the direct path.
- No `$A0/$A1` right-operand staging remains in the eight `Plot` clusters.
- Add/sub result bytes match classic and the VM oracle for wrap boundaries.
- Alias, return-slot overlap, absolute-memory, indirect, and hardware negative
  tests retain staging.
- CIRCLE2, direct Action word-argument, TN, SORTDM1, and ALLOCATE gates pass.

### Implemented result

The shared layout's deferred-direct-read predicate now governs binary call
operand selection. Direct `Local`, `Param`, `Static`, and ordinary-backed
`Global` lanes remain memory operands; absolute, hardware-backed global,
virtual-ZP, fixed-ZP, and overlapping staging lanes retain the scratch
fallback. The selector records candidate, selected, overlap-blocked, and
nonordinary-blocked counts per lane.

CIRCLE1 selects exactly 24 of 24 candidates and falls from 778 to 682 bytes,
matching the projection exactly. CIRCLE2 falls from 1058 to 962 bytes. The
direct Action word-argument VM gate, the CIRCLE INT VM gate, TN stability, and
the focused ALLOCATE/SORTDM1 compile gates pass. ALLOCATE and SORTDM1 remain
byte-identical at 876 and 3289 bytes; TN emits 9941 bytes.

Suggested commit:

```text
mir6502: use direct memory operands for arithmetic call args
```

## Slice 2: Place Word Arithmetic Chains Destructively

### Implementation

- Recognize a same-block one-use word chain consisting of stable loads and
  add/sub producers ending in a direct store.
- Require exact reaching definitions, lane coupling, unobserved intermediate
  flags, and no intervening call, machine block, indirect access, or volatile
  memory.
- Initialize the chosen transient pair with the first binary operation instead
  of copying the left input into the pair and reloading it.
- Place the final binary result directly in the durable store destination when
  doing so is cheaper than defining the transient pair and copying it.
- Keep the intermediate two-byte `$E0/$E1` accumulator; do not reassociate the
  arithmetic or split its carry chains.
- Add telemetry for first-stage destructive placement and final-store
  placement.

### Expected effect

- First-stage destructive placement: approximately 8 bytes.
- Final-store placement: approximately 6 bytes.
- Total CIRCLE1 arithmetic-chain saving: 14 bytes.

### Acceptance

- The `Phiy`/`Phixy` region falls from 108 bytes to approximately 94.
- The exact modulo-16-bit result is unchanged for positive, negative, carry,
  borrow, `$7FFF`, and `$8000` cases.
- Chains with multiple uses, observed carry/flags, volatile operands, or
  barriers are rejected.

### Implemented result

Two post-home transactional placements use the shared home-definition
liveness proof:

- the first word operation reads its stable direct operands into A and writes
  the transient pair only after arithmetic;
- the last operation writes the durable destination directly, then the next
  chain reloads that durable value instead of an obsolete transient copy.

Both rewrites reject scratch aliases, nonordinary deferred reads, and
observable destination-store reordering. CIRCLE1 falls from 682 to 668 bytes
and CIRCLE2 from 962 to 948 bytes, closing the expected 14-byte chain gap.
The same general rewrite reduces SORTDM1 from 3289 to 3277 bytes; its VM gate
passes. ALLOCATE remains 876 bytes, TN remains 9941 bytes, and the CIRCLE INT
VM oracle passes.

Suggested commit:

```text
mir6502: place word arithmetic chain endpoints directly
```

## Slice 3: Generalize Signed Word Relations Against Zero

### Implementation

- Generalize the existing signed return-word zero branch selection to stable
  register, direct-memory, and proven-home operands.
- Normalize zero-on-left relations by reversing the predicate.
- Select high-lane N directly for `< 0` and `>= 0`.
- Select sign plus word-zero logic for `> 0` and `<= 0`.
- When the source is still in the incoming A/X pair, use X's high lane without
  forcing a whole-word subtraction or a temp home.
- Use the shared compare/branch rewrite proof and preserve only the lanes
  required by the selected predicate.
- Keep the existing public-return-slot specialization as one source form of
  the generalized selector.

### Expected effect

The `Abs` entry comparison should lose its `SEC/SBC/SBC/BVS` scaffold and use
the sign of the high input lane. This should save approximately 8-12 bytes
before parameter-home coalescing.

### Acceptance

- `Abs` uses a high-lane sign branch, not general signed subtraction.
- All four signed relations work with zero on either side.
- Boundary tests cover `$8000`, `$FFFF`, `$0000`, `$0001`, and `$7FFF`.
- A low lane is not loaded for `< 0` or `>= 0`.

### Implemented result

The shared pre-home compare/branch proof now accepts signed word relations
against zero from ordinary direct homes, stable physical or virtual register
lanes, and public return slots. The return-slot call is retained as a prefix
operation rather than remaining a separate selector. Zero on the left is
normalized by reversing the predicate.

`< 0` and `>= 0` materialize only the high lane and branch on N. `> 0` and
`<= 0` retain the existing exact sign-plus-word-zero CFG. Global and absolute
memory are deliberately excluded because this pre-home selector has no layout
proof that such reads are ordinary and nonvolatile.

The CIRCLE `Abs` entry now emits `TXA` followed by a direct N branch. Its low
parameter lane is not loaded. CIRCLE1 falls from 668 to 656 bytes and CIRCLE2
from 948 to 936 bytes, saving 12 bytes in each program. The CIRCLE INT runtime
oracle and the dedicated eight-relation return-word oracle pass all boundary
cases.

Suggested commit:

```text
mir6502: select direct signed word zero branches
```

## Slice 4: Select Direct Signed Word Relations After Home Placement

### Implementation

- Add a post-home signed-relation selector for stable physical byte lanes.
- Normalize `>`, `<=`, and reversed operands to one subtraction/polarity
  implementation without changing subtraction order incorrectly.
- Emit low/high subtraction followed by V correction and one N-based branch.
- Use block-layout cost information to choose the fall-through edge and avoid
  branch-over-JMP scaffolding where relative reach permits.
- Require that the compare is the unique consumer or that removed definitions
  are proven dead.
- Treat every source load as ordered and reject volatile/absolute/indirect
  combinations unless an existing specialized selector already proves them.
- Keep equality/inequality on their existing word-zero/equality paths.

### Expected effect

This targets both:

```text
Abs(Phixy) < Abs(Phiy)
y1 > x1
```

The expected gross saving is 10-20 bytes, depending on final block layout.
The main structural win is replacing temp-backed sign dispatch with one
subtraction chain.

### Acceptance

- Neither CIRCLE comparison uses `cmp_i16_left_sign`,
  `cmp_i16_right_sign_pos`, or `cmp_i16_right_sign_neg`.
- Signed overflow cases agree with a reference predicate for a cross-product
  of negative, zero, positive, minimum, and maximum values.
- V is consumed before any instruction that changes it.
- Final branches use shared liveness and machine-state proofs.

### Implemented result

A post-home CFG transaction now recognizes the exact generated signed
sign-dispatch and subtraction/overflow shapes after logical lanes have physical
homes. It accepts only constants and stable compiler-owned or ordinary direct
memory, requires private helper-block predecessors, proves A and flags dead at
the semantic exits with the shared post-home analysis, and accepts an adjacent
entry copy only as an exact source/home alias proof.

Both shapes lower to a low/high `SBC` chain, branch on V, conditionally apply
`EOR #$80`, and converge on one N branch. The old sign tree and the duplicate
V-set/V-clear N branches disappear. Cost-aware block layout places the
correction path without introducing branch-over-JMP scaffolding.

CIRCLE1 falls from 656 to 618 bytes, 7 bytes below modern/classic. CIRCLE2
falls from 936 to 898 bytes, leaving a 9-byte gap. Each program saves 38 bytes
in this slice. Removing the sign tree also makes the newest `Abs` result's four
spill stores dead, so the post-home cleanup consumes part of the Slice 5
opportunity automatically.

The CIRCLE quality contract now rejects all old signed scaffolding labels and
caps the shared CIRCLE library at 544 bytes. A dedicated VM matrix verifies
`<`, `<=`, `>`, and `>=` over the 25 pairs formed by `$8000`, `$FFFF`, `$0000`,
`$0001`, and `$7FFF`.

Suggested commit:

```text
mir6502: lower signed word relations through direct flags
```

## Slice 5: Preserve Only the Earlier Known-Call Result

### Implementation

- Extend known-callee result-slot alias tracking through an immediately
  consuming compare:
  - the latest result remains in its public return slots;
  - only an earlier result crossing a later call receives a private home.
- Use the known-callee memory-effect summary to prove that the later call does
  not clobber the selected private home.
- Let the home planner reuse a dead two-byte virtual-ZP pair from earlier in
  the routine. Adjust profitability only for this proven shorter form; do not
  reserve global ZP unconditionally.
- If the effect or liveness proof fails, retain the current RAM homes.
- Record result-slot aliases, earlier-result preservation homes, ZP reuse, and
  blocked reasons.

### Expected effect

The second `Abs` result should lose both RAM homes. The first should use the
available private ZP pair after the arithmetic chain dies. This can remove all
four spill data bytes and replace or remove the nine absolute spill accesses,
for approximately 12-20 total bytes.

### Acceptance

- CIRCLE1 has zero RAM spill cells and zero RAM spill accesses.
- The first result survives the second call under a structured proof.
- The second result is consumed from `$A0/$A1`, not copied merely to create a
  temp identity.
- Unknown calls and callees that may touch the private home block the rewrite.

### Implemented result

The post-home storage workflow now assigns block-local zero-page lanes before
attempting cross-call result preservation. It recognizes only word results
copied from `$A0/$A1` immediately after a known routine call, proves that the
candidate pair does not interfere with an existing private zero-page pair,
and requires every crossed call to preserve the pair's resolved physical
addresses. Unknown calls, runtime helpers, machine blocks, opaque writes, and
callees that use either byte reject the placement.

Known-callee write summaries now resolve allocated virtual zero-page writes to
their physical addresses. Unallocated virtual writes conservatively block
fixed-pair preservation, so the new proof cannot mistake a callee-private
logical slot for a caller-private physical byte.

In CIRCLE, the earlier `Abs` result reuses `Circle`'s dead `$E0/$E1` arithmetic
pair. The later result remains in `$A0/$A1` through the direct comparison.
CIRCLE1 falls from 618 to 612 bytes and CIRCLE2 from 898 to 892 bytes. Both now
have zero RAM spill cells and zero spill accesses; CIRCLE1 is 13 bytes smaller
than modern/classic, while CIRCLE2 is 3 bytes larger.

Suggested commit:

```text
mir6502: retain paired call results in proven homes
```

## Slice 6: Coalesce Leaf Input and Result Storage

### Implementation

- Add a leaf-only ABI home placement for a read-only, non-address-taken first
  word parameter when:
  - it arrives in A/X;
  - the routine returns a word in `$A0/$A1`;
  - there are no calls, machine blocks, indirect escapes, or observable reads
    of the return slots before return;
  - all parameter uses are dominated by the entry capture.
- Capture A/X directly into `$A0/$A1` and rewrite internal parameter reads to
  those slots.
- Allow result definitions to overwrite the same slots only after the last
  input use.
- Keep public ABI argument and result contracts distinct in diagnostics and
  documentation.
- Fall back to the ordinary parameter home whenever the proof is incomplete.

### Expected effect

`Abs.n` loses its two data bytes and its absolute parameter traffic. Combined
with Slice 3, `Abs` should approach the classic 21-byte routine.

### Acceptance

- The `Abs.n` data label disappears.
- Positive, negative, zero, `$8000`, and `$7FFF` returns remain correct.
- Non-leaf, address-taken, machine-visible, recursive, and multiple-live-param
  cases retain ordinary homes.

### Implemented result

A leaf-only ABI placement now handles one private scalar word parameter whose
direct A/X capture dominates every use. It requires every return block to
define both `$A0` and `$A1`, rejects existing reads of those public result
slots, and permits each result lane to overwrite its input lane only after the
last parameter read on that path. Calls, runtime helpers, machine blocks,
barriers, indirect accesses, address-taking, observable Action entries, and
unsupported operation shapes retain the ordinary parameter home.

The selected routine still receives its argument in A/X and returns its value
in `$A0/$A1`; only its private storage choice is coalesced. Diagnostics and
telemetry describe those contracts separately.

`Abs.n` disappears from data. `Abs` falls from 33 to 24 instruction bytes,
leaving only a 3-byte difference from classic. CIRCLE1 falls from 612 to 601
bytes, 24 bytes below modern/classic. CIRCLE2 falls from 892 to 881 bytes,
8 bytes below modern/classic. Both retain zero RAM spill cells and accesses.
The dedicated CIRCLE INT VM oracle covers zero, positive, negative, `$7FFF`,
and `$8000` inputs.

Suggested commit:

```text
mir6502: coalesce leaf word inputs with result storage
```

## Slice 7: Final Listing Audit and Stop Decision

Regenerate:

- classic and MIR6502 CIRCLE1 listings, maps, loads, quality reports, NIR,
  pre-materialized MIR, materialized MIR, spills, and peephole telemetry;
- CIRCLE2 sizes;
- the complete modern/MIR6502 Toolkit batch.

Record:

- XEX, instruction, and data bytes;
- routine sizes;
- `LDA`, `STA`, and their byte share;
- direct RHS lane counts and blockers;
- residual lanes and physical homes;
- RAM and ZP spill counts/accesses;
- signed compare selector counts;
- branch-over-JMP and jump-to-return forms.

The gross opportunities overlap, so their estimates must not be added as a
guaranteed result. A reasonable final target is 610-640 bytes. Reaching or
beating modern/classic's current 625-byte CIRCLE1 output is plausible but is
not a correctness requirement.

Stop after the audit unless a remaining gap is:

- at least eight static bytes;
- represented by at least two general sites or a demonstrably hot loop site;
- removable with existing shared proofs and without weakening memory/effect
  conservatism.

Suggested commit:

```text
docs: audit CIRCLE INT optimization results
```

## Validation After Every Behavior-Changing Slice

Run:

```sh
cargo test circle_uses_direct_binary_call_arg_materialization
cargo test
fixtures/runtime/run-circle-int-math-vm.sh

tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/circle1-listing-audit-20260726 \
  --no-diffs \
  corpora/toolkit/original/extracted/CIRCLE.DM1

surveys/toolkit/compile-toolkit-batch.sh \
  --preset modern-mir6502 \
  CIRCLE1 CIRCLE2
```

Also compile and size TN, ALLOCATE, SORTDM1, and the direct Action word
arithmetic runtime fixture. Run the complete modern/MIR6502 Toolkit batch
before the final slice is accepted.

If any slice changes NIR lowering, optimization, verification, or printing,
also run the repository-required NIR gates:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

No NIR change is currently planned.
