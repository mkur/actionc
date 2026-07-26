# MIR6502 SORT Optimization Implementation Plan

Status: implementation in progress (Slices 0A-4 complete)

Date: 2026-07-26

Planning baseline: `367c6d4`

Primary source:
`samples/toolkit/modern/SORT.DM1`

Secondary source:
`samples/toolkit/modern/SORT.DM2`

Scope: modern profile, MIR6502 backend

## Objective

Close the modern/MIR6502 size and load/store-traffic gap on the Toolkit SORT
demos while protecting every addressing, comparison, call-argument, and
deferred-storage change with an execution-level sorting oracle.

The work is deliberately divided into two parts:

1. optimize the sorting library and its working storage first;
2. optimize the large `Test`/`PrintF` argument-staging cluster only after the
   sorting core is covered and improved.

The classic backend is a directional target-strategy comparison. It is not a
correctness oracle and is not modified by this plan.

## Audit Baseline

The fresh SORTDM1 comparison at the planning baseline is:

| Metric | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| XEX bytes | 4,113 | 4,965 | +852 |
| Recognized instruction bytes | 3,777 | 4,468 | +691 |
| Data and inline machine bytes | 324 | 485 | +161 |
| Recognized instructions | 1,729 | 1,921 | +192 |
| `LDA` instructions | 495 | 596 | +101 |
| `STA` instructions | 445 | 511 | +66 |
| `LDA` + `STA` bytes | 2,244 | 2,850 | +606 |

The current load-file hashes are:

| Backend | SHA-256 |
| --- | --- |
| modern/classic | `9e308dd7350075da0066caf43d3f10b6649a4c47b76864c8c55e144512d6ea60` |
| modern/MIR6502 | `b2f9fa3d778e09ddcbec8f211b77406233ad8d2a88a19cc03ce9dc573f818616` |

The code-byte deficit is concentrated as follows:

| Routine | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `Test` | 1,609 | 1,985 | +376 |
| `QuickSort` | 246 | 334 | +88 |
| `SAscend` | 89 | 135 | +46 |
| `SDescend` | 98 | 133 | +35 |
| `CAscend` | 70 | 101 | +31 |
| `CDescend` | 70 | 95 | +25 |
| `AddList` | 130 | 149 | +19 |
| `Partition` | 345 | 362 | +17 |

The complete gap divides into:

- approximately 460 bytes in the sorting library, including its global and
  routine storage;
- approximately 392 bytes in the demo/display routine `Test`, including its
  transient homes.

The current output already has several positive controls:

- `Compare` is 6 instruction bytes smaller than classic;
- `Swap` is 9 instruction bytes smaller than classic;
- `SortB`, `SortC`, `SortI`, and `SortS` are byte-identical to classic at 38
  instruction bytes each;
- `Partition` is only 17 instruction bytes larger despite being the most
  involved sorting routine.

The current SORTDM2 batch sizes are 2,597 bytes for modern/classic and 3,041
bytes for modern/MIR6502, a 444-byte gap. Most library improvements below
should benefit both demos.

## Decisions Fixed by This Plan

1. Action type, array, comparison, and call meaning remain owned by SemIR and
   NIR.
2. Pointer-pair choice, scaled addressing, A/X/Y placement, flag selection,
   fixed argument homes, and deferred segment placement remain owned by
   MIR6502.
3. No optimization may inspect routine names, Toolkit symbol names, source
   paths, or literal source spelling.
4. Pre-home rewrites use the shared routine-aware rewrite driver and its
   reaching-definition, liveness, effect, and machine-state proofs.
5. CFG-changing compare rewrites invalidate and rebuild routine analyses before
   another rewrite family runs.
6. Calls, machine blocks, absolute memory, hardware memory, and arbitrary
   pointer dereferences remain barriers unless existing structured effects
   prove otherwise.
7. Read-only indexed comparisons may use two scratch pointer pairs, but neither
   pair may overwrite a value live at the rewrite boundary.
8. Call-argument evaluation order and observable memory-read order are
   preserved. Destination-aware placement may reorder only proven-pure
   register/home moves and ordinary private-storage address calculations.
9. `high+1 > low+1` must not be simplified algebraically to `high > low`
   without a no-wrap proof. The planned optimization fuses the original
   arithmetic and comparison instead.
10. Deferred storage must remain part of `runtime_high_water`, skipped ranges,
    maps, and `SET symbol=*` behavior even though it is omitted from load-file
    bytes.
11. This plan does not redesign SARGS. The final `PrintF` work is
    destination-aware caller argument placement using the existing Action ABI.
12. Every behavior-changing slice is committed independently after its focused
    tests, the SORT VM oracle, and repository-wide tests pass.

## Pipeline Placement

| Opportunity | Owner and phase |
| --- | --- |
| Deferred uninitialized non-byte global array | MIR6502 layout and emission |
| Two indexed BYTE elements feeding compare/branch | MIR6502 pre-home compare selection |
| Two scaled indexed CARD elements feeding compare/branch | MIR6502 pre-home compare selection |
| Signed word relation against zero | MIR6502 pre-home compare/branch selection |
| Word arithmetic feeding `Y:$A3` | MIR6502 pre-home call-argument selection |
| Two word-arithmetic expressions feeding compare | MIR6502 pre-home compare selection |
| Indexed BYTE values feeding fixed call homes | MIR6502 pre-home call-argument selection |
| Final branch and reload cleanup | Existing post-home rewrite workflow |

The audited optimized NIR already retains the required structure: typed indexed
loads, word arithmetic, comparisons, branches, and call arguments are explicit.
No NIR change is planned. If implementation discovers that a required typed
fact is missing, stop and amend this note before adding a target workaround or
making MIR6502 consult SemIR.

## Baseline and Measurement Protocol

The audit artifacts are regenerated under:

```text
target/sortdm1-listing-audit-20260726/sort/
```

The required files are:

```text
classic.listing
classic.load
classic.map
classic.quality
mir6502.listing
mir6502.load
mir6502.map
mir6502.quality
mir6502
mir6502.materialized
nir.optimized
```

After Slice 0A repairs the comparison helper, regenerate the common artifacts
with:

```sh
tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/sortdm1-listing-audit-20260726 \
  samples/toolkit/modern/SORT.DM1
```

After every behavior-changing slice:

1. run focused unit, fixture, and emitted-shape tests;
2. run the opt-in SORT VM oracle under modern/classic and modern/MIR6502;
3. run `cargo test`;
4. regenerate SORTDM1 listing, materialized MIR, map, quality report, and XEX;
5. record total bytes, code bytes, data bytes, `LDA`, `STA`, spill slots, spill
   accesses, selector counts, and affected routine sizes;
6. compile and size SORTDM2;
7. regenerate ALLOCATE and TN and reject unexplained growth or code-shape
   regressions;
8. compile the complete modern/MIR6502 Toolkit batch;
9. commit the coherent slice separately.

If a slice changes NIR, semantic lowering, the NIR verifier, or the NIR printer,
also run the required NIR gates:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Generated target artifacts remain uncommitted. Runtime fixtures, focused
compiler fixtures, stable snapshots, and this implementation note are
committed.

## Slice 0A: Repair the Reproducible Comparison Workflow

### Problem

`tools/compare-codegen.sh --profile modern` currently passes the selected
profile to classic invocations but omits it from the MIR6502 listing, map,
load, and source-listing invocations. MIR6502 therefore fails with
`--backend mir6502 requires --profile modern`, and the audit must be generated
manually.

### Implementation

- Pass `--profile "$profile"` to every MIR6502 compiler invocation.
- Pass the selected profile to SemIR, NIR, pre-materialized MIR, and
  materialized MIR generation so all artifacts describe the same compilation.
- Add a lightweight integration check that runs the helper on a small fixture
  with `--profile modern` and requires nonempty classic/MIR listing and load
  artifacts.
- Keep comparison-tool behavior otherwise unchanged.

### Acceptance

- One helper invocation reproduces all SORTDM1 comparison artifacts.
- Both generated load sizes match direct `actionc-emit` invocations.
- No compiler output changes in this slice.

Commit independently before the runtime oracle.

## Slice 0B: VM-Backed SORT Runtime Oracle

### Goal

Create a hard correctness gate before changing comparison schedules, pointer
pairs, call placement, or the `List` backing location.

### Fixture

- Add `fixtures/runtime/sort_runtime.act`.
- Include the maintained `samples/toolkit/modern/SORT.ACT`.
- Do not execute the screen-oriented SORTDM1 `Test` routine.
- Use controlled arrays and copy the observed results into a fixed result area
  beginning at `$0600`.
- Put fixed sentinels around source arrays where practical.
- End in a generated-code loop so the VM stops at a bounded step count.
- Compile and execute both modern/classic and modern/MIR6502.
- Compare against hard-coded expected bytes. Backend equality alone is not a
  correctness oracle.

### Required scenarios

`SortB`:

- one element;
- two elements in both orders;
- duplicates;
- already ascending;
- reverse order;
- ascending and descending results;
- values around `$00`, `$7F`, `$80`, and `$FF`.

`SortC`:

- ascending and descending;
- duplicates;
- `$0000`, `$00FF`, `$0100`, `$7FFF`, `$8000`, and `$FFFF`.

`SortI`:

- ascending and descending;
- duplicates;
- negative and positive values;
- `-32768`, `-1`, `0`, `1`, and `32767`.

`SortS`:

- ascending and descending;
- equal strings;
- prefix-related strings;
- a mixture of short and longer strings that exercises `SCompare`.

The oracle must also check:

- every result is a permutation of the input, not merely monotonic;
- array-edge sentinels are unchanged;
- repeated sorts do not corrupt the internal partition list;
- sorting an already sorted array remains correct;
- the completion signature is written only after every scenario passes.

Length zero is excluded unless the Toolkit source contract is separately
changed: its current `QuickSort` computes `len-1`.

### Host integration

- Add `fixtures/runtime/run-sort-vm.sh`.
- Add an ignored `sort_runtime_check` in `tests/compatibility.rs`.
- Document the direct command in `fixtures/runtime/README.md`.
- On failure, retain or print the result range and the VM's recent instruction
  history.

### Acceptance

- Both backends produce the exact hard-coded result bytes.
- A wrong comparison polarity, signed comparison, swap order, pointer scale,
  partition boundary, or storage overlap fails the oracle.
- Ordinary `cargo test` remains independent of the external VM.
- The opt-in compatibility suite passes.

Commit independently before optimization work.

## Slice 1: Defer Uninitialized Sized Non-Byte Global Arrays

### Finding

`CARD ARRAY List(64)` occupies 128 zero bytes in the MIR6502 load image.
Classic emits a four-byte descriptor and places the backing storage after the
saved image. This accounts for 124 bytes of the SORTDM1 data gap.

MIR6502 does not need to copy the classic descriptor strategy. It can preserve
its direct absolute backing references while placing the backing in
`DeferredData`.

### Implementation

- Carry the existing structured NIR array fact into `MirGlobal`; do not infer
  array identity from a display string or from width and size alone.
- Generalize the layout eligibility predicate:
  - ordinary mutable global;
  - sized, non-pointer-backed array;
  - uninitialized/zero-fill backing;
  - no explicit initialized bytes;
  - not absolute-backed and not an alias.
- Preserve the existing 256/257 inline threshold for BYTE arrays.
- Allow uninitialized sized CARD and INT array backing to be deferred
  regardless of that BYTE-array threshold, matching the documented Action
  non-byte storage model.
- Keep the global symbol bound to the logical element backing expected by MIR
  indexed operations.
- Include the complete backing range in `runtime_high_water`, map skipped
  ranges, and `CodegenOutput.skipped_ranges`.
- Add layout/emission tests for CARD and INT arrays, multiple modules, following
  `ProgramEndWord`, and initialized non-byte arrays that must remain emitted.

### Acceptance

- SORTDM1 no longer emits the 128 zero bytes for `List`.
- The XEX shrinks by 128 bytes unless another intentional header/segment change
  is documented.
- `List` indexing and repeated QuickSort operations pass the VM oracle.
- Initialized arrays and small inline BYTE arrays retain their current policy.

Implemented result: SORTDM1 shrank from 4,965 to 4,837 load-file bytes, exactly
128 bytes, with unchanged instruction counts. SORTDM2 is 2,913 bytes. The SORT
VM oracle and the complete modern/MIR6502 Toolkit batch pass.

Commit independently.

## Slice 2: Direct Dual-Pointer Indexed BYTE Comparisons

### Finding

`BAscend` and `BDescend` each materialize the first array element into a
temporary zero-page home, rebuild `$AC/$AD` for the second element, and then
reload both values for `CMP`. Classic keeps two element pointers and compares
the indirect operands directly.

The two routines are 22 instruction bytes larger in aggregate.

### Implementation

- Add a pre-home compare candidate for two single-use indexed BYTE loads feeding
  one compare whose only consumer is a branch.
- Reuse the existing typed indexed-address description and shared proof
  workflow.
- Assign distinct scratch pointer pairs, normally `$AE/$AF` and `$AC/$AD`.
- Materialize both addresses before the final load/compare.
- Emit the equivalent of:

  ```asm
  LDY #0
  LDA (leftPointer),Y
  CMP (rightPointer),Y
  ```

- Normalize reversible predicates only when branch polarity and unsigned
  comparison meaning are preserved.
- Reject candidates crossing calls, machine blocks, volatile/absolute accesses,
  or a scratch-pair value live at the rewrite boundary.
- Add fixtures for `<`, `>`, `<=`, `>=`, equal indexes, different arrays,
  pointer-backed arrays, and reversed operand order.

### Acceptance

- `BAscend` and `BDescend` contain no value spill between their two indexed
  loads and the comparison.
- The selector fires twice in SORTDM1.
- Both BYTE sort directions pass the VM oracle.
- No general store/copy dual-pointer behavior changes.

Implemented result: the proof-backed `dual-indexed-byte-compare` selector fires
once in `BAscend` and once in `BDescend`. Both routines now materialize
independent `$AE/$AF` and `$AC/$AD` pointers and feed the final branch directly
from `CMP (zp),Y`, without byte-value homes. The selector accepts both computed
and pointer-backed indexed operands, preserves reversed operand meaning, and
requires both selected scratch pairs to be dead after the comparison.

SORTDM1 shrank from 4,837 to 4,815 load-file bytes: 22 instruction bytes and 11
instructions were removed. `LDA` fell from 596 to 592 and `STA` from 511 to
507. SORTDM2 likewise shrank from 2,913 to 2,891 bytes. The hard-coded SORT VM
oracle passes both modern/classic and modern/MIR6502, all 20 modern/MIR6502
Toolkit programs compile, and TN plus ALLOCATE do not match the new selector.

Commit independently.

## Slice 3: Direct Dual-Pointer Scaled CARD Comparisons

### Finding

`CAscend` and `CDescend` use one scaled pointer pair at a time and preserve the
first word in RAM spills. Classic retains two scaled element pointers and
performs the low/high compare through them.

The routines are 56 instruction bytes larger in aggregate and allocate seven
spill bytes between them.

### Implementation

- Extend the Slice 2 candidate from BYTE to unsigned WORD elements.
- Build both scale-two element pointers with independent pointer pairs.
- Preserve the ASL carry in each pointer high byte.
- Reuse `Y=0/1` across the final two-pointer compare.
- Preserve the low-byte `CMP` to high-byte `SBC` carry chain.
- Select branch polarity through the existing unsigned word compare rules.
- Do not materialize either word into ordinary RAM or a virtual spill.
- Cover index `$007F/$0080`, page crossings, identical indexes, and reversed
  relations in focused emitted-shape and VM tests.

### Acceptance

- `CAscend` and `CDescend` have no word-value spill slots.
- The final compare uses both indirect pointer pairs.
- The selector fires twice in SORTDM1.
- CARD boundary cases pass the VM oracle.

Implemented result: `dual-indexed-word-compare` generalizes the Slice 2
proof-backed selector to unsigned CARD elements. It builds both scale-two
element pointers in independent scratch pairs, emits a low-byte `CMP` followed
by a carry-linked high-byte `SBC`, and branches directly on the resulting
unsigned flags. The selector handles computed and pointer-backed arrays,
normalizes reversed relations, rejects signed comparisons and offsets whose
high byte would wrap, and requires both scratch pairs to be dead after the
comparison.

The selector fires once in `CAscend` and once in `CDescend`. SORTDM1 shrank
from 4,815 to 4,754 load-file bytes. Recognized code fell from 4,446 to 4,392
bytes, data from 357 to 350 bytes, and the instruction count from 1,910 to
1,888. `LDA` fell from 592 to 585 and `STA` from 507 to 500. SORTDM2 likewise
shrank from 2,891 to 2,830 bytes. The dedicated VM gate covers odd bases,
indices `$007F/$0080/$00FF`, page crossings, equal indexes, different arrays,
reversed operands, and all four unsigned relations under both backends. The
SORT VM oracle passes, all 20 modern/MIR6502 Toolkit programs compile, and TN
plus ALLOCATE remain byte-identical.

Commit independently.

## Slice 4: Signed Word Compare-to-Zero Selection

### Finding

After `SCompare`, `SAscend` and `SDescend` store `$A0/$A1` into spills and
expand a general signed word relation against zero. The expansion includes
constant sign comparisons such as `LDA #$00; CMP #$80`.

Classic uses the return high-byte sign directly:

- `value < 0` is a high-byte negative test;
- `value > 0` is nonnegative and nonzero.

The two routines are 81 instruction bytes larger in aggregate.

### Implementation

- Add direct signed WORD relation-to-zero selection before generic signed word
  compare expansion.
- Normalize zero on either side and all four relational predicates.
- Implement:
  - `< 0` and `>= 0` from the high-byte N flag;
  - `> 0` as nonnegative and `(lo | hi) != 0`;
  - `<= 0` as negative or zero.
- Load `$A0/$A1` directly when the known call-result state proves those bytes
  still contain the result.
- Avoid result spill homes when the compare/branch is the only use.
- Preserve branch target meaning without materializing a boolean.
- Add signed boundary fixtures for `$8000`, `$FFFF`, `$0000`, `$0001`, and
  `$7FFF`, with zero on both the left and right.

### Acceptance

- `SAscend` no longer contains generic `cmp_i16_*` scaffolding or result spills.
- `SDescend` performs only the necessary sign/nonzero tests.
- String sort in both directions passes the VM oracle.
- Existing general signed word comparison fixtures remain green.

Implemented result: `signed-return-word-zero-compare-branch` recognizes an
adjacent signed WORD relation consuming a public return-slot result. The
proof removes the logical call-result definition only when it is uniquely
consumed by the comparison and the condition is used only by the block
terminator. Zero is normalized from either operand position. `< 0` and `>= 0`
branch directly on the high-byte sign; `> 0` and `<= 0` preserve the
high-byte Z/N state across an empty edge and inspect the low byte only when
the high byte is zero.

The selector fires once in `SAscend` and once in `SDescend`. Both result
spills and the general signed comparison CFG are gone. SORTDM1 shrank from
4,754 to 4,672 load-file bytes. Recognized code fell from 4,392 to 4,314
bytes, data from 350 to 346 bytes, and the instruction count from 1,888 to
1,854. `LDA` fell from 585 to 575 and `STA` from 500 to 496. SORTDM2 likewise
shrank from 2,830 to 2,748 bytes. A dedicated MIR6502 VM gate covers all four
relations, zero on both sides, and signed values `$8000`, `$FFFF`, `$0000`,
`$0001`, and `$7FFF`. The SORT oracle passes, and TN plus ALLOCATE remain
byte-identical.

Commit independently.

## Slice 5: Word Arithmetic Directly into the `Y:$A3` Call Lane

### Finding

`QuickSort` computes `middle+1` and `middle-1` for the second word argument of
`AddList`. Pre-materialized MIR retains the correct single-use arithmetic
structure, but materialization writes the result through RAM spills before
loading `Y` and `$A3`.

The current call-argument expression selector handles important `A:X`
destinations but does not cover this second word lane.

### Implementation

- Extend the shared call-argument expression candidate to word arithmetic whose
  destination is `Y:fixed_zp $A3`.
- Schedule the second word argument before final `A:X` placement when doing so
  avoids clobbering the first argument.
- Keep the arithmetic low byte in Y when possible and place the propagated high
  byte directly in `$A3`.
- If the high-lane computation requires Y or overlaps `$A3`, reject or choose a
  proven safe scratch schedule rather than silently staging through RAM.
- Preserve carry/borrow from the low lane to the high lane.
- Require a single call consumer and dead producer definitions after the call
  site through the shared rewrite proofs.
- Add focused fixtures for addition, subtraction, carry, borrow, wrap, reversed
  argument order, a live first argument, and a conflicting fixed-ZP source.

### Acceptance

- The four branch-local `middle+1`/`middle-1` AddList arguments no longer use
  `sp34/sp35` or `sp44/sp45`-style RAM homes.
- The initial `len-1` call site improves if it satisfies the same general
  candidate; it is not special-cased.
- `QuickSort` shrinks measurably and all sort types pass the VM oracle.
- No SARGS callee-entry logic changes.

Commit independently.

## Slice 6: Fuse Two Word-Arithmetic Producers into Compare/Branch

### Finding

`AddList` compares `high+1` with `low+1`. MIR6502 materializes both word
arithmetic results into RAM before comparing them. The routine is 19
instruction bytes larger than classic.

### Implementation

- Extend the existing `WordArithmeticCompareCandidate` to accept arithmetic
  sources on both sides when both definitions are single-use and
  dominance-safe.
- Preserve the original modulo-16-bit arithmetic exactly.
- Keep one result in an available register/ZP pair while evaluating the other,
  then branch directly from the final comparison flags.
- Use the shared source-description and proof logic; do not introduce an
  AddList-specific pattern.
- Explicitly reject unsafe operand overlap, effect barriers, and scratch-state
  conflicts.
- Add wrap-focused tests proving that `$FFFF+1` behavior is not optimized into
  a non-wrapping algebraic comparison.

### Acceptance

- `AddList` loses its three comparison spill bytes and associated RAM traffic.
- The source expression remains semantically `high+1 > low+1`.
- Boundary and repeated-sort VM scenarios pass.

Commit independently.

## Slice 7: Definition-Sensitive QuickSort Result and Home Cleanup

### Finding

After `Partition`, MIR6502 copies `$A0/$A1` into both virtual zero-page bytes
and the `middle` local, then reloads those copies for the partition-size
comparison. Some duplication may remain after Slice 5.

### Implementation

- Reaudit `QuickSort` after Slices 5 and 6 before writing a matcher.
- Apply definition-sensitive dead-store analysis to individual result lanes
  whose later uses are already served by the canonical `middle` definition.
- Forward exact `$A0/$A1` result lanes only until a call or fixed-ZP clobber.
- Do not weaken whole-home liveness or remove the public/local `middle` store
  while any path still reads it.
- Prefer the existing known-callee result and exit-state infrastructure.

### Acceptance

- Every removed store is tied to an individually dead definition.
- No extra `$A0/$A1` reload survives solely to duplicate `middle`.
- QuickSort and the full SORT VM oracle remain correct.
- Skip this slice without code changes if the preceding slices already remove
  the duplication.

Commit only if it produces a coherent general improvement.

## Slice 8: Direct Indexed Values into Fixed Call-Argument Homes

### Finding

`Test` accounts for 376 instruction bytes of the SORTDM1 gap. For each
`PrintF`, MIR6502 calculates `d(i+n)` through absolute homes, stores the loaded
element into another spill, and later reloads it into the Action call ABI.

The routine currently has 20 spill slots and 280 spill accesses. Classic stages
the values directly in fixed call homes and moves the first variable argument
to Y only after the remaining arguments are prepared.

This is the largest single routine opportunity, but it is demo/output code and
is intentionally scheduled after the sorting core.

### Implementation

- Extend destination-aware call-argument selection for single-use indexed BYTE
  loads feeding Y or fixed-ZP argument homes.
- Prepare fixed-ZP arguments whose homes do not overlap pointer scratch
  `$AC-$AF` directly at their final destination.
- Delay the Y argument until later address calculations no longer need to
  clobber Y, or stage it in its existing ABI scratch byte rather than a general
  RAM spill.
- Fold `i+constant` into the indexed-load address calculation without creating
  a RAM word home when the sum has one load consumer.
- Initialize known zero-extension lanes directly in their final fixed homes.
- Preserve source evaluation order. Reject calls, machine blocks, absolute or
  volatile loads, arbitrary alias-sensitive dereferences, and argument-home
  overlap that the shared effect analysis cannot prove safe.
- Add focused calls with four through ten argument bytes, page-crossing array
  indexes, repeated fixed homes, and deliberately observable argument
  evaluation barriers.

### Acceptance

- Each SORTDM1 `PrintF` cluster has no general RAM spill between a proven-safe
  array-element load and its final fixed call home.
- `Test` spill slots and spill accesses fall substantially.
- The rewrite is described by argument destinations and effects, not by
  `PrintF` or format-string identity.
- Existing SARGS and call ABI fixtures remain green.
- The SORT VM oracle remains green even though it does not execute `Test`.
- Add a focused execution fixture for the new call-argument schedule if the
  ordinary emitted-shape tests cannot observe argument values.

Commit independently.

## Slice 9: Final Post-Home Cleanup and Listing Reaudit

### Implementation

- Rerun the existing post-home rewrite workflow after all new selectors.
- Inspect newly adjacent transfer, reload, tail-call, jump-to-return, and branch
  patterns.
- Add only general rewrites justified by shared liveness and exact machine-state
  proofs.
- Do not add a cleanup merely to match one classic instruction sequence.
- Regenerate full SORTDM1 and SORTDM2 listing analyses.
- Update this note with final per-slice measurements and mark completed slices.

### Final acceptance

- The SORT runtime oracle passes under modern/classic and modern/MIR6502.
- All ordinary tests pass.
- ALLOCATE and TN runtime/shape gates remain green.
- The complete Toolkit modern/MIR6502 batch compiles.
- `BAscend`, `BDescend`, `CAscend`, and `CDescend` contain no value spills for
  their final comparisons.
- `SAscend` and `SDescend` use specialized signed-zero branches.
- `QuickSort` does not stage `middle+1` or `middle-1` through ordinary RAM
  before `AddList`.
- `List(64)` backing is absent from load-file bytes but included in runtime
  allocation high water.
- Every remaining `Test` spill has a documented live-range, alias, ABI, or
  scheduling reason.

Reaching or beating the modern/classic 4,113-byte SORTDM1 result is a useful
checkpoint, not a correctness requirement. A slice is accepted because it
removes a proven inefficiency without weakening compiler guarantees.

## Stop and Rollback Rules

Stop or revert a slice if:

- either backend fails the hard-coded SORT runtime oracle;
- a sort result is monotonic but is not a permutation of the input;
- an array sentinel or the internal partition stack is corrupted;
- deferred storage overlaps code, another allocation, or the test arrays;
- a rewrite crosses a call, machine block, volatile/absolute access, or unknown
  pointer effect without proof;
- call argument order or fixed-home contents become observable and differ;
- a comparison relies on flags whose producer is not proven on every incoming
  edge;
- SORT improves only through a source/routine-specific exception;
- TN, ALLOCATE, or another Toolkit object grows without an understood and
  accepted tradeoff.

When a candidate is blocked, record the proof blocker and keep the conservative
code rather than weakening the workflow.
