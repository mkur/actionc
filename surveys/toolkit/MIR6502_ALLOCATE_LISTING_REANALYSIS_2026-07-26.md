# ALLOCATE MIR6502 Final-Listing Reanalysis

Status: current optimization backlog

Date: 2026-07-26

Baseline commit: `cac0fd3` (`mir6502: reuse arithmetic pointers for word results`)

Scope: `samples/toolkit/modern/ALLOCATE.ACT`, `--profile modern --backend
mir6502`

Implementation reference:
[`MIR6502_ALLOCATE_OPTIMIZATION_IMPLEMENTATION_PLAN.md`](MIR6502_ALLOCATE_OPTIMIZATION_IMPLEMENTATION_PLAN.md)

## Purpose

This note audits the final ALLOCATE listing after word arithmetic was fused
into compare, indirect-store, pointer, and result consumers. It measures the
remaining output against the modern/classic backend and ranks the next
MIR6502 optimization opportunities.

The final listing is the source of truth for emitted instruction shape.
Materialized and pre-materialized MIR are used to explain why a sequence
survived. Classic output is a directional target-strategy comparison, not a
correctness oracle: some classic sequences rely on weaker pointer-alias
assumptions than MIR6502 currently permits.

## Reproducing the Artifacts

Run from the repository root:

```sh
mkdir -p target/allocate-reanalysis-20260726

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/toolkit/modern/ALLOCATE.ACT \
    > target/allocate-reanalysis-20260726/ALLOCATE.lst \
    2> target/allocate-reanalysis-20260726/ALLOCATE.peepholes

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/toolkit/modern/ALLOCATE.ACT \
  > target/allocate-reanalysis-20260726/ALLOCATE-materialized.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-mir6502 \
  samples/toolkit/modern/ALLOCATE.ACT \
  > target/allocate-reanalysis-20260726/ALLOCATE-pre.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-map \
  samples/toolkit/modern/ALLOCATE.ACT \
  > target/allocate-reanalysis-20260726/ALLOCATE.map

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/toolkit/modern/ALLOCATE.ACT \
  > target/allocate-reanalysis-20260726/ALLOCATE.xex

cargo run --quiet --bin actionc-listing-quality -- \
  target/allocate-reanalysis-20260726/ALLOCATE.lst \
  > target/allocate-reanalysis-20260726/ALLOCATE.quality
```

Repeat the listing, map, load, and quality commands with `--backend classic`
for the comparison artifacts.

The listing-quality report counts the three inline bytes following Free's
SARGS helper call as data rather than instructions. That is intentional.
The XEX size and segment range remain authoritative.

## Current Result

The MIR6502 load file is 1,015 bytes. Its main segment spans
`$3000-$33EA`, or 1,003 bytes; the remaining 12 bytes are XEX headers and the
run-vector segment.

The result is byte-identical to the preceding
`ALLOCATE-word-arith-result.xex` artifact. Its SHA-256 is:

```text
01afd51cee248126a119151804a8f3a878d2f705ed9c1865b14900e97baeadd6
```

The modern/classic load file is 935 bytes with SHA-256:

```text
8a4b0ae45ecfb6c43139f27914468bfb6e529f45257c56bde52d1f7d9209b584
```

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| Load file | 1,015 | 935 | +80 |
| Recognized instructions | 436 | 381 | +55 |
| Recognized instruction bytes | 958 | 880 | +78 |
| Data and inline machine bytes | 45 | 43 | +2 |
| `LDA` | 152 | 136 | +16 |
| `STA` | 142 | 128 | +14 |
| `LDA` + `STA` instructions | 294 | 264 | +30 |
| `LDA` + `STA` instruction share | 67.4% | 69.3% | -1.9 points |
| `LDA` + `STA` byte share | 73.3% | 74.0% | -0.7 points |
| RAM spill labels | 4 | 0 | +4 |
| `JMP` | 8 | 6 | +2 |

The earlier dual-pointer baseline was 1,091 bytes. The four word-arithmetic
consumer slices therefore removed 76 bytes:

| Selector | ALLOCATE applications |
| --- | ---: |
| `word-arithmetic-compare-branch` | 2 |
| `word-arithmetic-indirect-store-consumer` | 2 |
| `word-arithmetic-result-consumer` | 1 |

## Routine Concentration

| Routine | MIR6502 instruction bytes | Classic instruction bytes | Difference |
| --- | ---: | ---: | ---: |
| `Alloc` | 345 | 314 | +31 |
| `Free` | 395 | 371 | +24 |
| `AllocInit` | 98 | 90 | +8 |
| `PrintFreeList` | 120 | 105 | +15 |
| **Total** | **958** | **880** | **+78** |

The gap is distributed across all four routines. `Alloc` and `Free` remain the
largest targets, but the causes differ: `Alloc` is dominated by comparison
materialization, while `Free` is dominated by indirect-store and two-pointer
staging.

## 1. Direct Ordinary Word Compare-to-Branch Selection

This is the highest-value next slice.

The comparisons at `$305B-$3092` and `$309E-$30CD` implement:

```action
current.size < nBytes
current.size = nBytes
```

Although each value has only one comparison consumer, MIR6502 writes both word
operands into `spill8` through `spill11` and then reloads them:

```text
spill8  $301A
spill9  $301B
spill10 $301C
spill11 $301D
```

Together these homes have eight writes and eight reads. They are all four RAM
spill cells in the program. The materialized MIR reports eight compare-consumer
lanes with RAM fates.

The new arithmetic-to-compare selector does not apply because these producers
are ordinary word loads, not arithmetic. The needed general selector is:

```text
word load-indirect/direct operands
    -> unsigned equality or relational comparison
    -> explicit branch
```

It should consume the load operands directly, preserve the required carry
chain, and select the branch without materializing either word. The equivalent
classic shapes show an opportunity of approximately 45-50 instruction bytes,
plus four data bytes from deleting the spill cells.

This change should remain proof-gated and reject operands whose reads cannot be
safely reordered, including volatile absolute memory and hardware locations.

## 2. Propagate Exit-Edge Conditions and Prepared Pointers

The source first exits:

```action
WHILE (current<>NULL) AND (current.size<nBytes)
```

and then immediately asks:

```action
IF current=NULL THEN
```

The listing repeats the word-zero test at `$3093-$309D`. On the loop condition's
null edge, `current` is known to be zero. On the size-comparison exit edge,
`current` is known to be nonzero. Neither predecessor changes `current`.

A routine-wide condition fact can therefore split the successors directly:

```text
current == 0       -> null return
current != 0 and size >= nBytes -> equality test
```

The nonnull comparison edge also leaves `$AC/$AD` prepared with `current`.
The equality block currently prepares the same pointer again. Coordinating NIR
edge-condition propagation with MIR6502 fixed-pointer exit state could remove:

- the repeated zero test and its long jump;
- the repeated `$AC/$AD` pointer setup;
- some of the block-layout scaffolding around the null return.

The independent opportunity is approximately 11-21 bytes, depending on how
much overlaps the direct compare selection.

## 3. Destination-Aware Two-Pointer In-Place Addition

The right-side free-list merge at `$327C-$32C5` implements:

```action
target.size = target.size + current.size
```

It prepares `target`, loads its two lanes into `$E4/$E6`, prepares `current`,
loads its two lanes into `$E5/$E7`, performs the addition into `$E4/$E6`, then
prepares `target` again to store the result.

This is not a reason to restore the previously rejected generic two-pointer
consumer. That implementation grew ALLOCATE because it staged too much state.
The profitable shape is narrower:

```text
destination pointer == left operand pointer
second prepared source pointer
word add/sub
same indirect destination
```

An overlap-safe implementation must read everything that a low-lane write
might alias before performing that write. One practical shape keeps both
pointers prepared, stages the low result in a fixed scratch byte, computes and
stores the high result, then stores the staged low result.

The comparable classic sequence is approximately 23 bytes smaller, although
MIR6502 must retain its stronger overlap guarantees.

## 4. Reduce Staging Around Indirect Stores

Free's non-coalescing branch at `$3226-$3262` performs:

```action
target.next = current
last.next = target
```

Each source word is staged in virtual ZP before the destination pointer is
prepared. The block is about 16 bytes larger than classic.

Loading the source only after preparing the destination would be smaller, but
is not generally safe if an arbitrary pointer may alias routine storage. A
profitable selector therefore needs one of:

- a proof that the destination cannot alias the direct source home;
- register or stack staging that survives pointer preparation;
- a narrower storage-class rule that establishes disjoint regions.

This is a medium-priority opportunity of roughly 12-16 bytes, but it should not
be implemented by silently adopting classic's weaker alias assumption.

## 5. Place PrintF Arguments Without Four Residual ZP Homes

`PrintFreeList` is 15 bytes larger than classic, all in the argument preparation
range `$3392-$33CF`.

MIR6502 first reads `p.size` and `p.next` into four virtual-ZP bytes
`$E0-$E3`, then reloads those bytes into ABI locations `$A4-$A7`. The four
residual lanes account for four stores and four reloads.

Direct placement into `$A4-$A7` would recover most of the 15-byte gap. It is
only safe if evaluation of later indirect operands cannot observe earlier ABI
stores through pointer aliasing. The implementation should therefore be based
on explicit argument-evaluation and alias facts, or on a compact register/stack
staging schedule, rather than an unconditional peephole.

## 6. Finish AllocInit Arithmetic Placement

`AllocInit` is eight bytes larger than classic. Two related word values remain
materialized through a two-byte virtual-ZP home:

- `EndProg`, before it is stored into both `p` and `FreeList.next`;
- `MemHi-p`, before it is stored through `p`.

The report records a single word subtraction-to-store candidate, but its result
lanes remain live in the selected home. Extending the arithmetic-to-indirect
store path to safe absolute-memory operands can remove the result round trip.
Because `MemHi` is absolute OS memory, the selector must preserve read order
and must not assume that arbitrary indirect storage is disjoint without proof.

## 7. Coordinate Block Layout With Comparison Fusion

The final listing contains four branch-over-`JMP` patterns. None can currently
be converted to a single relative branch because the alternate destination is
out of range.

The distant destinations are mainly:

- `Alloc`'s loop continuation and null-return blocks;
- `Free`'s loop continuation block.

Moving these blocks nearer their tests can remove an estimated 3-6 bytes.
Layout should follow the compare and edge-fact work, because those changes will
replace several of the branches being measured.

## Patterns Not to Prioritize

The final listing contains no:

- adjacent `STA m; LDA m` pairs;
- absolute `$00xx` accesses;
- `LDA; CMP #0; BEQ/BNE` forms;
- `JSR; RTS` tails;
- load/add-one/store or load/sub-one/store sequences.

The five-byte premium in Free's final indirect word copy is also not an
immediate target. MIR6502 reads both source lanes before either destination
write, preserving overlapping-copy behavior that the shorter classic sequence
does not preserve.

The peephole report's `analyzed-rewrite-estimated-bytes-saved: 127` is not an
additive opportunity estimate. It includes the same blocked candidates
revisited during multiple fixed-point rounds. The final listing and residual
lane fates are the reliable basis for prioritization.

## Recommended Order

1. Add proof-gated ordinary word load-to-compare-and-branch selection.
2. Propagate the loop exit condition and prepared-pointer state into `Alloc`.
3. Add the destination-equals-left-operand two-pointer update selector.
4. Improve safe source placement for Free's indirect word stores.
5. Improve `PrintFreeList` argument placement with explicit alias guarantees.
6. Extend safe absolute-source arithmetic placement in `AllocInit`.
7. Re-run block layout after the structural changes.

The first three items expose approximately enough static savings to close the
current 80-byte gap, although their estimates overlap and must be remeasured
after every slice.
