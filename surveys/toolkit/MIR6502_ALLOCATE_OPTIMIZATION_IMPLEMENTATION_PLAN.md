# MIR6502 ALLOCATE Optimization Implementation Plan

Status: in progress; Slices 0-2 completed

Date: 2026-07-26

Planning baseline: `cac0fd3`

Source audit:
[`MIR6502_ALLOCATE_LISTING_REANALYSIS_2026-07-26.md`](MIR6502_ALLOCATE_LISTING_REANALYSIS_2026-07-26.md)

Scope: `samples/toolkit/modern/ALLOCATE.ACT`, modern profile, MIR6502 backend

## Objective

Close the remaining ALLOCATE gap without weakening the compiler's current
liveness, alias, pointer-overlap, absolute-memory, or machine-state guarantees.

The baseline is:

| Metric | MIR6502 | Modern/classic | Gap |
| --- | ---: | ---: | ---: |
| XEX bytes | 1,015 | 935 | +80 |
| Recognized instruction bytes | 958 | 880 | +78 |
| Data and inline machine bytes | 45 | 43 | +2 |
| Instructions | 436 | 381 | +55 |
| RAM spill labels | 4 | 0 | +4 |

The first checkpoint is not an arbitrary byte target. It is the removal of the
four comparison spill cells and the corresponding load/store traffic. The
second checkpoint is ALLOCATE parity with modern/classic while retaining
MIR6502's stronger overlap behavior.

## Decisions Fixed by This Plan

These decisions are part of the plan so that implementation does not require
another workflow refactor.

1. Target-independent predicate reasoning belongs in NIR.
2. 6502 comparison schedules, pointer pairs, registers, flags, and scratch
   bytes belong in MIR6502.
3. Every MIR operation rewrite uses the existing routine-aware rewrite driver
   and shared pre-home or post-home analysis snapshot.
4. A matcher describes a candidate; the driver proves removed definitions,
   observable exit state, and effects.
5. CFG mutation is separate from contiguous operation rewriting and is followed
   by CFG verification and full analysis invalidation.
6. Arbitrary pointer dereferences may alias routine storage and fixed ABI
   locations unless a shared analysis proves otherwise.
7. Absolute memory remains ordered and potentially volatile. Do not globally
   relax the existing absolute-memory exclusions to optimize `MemHi`.
8. Word copies and arithmetic updates retain read-before-write behavior when
   pointer ranges can overlap.
9. Do not add ALLOCATE-specific routine, symbol, address, or source-shape
   exceptions.
10. The classic backend is a directional comparison only and is not modified by
    this work.

## Pipeline Placement

| Work | Owner and phase |
| --- | --- |
| Direct word load to compare/branch | MIR6502, pre-home compare selection |
| Predicate-based branch threading | NIR optimization |
| Redundant fixed-pointer setup | MIR6502, post-home machine-value rewrite |
| Destination-aware two-pointer arithmetic | MIR6502, pre-home store selection plus post-home operation |
| Alias-safe direct-word to indirect copy | MIR6502, post-home structural rewrite |
| Indirect call-argument staging | MIR6502 call-argument materialization |
| Absolute-source arithmetic | MIR6502 only when exact ordering is preserved |
| Final block placement | MIR6502 CFG layout |

## Baseline and Measurement Protocol

Before Slice 1, regenerate:

```text
target/allocate-reanalysis-20260726/ALLOCATE.lst
target/allocate-reanalysis-20260726/ALLOCATE.peepholes
target/allocate-reanalysis-20260726/ALLOCATE-materialized.mir
target/allocate-reanalysis-20260726/ALLOCATE-pre.mir
target/allocate-reanalysis-20260726/ALLOCATE.map
target/allocate-reanalysis-20260726/ALLOCATE.xex
target/allocate-reanalysis-20260726/ALLOCATE.quality
```

The baseline XEX SHA-256 is:

```text
01afd51cee248126a119151804a8f3a878d2f705ed9c1865b14900e97baeadd6
```

After every behavior-changing slice:

1. run focused unit and shape tests;
2. run `cargo test`;
3. regenerate ALLOCATE listing, materialized MIR, peephole report, map, and XEX;
4. record XEX size, instruction bytes, LDA, STA, spill cells, and selector count;
5. regenerate TN modern/MIR6502 and reject unexplained growth or code-shape
   regressions;
6. inspect the changed routines rather than relying only on aggregate size;
7. commit that slice separately.

Generated target artifacts remain uncommitted. Stable source fixtures, runtime
probes, and documentation are committed with the slice that needs them.

## Slice 0: VM-Backed Allocator Runtime Oracle

### Goal

Establish an execution-level correctness gate before changing ALLOCATE's word
comparisons, pointer scheduling, or overlapping-memory behavior.

The existing VM already provides the required observability: deterministic
object loading, a bounded generated-code run, a final 64K memory dump, recent
instruction history, address-range watch logging, and optional Action routine
tracing from a listing or map. No VM change is required for this slice.

### Fixture and harness

- Add a focused Action! fixture that includes the maintained modern
  `ALLOCATE.ACT` implementation.
- Keep the test heap and result area at fixed addresses so the host check does
  not depend on compiler-selected storage.
- Reset the free-list sentinel between independent scenarios.
- Exercise both modern/classic and modern/MIR6502.
- Leave execution in a generated-code loop and stop through the VM step bound,
  following the existing runtime-fixture convention.
- Dump memory and compare hard-coded expected bytes. Backend equality alone is
  not a correctness oracle.
- On failure, retain a concise result-area dump. Developers can rerun with the
  VM's existing history, watch-range, and Action-call tracing controls.

### Required scenarios

- empty-list allocation;
- exact-size removal;
- split allocation;
- skipping an undersized block;
- comparisons around `$00FF/$0100`;
- insertion without coalescing;
- left, right, and two-sided coalescing;
- repeated allocate/free operations;
- `AllocInit` using controlled `EndProg` and `MemHi` values where the fixture
  can do so without changing ALLOCATE's production declarations.

If an `AllocInit` case cannot safely control the absolute OS-backed `MemHi`
source in the execution environment, keep it as a separate fixture or document
that exclusion rather than weakening the allocator fixture.

### Integration

- Add an ignored compatibility test invoking the VM harness.
- Document the direct command in `fixtures/runtime/README.md`.
- Keep the runtime script opt-in because it depends on the sibling
  `action-compiler-vm` checkout and Atari ROM images.

### Acceptance

- Both backends produce the exact expected result and heap snapshots.
- The test fails for a comparison result, link, size, insertion, or coalescing
  error.
- `cargo test` remains green without requiring the external VM.
- The opt-in VM compatibility test passes.

Commit independently before Slice 1.

## Slice 1: General Word-Source Description

### Goal

Give compare and store consumers one typed description of a word producer
without duplicating safety rules.

The current `WordArithmeticSource` and its helpers live in
`materialize/compare_branch.rs`, although `store_consumers.rs` also consumes
them. The next compare selector is a third use of the same source model.

### Implementation

- Move the source description and only its directly related helpers into a
  small `materialize/word_sources.rs` module.
- Rename it to a consumer-neutral name such as `WordConsumerSource`.
- Preserve the two existing source forms:
  - direct byte values;
  - one indirect word with a typed pointer value and constant offset.
- Preserve the existing distinction between:
  - reorderable ordinary storage;
  - pointer-source storage;
  - excluded absolute/fixed-hardware storage.
- Keep source matching independent of a particular ALLOCATE shape.
- Update compare and store consumers to use the shared model.

### Tests and acceptance

- Existing word-arithmetic compare, indirect-store, pointer, and result tests
  remain green.
- ALLOCATE and TN materialized MIR and XEX remain byte-identical.
- No selector count changes.

This is a behavior-neutral commit and must not be combined with Slice 2.

## Slice 2: Direct Equality and Inequality Word Compare-to-Branch

### Goal

Select ordinary word loads directly into `Eq`/`Ne` branches without creating
logical homes.

The first ALLOCATE target is:

```text
load.w *current+0
load.w nBytes
compare.w Eq
branch
```

### Candidate

Add a typed candidate in `materialize/compare_branch.rs` for:

```text
zero or more recognized word loads
Compare Eq/Ne word, unsigned
BoolValue branch using that exact compare result
```

Initial legality:

- both operands have a single use in the compare;
- the compare result is used only by the block terminator;
- at most one operand is indirect;
- an indirect operand uses an ordinary local/parameter/global/spill-backed
  pointer and a constant byte-sized offset;
- the other operand is constants or reorderable direct storage;
- there is no call, machine block, barrier, volatile absolute access, or
  unknown memory effect inside the candidate;
- no removed definition is live after the rewrite.

### Selection

- Materialize the indirect pointer once.
- Compare one lane and branch immediately on mismatch.
- Compare the surviving lane and branch to the original successors.
- Use explicit MIR CFG blocks; do not materialize a boolean byte.
- Preserve the source read order whenever the memory-effect analysis does not
  prove reordering safe.
- Feed the candidate through `PreHomeAnalysisSnapshot`,
  `PreHomeRewriteContext`, and `prove_removed_window_definitions`.
- Record a dedicated statistic such as
  `word-load-equality-compare-branch`.

### Tests

Add focused positive tests for:

- indirect left, direct right;
- direct left, indirect right with safe operand reversal;
- `Eq`;
- `Ne`;
- zero and nonzero offsets.

Add negative tests for:

- two indirect operands;
- absolute or hardware-backed direct memory;
- an intervening barrier/call/machine block;
- reused operand temp;
- reused comparison temp;
- signed comparison;
- a removed word lane live in the successor.

### Acceptance

- The equality comparison in `Alloc` no longer uses `spill8`-`spill11`.
- ALLOCATE loses the equality-side spill traffic.
- No absolute-memory or two-pointer case is accepted accidentally.
- Full tests pass.

Commit independently.

Implementation result:

- one `word-load-equality-compare-branch` site selected in `Alloc`;
- ALLOCATE decreased from 1,015 to 995 XEX bytes;
- recognized instruction bytes decreased from 958 to 938;
- equality-side compare spill accesses decreased by eight;
- TN remained byte-identical at 9,994 XEX bytes;
- the ALLOCATE VM oracle passed under modern/classic and modern/MIR6502.

## Slice 3: Direct Unsigned Relational Word Compare-to-Branch

### Goal

Extend Slice 2 to `Lt`/`Ge`, then cover `Gt`/`Le` only through proven-safe
operand reversal.

### Selection

Use a real 16-bit carry chain:

```text
load low-left
CMP low-right
load high-left
SBC high-right
branch on final carry
```

The low-byte compare establishes the borrow input for the high-byte
subtraction. No operation between those instructions may overwrite carry.

Requirements:

- unsigned comparisons only;
- carry production/consumption is explicit in MIR;
- operand reversal is allowed only when both memory-order and comparison
  semantics remain safe;
- the candidate still permits no more than one indirect source;
- mismatch/equality helper blocks are not introduced unless the selected
  relation requires them.

### Tests

Cover boundary pairs:

```text
$0000 / $0000
$00FF / $0100
$0100 / $00FF
$7FFF / $8000
$FFFF / $0000
```

Test all supported relations and both branch polarities. Add a runtime probe or
equivalent emitted-code execution test for the carry chain rather than relying
only on MIR shape.

### Acceptance

- The `<` comparison in `Alloc` uses no spill homes.
- `spill8` through `spill11` disappear completely.
- ALLOCATE has zero RAM spill labels.
- The expected combined Slice 2/3 opportunity is roughly 45-50 instruction
  bytes plus four data bytes, but the emitted result is authoritative.
- Full tests and the comparison runtime probe pass.

Commit independently and perform the first listing reanalysis.

## Slice 4: NIR Predicate-Based Branch Threading

### Goal

Remove the repeated `current == 0` test after the loop without introducing
target information into NIR.

This is not ordinary constant propagation. The join block receives two
different facts:

- the null exit proves `current == 0`;
- the size-comparison exit is reachable only after proving `current != 0`.

The optimizer must redirect each predecessor edge to the successor selected by
its own predicate facts.

### Analysis

Add a conservative target-neutral predicate analysis under `nir/analysis`.
The initial fact domain should contain only:

- equality or inequality of a promotable direct storage value and an integer
  constant;
- equality or inequality of an unchanged temp and an integer constant;
- facts learned from true and false branch edges;
- facts that survive a block only when no relevant store or memory barrier can
  invalidate them.

Use stable `BlockId`, storage identity, and `TempId`. Do not encode 6502 flags,
registers, or pointer pairs.

### Transform

Implement restricted predicate-based branch threading:

- inspect a side-effect-free compare-and-branch block;
- evaluate that branch separately for each incoming edge;
- retarget only incoming edges whose predicate facts prove one successor;
- compose edge arguments correctly;
- initially reject blocks with parameters or successor arguments that depend on
  definitions inside the bypassed block;
- remove the now-unreachable block through existing CFG cleanup.

Do not clone general blocks in this slice.

### Safety tests

Positive coverage:

- two incoming edges decide opposite successors;
- a fact propagates through a side-effect-free intermediate block;
- a loop exit matching the ALLOCATE shape.

Negative coverage:

- direct store to the tracked storage;
- unknown indirect store when promotability cannot prove disjointness;
- call, machine block, or barrier;
- block parameter or successor argument dependency;
- fact disagreement at a join without per-edge proof;
- loop backedge convergence.

### Required NIR checks

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Any fixture change must be documented as an intentional CFG optimization, not
a printer change.

### Acceptance

- The optimized NIR for `Alloc` routes the null edge directly to the null
  return and the nonnull size-exit edge directly to the equality test.
- The repeated NIR load/compare of `current == 0` disappears.
- MIR6502 loses the `$3093-$309D` zero-test scaffold.
- Classic may also improve because this is a target-neutral NIR pass; measure
  both backends.

Commit independently.

## Slice 5: Cross-Edge Redundant Fixed-Pointer Setup Removal

### Goal

Reuse an exact fixed pointer-pair value established by a predecessor instead of
reloading the same local pointer.

After Slices 2-4, the nonnull edge into `Alloc`'s equality block should already
leave `$AC/$AD` holding `current`.

### Implementation

Add a post-home structural rewrite for the lowered pointer-setup sequence:

```text
LDA pointer.low
STA fixed-pair.low
LDA pointer.high
STA fixed-pair.high
```

The candidate is removable only when
`MirMachineValueAvailability` proves at the first operation that both fixed
bytes already contain those exact direct-memory values.

Use `PostHomeRewriteContext` to prove:

- the fixed-pair bytes are exact on every executable predecessor;
- the source memory has not been invalidated;
- removing the loads does not change an observable A or Z/N exit value;
- removing the stores does not change observable fixed scratch state;
- no intervening call, machine block, indirect write, or unknown effect kills
  the fact.

Do not make a second pointer-state data-flow implementation. Extend the existing
machine-value query surface if the rewrite context lacks a typed pair query.

### Tests

- same pointer value on one predecessor;
- same value on multiple predecessors;
- different value on one predecessor rejects;
- one byte changed rejects;
- source store rejects;
- unknown indirect effect rejects;
- live A/Z/N exit state rejects or retains the minimal required load;
- known-callee preservation works through the existing summaries.

### Acceptance

- The repeated equality-block pointer setup in `Alloc` disappears.
- At least one existing `ssa-lite-v2-address-reuse-candidates` site becomes an
  applied rewrite rather than telemetry only.
- No raw opcode peephole is added.

Commit independently, then perform the second listing reanalysis.

## Checkpoint A: Re-rank Before Adding New MIR Operations

After Slice 5:

1. regenerate the full ALLOCATE audit;
2. compare routine sizes with both the 1,015-byte baseline and modern/classic;
3. regenerate TN and inspect changed selectors;
4. recount residual ZP lanes and pointer preparations;
5. update this plan with actual results.

Do not automatically implement Slices 6-9 if the listing no longer contains
their target shapes or if the remaining gap has moved elsewhere.

## Slice 6: Destination-Aware Two-Pointer Word Update

### Goal

Fuse:

```action
target.size = target.size + current.size
```

without the four source homes and two result homes seen at
`$327C-$32C5`.

### Candidate

Match:

```text
left  = load.w *target+offset
right = load.w *source+offset
sum   = left Add/Sub right
store.w *target+offset = sum
```

Restrictions:

- destination pointer and left operand pointer are identical;
- both offsets are identical and byte-sized;
- the second pointer is distinct as an identity, but may point to overlapping
  memory at runtime;
- the arithmetic result has no other consumer;
- no call, barrier, machine block, or intervening memory effect;
- Add is implemented first; Sub is enabled only after its borrow schedule has
  equivalent coverage.

### MIR operation

If existing MIR operations cannot express the profitable carry-preserving
schedule, add one narrow MIR6502 operation, for example:

```text
IndirectWordCompound {
    op,
    target,
    source,
    offset,
}
```

Its contract must state:

- both source lanes are read before either destination lane is written;
- low-to-high destination write order is preserved;
- carry flows from low arithmetic to high arithmetic;
- A, Y, carry, Z/N, and fixed result scratch effects are explicit;
- target and source fixed pointer pairs are distinct and reserved;
- arbitrary overlap, including equal pointers and cross-lane overlap, is safe.

The expected implementation prepares target and source in separate fixed pairs,
computes both result lanes into fixed scratch, then writes both lanes.

Update:

- MIR IR and phase verifier;
- effect classification;
- home and machine liveness;
- zero-page reservation;
- emitter;
- printer;
- pseudo-machine contract documentation;
- rewrite candidate and proof plan;
- peephole telemetry.

### Profitability

Compute the actual emitted-byte estimate before applying the candidate. Reject
the rewrite when pointer preparation and staging are not smaller than the
ordinary sequence. This prevents repeating the rejected generic two-pointer
experiment.

### Tests

In addition to MIR shape tests, execute runtime cases for:

- disjoint words;
- identical target/source pointers;
- source one byte above target;
- source one byte below target;
- page crossing;
- `$FFFF` wrap behavior where the runtime permits it;
- carry and borrow propagation.

### Acceptance

- The Free merge block loses the `$E4-$E7` source/result traffic.
- The candidate applies once to ALLOCATE.
- The expected opportunity is approximately 20-23 bytes.
- Runtime overlap tests pass.

Commit independently.

## Slice 7: Alias-Safe Direct-Word to Indirect Copy

### Goal

Reduce staging for:

```action
target.next = current
last.next = target
```

without assuming that the indirect destination is disjoint from routine
storage.

### Implementation

Prefer a post-home operation/rewrite that:

1. prepares the destination pointer;
2. reads both direct source lanes before either indirect write;
3. holds the word in A plus one proven-dead index register, with balanced stack
   staging if needed;
4. writes low then high;
5. declares all A/X/Y/flag/stack effects to the post-home proof context.

A narrow operation such as `CopyDirectWordToIndirect` is acceptable if it
avoids duplicating an opaque opcode sequence in multiple selectors. It must not
apply unless machine liveness proves its register clobbers unobservable.

Do not implement the smaller but unsafe sequence that prepares the pointer and
then reloads a possibly aliased direct source one lane at a time.

### Tests

- destination aliases source exactly;
- destination overlaps source by one byte in either direction;
- X live rejects the X-clobbering schedule;
- stack effects are balanced;
- fixed pointer and flags remain correctly classified;
- two ALLOCATE Free sites select.

### Acceptance

- Both Free non-coalescing word stores remain overlap-safe.
- Emitted code shrinks; reject the slice if the real selector cost is neutral
  or larger.

Commit independently.

## Slice 8: Indirect Field Loads Directly Into Call Homes

### Goal

Remove `PrintFreeList`'s four virtual-ZP argument staging bytes while preserving
simultaneous argument evaluation.

### Candidate

Recognize multiple word call arguments loaded through the same pointer with
constant, nonoverlapping field offsets and placed into consecutive fixed-ZP ABI
homes.

The selection must:

- read every indirect source byte before writing any potentially aliasing ABI
  home;
- prepare the common pointer once;
- use a balanced stack or another explicitly modeled staging mechanism;
- restore the final A/X/Y call arguments after staging;
- stop at calls, machine blocks, barriers, or unknown effects;
- preserve source-language argument evaluation order.

If a new pseudo operation is required, give it a general contract such as
copying a bounded indirect byte range into fixed call homes. Do not name
`PrintF` or encode its address in the operation.

### Tests

- two adjacent word fields into four fixed homes;
- source pointer aliases the destination ABI range;
- pointer offsets cross a page;
- nonconsecutive homes reject;
- mixed effectful arguments reject;
- call A/X/Y setup remains correct;
- known machine-call effects remain conservative.

### Acceptance

- `$E0-$E3` disappear from `PrintFreeList`.
- All four indirect reads occur before the first `$A4-$A7` write.
- The expected opportunity is roughly 8-15 bytes.

Commit independently.

## Slice 9: AllocInit Ordered-Absolute Cleanup

### Goal

Improve `AllocInit` only where exact absolute-memory and pointer-write ordering
can be preserved.

### Rules

- Do not add a general rule that treats `MirMem::Absolute` as ordinary memory.
- Do not interleave another memory access between the two bytes of an absolute
  word load unless the original MIR already permits that ordering.
- Do not write through `p` until both absolute source bytes needed afterward
  have been read.
- Do not reload a direct source after an indirect write unless disjointness is
  proven.

### Candidate work

First inspect whether post-home machine liveness can replace a result
store/reload pair with a dead X or Y carrier while preserving the absolute
reads and low-to-high indirect writes. Separately inspect repeated direct loads
of `EndProg`, but retain early staging if the indirect `FreeList.next` write
could alias the local `p` home.

If no generally safe smaller schedule exists, record the rejection and leave
the eight-byte classic gap intact. A documented no-op is preferable to a
sample-specific or volatility-unsafe optimization.

### Acceptance

- Any applied rewrite has negative tests for volatile absolute reads and
  pointer aliasing.
- ALLOCATE shrinks measurably.
- Otherwise the slice ends with documentation only and no code change.

Commit only a real implementation or a useful documented rejection.

## Slice 10: Size-Aware CFG Layout

### Goal

Run layout after the structural optimizations and remove remaining long
branch-over-`JMP` scaffolding.

### Implementation

Extend the current reverse-postorder layout cost model rather than adding a
second layout pass. Account for:

- fall-through successor choice;
- estimated block byte size;
- relative branch reachability;
- unconditional jump cost;
- pure return blocks;
- deterministic tie breaking by stable block ID.

Keep compare helper blocks near their producer. Prefer loop continuation and
exit placement using the real post-expansion CFG. Do not change branch
semantics merely to improve layout.

### Tests

- near and far alternate targets;
- loop header, body, continuation, and exit;
- compare helper chains;
- pure return successor;
- equal-cost deterministic ordering;
- no layout change when the estimated cost is not lower.

### Acceptance

- Recount branch-over-`JMP` forms after Slices 2-9.
- Apply only when estimated and actual emitted bytes decrease.
- Do not use the original four-pattern estimate after earlier slices have
  changed the CFG.

Commit independently.

## Final Validation and Documentation

After the final profitable slice:

```sh
cargo fmt --check
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Also:

- run every new runtime comparison/overlap probe;
- compile ALLOCATE under modern/MIR6502 and modern/classic;
- compile TN under modern/MIR6502;
- run the Toolkit modern/MIR6502 batch;
- inspect KALSCOPE and the existing dual-pointer runtime coverage;
- regenerate the ALLOCATE final listing audit;
- update this plan with applied selector counts, exact size deltas, and rejected
  slices.

## Commit Boundaries

Use one commit per slice:

```text
mir6502: share word consumer source matching
mir6502: select direct word equality branches
mir6502: select direct unsigned word relational branches
nir: thread branches using executable-edge predicates
mir6502: remove redundant fixed pointer setup
mir6502: select destination-aware indirect word updates
mir6502: select alias-safe direct-to-indirect word copies
mir6502: stage indirect call fields directly into ABI homes
mir6502: preserve ordered absolute arithmetic stores
mir6502: account for branch reach in block layout
```

If a slice needs a new MIR operation, its IR, verifier, effects, emitter,
printer, tests, and contract documentation belong in the same vertical-slice
commit. Do not land a temporarily unverified operation.

## Stop Conditions

Pause and reanalyze rather than broadening a matcher when:

- ALLOCATE grows;
- TN grows without a clearly explained tradeoff;
- a candidate requires treating arbitrary pointers as nonaliasing;
- a candidate requires treating absolute memory as stable;
- a new MIR operation duplicates an existing operation's contract;
- the rewrite cannot state its A/X/Y/flags/fixed-scratch exit effects;
- a runtime overlap or carry test fails;
- the targeted listing shape disappeared after an earlier slice.

These are slice stop conditions, not reasons to weaken the shared compiler
invariants.
