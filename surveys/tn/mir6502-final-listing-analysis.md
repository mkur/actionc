# TN MIR6502 Final Listing Analysis

Status: public-ABI shadow-store slice implemented; remaining MIR6502 work ranked

Date: 2026-07-20

Baseline commit: `d42a519` (`mir6502: color nonlocal routine spill homes`)

Scope: `samples/tn/modern/TN.ACT`, `--profile modern --backend mir6502`

## Purpose

This note analyzes TN from the final emitted 6502 listing backward. The final
listing is the source of truth for static bytes and instruction selection;
materialized and pre-materialized MIR are used only to explain why a costly
sequence survived.

The first recommendation from the original listing analysis, proof-guided
removal of redundant caller-side public-ABI argument mirrors, is now
implemented. It removes 601 TN load-file bytes. Scaled word indexing is now the
largest well-bounded addressing slice.

## Reproducing the artifacts

Run from the repository root:

```sh
mkdir -p target/tn-final-listing-analysis

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/tn/modern/TN.ACT \
    > target/tn-final-listing-analysis/TN.lst \
    2> target/tn-final-listing-analysis/TN.peepholes

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-final-listing-analysis/TN-materialized.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-final-listing-analysis/TN-pre.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-map \
  samples/tn/modern/TN.ACT \
  > target/tn-final-listing-analysis/TN.map

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/tn/modern/TN.ACT \
  > target/tn-final-listing-analysis/TN.xex

cargo run --quiet --bin actionc-listing-quality -- \
  target/tn-final-listing-analysis/TN.lst
```

For directional comparison, repeat the listing and load commands with
`--backend classic`.

The listing-quality parser can interpret bytes inside machine/data procedures
as instructions. Load-file size and segment address ranges are authoritative.
Instruction counts in this note include only recognized 6502 mnemonics; they
remain comparative measurements rather than segment accounting.

## Current result

The load file is 12,102 bytes. Its primary segment spans `$2C00-$5B39` and is
12,090 bytes; the other 12 bytes are XEX headers and the run-vector segment.

The load-file SHA-256 is:

```text
570850e021155b12cb695d0ef51a7e35e970678172f3764525aa24d9864fe630
```

This is 2,043 bytes smaller than the 14,145-byte MIR6502 baseline in
`mir6502-optimization-opportunities.md`, and 601 bytes smaller than the
12,703-byte pre-shadow-elision result at baseline commit `d42a519`.

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| Load file | 12,102 | 10,445 | +1,657 |
| Recognized instructions | 4,950 | 4,236 | +714 |
| Recognized instruction bytes | 11,098 | 9,408 | +1,690 |
| `LDA` + `STA` instructions | 2,429 | 1,910 | +519 |
| `LDA` + `STA` instruction share | 49.1% | 45.1% | +4.0 points |
| `LDA` + `STA` bytes | 5,655 | 4,562 | +1,093 |
| `LDA` + `STA` byte share | 51.0% | 48.5% | +2.5 points |
| `JMP` | 197 | 158 | +39 |
| `JSR` | 369 | 368 | +1 |

Classic is a directional target-strategy comparison, not a byte-level oracle.
It interleaves storage and hidden data with routine ranges and makes different
layout choices. The almost identical call count is nevertheless useful: the
remaining MIR6502 excess is around calls and values, not caused by doing more
calls.

## Routine concentration

The largest positive matched-routine differences are:

| Routine | MIR6502 bytes | Classic bytes | Difference |
| --- | ---: | ---: | ---: |
| `Handle` | 1,030 | 839 | +191 |
| `Copy` | 864 | 723 | +141 |
| `Tag` | 175 | 93 | +82 |
| `Draw` | 198 | 137 | +61 |
| `Xloop` | 266 | 221 | +45 |
| `Inv` | 78 | 53 | +25 |
| `SwapScr` | 201 | 196 | +5 |
| `SetWin` | 1,177 | 1,233 | -56 |

`SetWin` is still the largest routine and the largest remaining temp-home
traffic site, but it is already smaller than classic. Optimization priority
should therefore use concrete sequence families rather than routine size alone.

## Ranked opportunities

| Status | Opportunity | Current evidence | Expected static impact |
| --- | --- | --- | ---: |
| Done | Elide unobserved public-ABI shadow arguments | 601 measured TN bytes removed | Implemented |
| 1 | Select scaled `(zp),Y` for word elements | 34 genuine `ASL/PHP/.../PLP` sites remain | About 96-136 bytes |
| 2 | Fuse direct indexed byte read/modify/write | Two full-address `tagged(winnum) +/- 1` sites in `Tag` | Roughly 50 bytes |
| 3 | Finish compare and lane-demand combines | 10 blocked byte-binary compares and four unused lanes | Tens of bytes across narrow slices |
| 4 | Summarize routine scratch-pair clobbers | 10 prepared call-result addresses rejected only for conservative clobbering | Tens of bytes |
| 5 | Select direct byte/word increment forms | Four exact load/add-one/store sites | About 26 bytes |
| 6 | Improve routine block placement | 35 converged far veneers; none currently relative-reachable | Up to 105 gross, substantially less realistic |

### Completed: elide unobserved public-ABI shadow arguments

Before this slice, calls to public current-location routines mirrored the first
three argument bytes into `$A0`, `$A1`, and `$A2` and also placed those bytes in
A, X, and Y. TN contained 205 such shadow lanes:

| Callee | Shadow lanes | Calls |
| --- | ---: | ---: |
| `Relpos` | 72 | 36 |
| `Block` | 30 | 10 |
| `Xio` | 21 | 7 |
| `MovePage` | 14 | 7 |
| `Position` | 10 | 5 |
| `Internal` | 8 | 8 |
| `PutImage` | 8 | 4 |
| `Open` | 8 | 4 |
| `Close` | 7 | 7 |
| Remaining eight known library routines | 27 | 15 |
| **Total** | **205** | **103** |

The busiest callers are `Copy`, `InputLine`, `Inv`, and `SetWin` with 15 lanes
each; `SwapScr` has 14, `DrawWinFrame` 13, and `Xloop` 11.

Every mirror store is a two-byte zero-page `STA`, establishing a 410-byte store
floor. Most mirrors also require an immediate or memory load that is repeated
when the register argument is placed. `Inv` is a useful closed example: its 15
mirror lanes explain approximately 63 of its 64-byte routine difference.

This cannot be a blanket public-ABI relaxation. A current-location routine may
legally read `$A0-$A2` through absolute aliases, as covered by the existing
`Capture` regression. The implemented optimization is a conservative
per-callee, per-lane demand proof in MIR6502:

1. accept only known direct calls to public current-location routines;
2. decode a single structured machine block from its entry and track direct
   `$A0-$A2` reads and definite writes;
3. omit a mirror only when the entry overwrites it before observation, or
   reaches `RTS` without observing it;
4. stop and retain every still-live mirror at calls, jumps, branches, indirect
   reads, undecodable bytes, non-machine bodies, and unknown symbols.

The transformation removes only leading shadow homes from the direct call plan
and rebuilds its ABI home list; the primary A/X/Y arguments are unchanged. The
existing high-level `Capture` regression and a raw-machine `$A0` reader both
remain mirrored.

On TN, `LDA` falls from 1,529 to 1,410 and `STA` from 1,187 to 1,019. Their
combined static footprint falls by 606 bytes; minor layout changes leave the
net load-file reduction at 601 bytes. The remaining mirrors are concentrated
in `Block` and the CIO wrappers, where calls, branches, pointer access, or an
actual shadow read prevent this entry-prefix proof. Splitting partially needed
word shadow homes could remove a few more stores, but it requires explicit word
lane representation and is no longer the top opportunity.

The proof and transformation remain entirely in MIR6502 call/ABI planning; no
target-specific ABI facts were added to NIR.

### 1. Select scaled `(zp),Y` for word elements

All 34 generic full-address word-index sequences remain in MIR6502 output. Each
scales the index, saves its carry with `PHP`, adds both bytes to the base,
restores carry with `PLP`, writes a zero-page pointer, and then starts the
consumer at `Y=0`.

The already validated classic strategy keeps `low(2*index)` in Y, propagates
the scale carry into the pointer high byte, and consumes through `(zp),Y`.
The sites remain concentrated in `Sort`, `Copy`, `Handle`, `SetWin`, `Xloop`,
and `Draw`.

The MIR6502 implementation must remain consumer- and flag-aware. It should
reuse the classic proof boundaries for index range, carry propagation, Y
lifetime, volatile access order, and two-address consumers.

### 2. Fuse direct indexed byte read/modify/write

`Tag` contains the only two remaining scale-one `MaterializeIndexedAddress`
operations. They implement:

```action
tagged(winnum)-=1
tagged(winnum)+=1
```

Each branch currently:

1. constructs `tagged + winnum` in a zero-page pointer;
2. loads through `(zp),Y`;
3. stores the value in a temporary;
4. reconstructs the base pointer;
5. reloads the index and temporary;
6. performs add/subtract and stores indirectly.

The direct fixed base permits the much shorter target strategy:

```asm
LDX winnum
DEC tagged,X       ; or INC tagged,X
```

The combine should match a same-address load, add/subtract one, and store; prove
the direct base and identical index; and require any incompatible arithmetic
flags to be dead. It should generalize beyond TN.

### 3. Finish compare and lane-demand combines

The current MIR6502 census reports 10 byte-binary compare candidates and all 10
are blocked only because one or both binary operands remain temps:

| Routine | Blocked candidates |
| --- | ---: |
| `Copy` | 3 |
| `DrawWinFrame` | 2 |
| `Handle` | 2 |
| `FindNext` | 1 |
| `GetAnyKey` | 1 |
| `Sort` | 1 |

Resolve unique, dominance-safe temp operands through their local producer
chains before applying the existing binary-to-compare combine. This is a
better next home-reduction slice than more physical-home coloring.

There are also four unused residual lanes. One visible example is `Tag`, where
a byte value is widened to CARD for XOR with `$7F`; MIR emits the unused high
lane before passing only the low byte to `Store`. Lane demand should be
consulted before word expansion so an unused high lane is never emitted.

`Draw` exposes a related carry-aware opportunity. NIR correctly retains the
CARD meaning of `BYTE + $10`, but MIR currently constructs and spills both word
lanes before comparison with a byte. A 6502-specific compare fusion can branch
on addition carry for the nonzero-high case and compare the low byte otherwise.
This does not require weakening NIR types.

### 4. Summarize routine scratch-pair clobbers

Ten call-result store-address preparations are rejected only because a direct
routine call is conservatively considered capable of clobbering the prepared
fixed pointer pair:

| Routine | Candidates blocked by clobbering |
| --- | ---: |
| `SetWin` | 6 |
| `Convert` | 2 |
| `Init` | 1 |
| `Putchar` | 1 |

A transitive MIR6502 routine summary for fixed scratch-pair reads and writes
would let known direct calls preserve a prepared destination when safe. Unknown
or recursive summaries must converge conservatively, and external/indirect
calls must retain the current fallback.

### 5. Select direct increment forms

Four exact `LDA; CLC; ADC #1; STA` families remain:

- two word-pointer increments in `Strcat`;
- one direct byte increment in `Handle`;
- one direct byte increment in `InitPanels`.

The byte sites can use `INC mem` when ADC carry and overflow are dead. The word
sites can use `INC low; BNE done; INC high` when their final flag contract
allows it. The expected total saving is about 26 bytes.

## Residual homes are no longer the first target

TN currently allocates 106 physical temp-home cells:

| Home kind | Cells |
| --- | ---: |
| Virtual zero page | 74 |
| RAM spill | 32 |

They cause 111 stores and 159 reloads. The largest remaining routine-level
traffic is:

| Routine | Stores + reloads |
| --- | ---: |
| `SetWin` | 49 |
| `Strcat` | 29 |
| `Handle` | 26 |
| `Sort` | 20 |
| `Copy` | 17 |

The previous routine-wide coloring slice saved only eight TN bytes. More home
sharing may reduce storage cells but usually cannot remove the instructions
that access them. The remaining homes are predominantly multi-use, call-live,
coupled word lanes, or loop-live values. The compare, lane-demand, addressing,
and call-result combines above attack their causes instead.

## Branches and tail calls

Forward branch relaxation has converged. The listing contains 35 genuine
branch-over-`JMP` veneers, including the indirect `_Cio` veneer, and none of
their final targets fit a relative branch in the current layout. Further local
relaxation cannot help.

Routine block placement could make some targets reachable. `Handle` and `Copy`
contain most of the veneers, but moving blocks can lengthen other edges. The
105-byte gross ceiling assumes every veneer disappears and is not realistic.

The listing-quality report also finds four adjacent `JSR; RTS` instruction
pairs in `Delete`, `Attrib`, `InitPanels`, and `NavError`. In every case the RTS
is also the target of another path. Replacing the JSR with a tail `JMP` can save
stack traffic and cycles, but cannot remove the shared RTS and therefore saves
no static bytes.

## Recommended implementation order

1. Port the proven scaled `(zp),Y` word-element strategy to MIR6502.
2. Add the direct indexed byte `INC/DEC` read/modify/write combine.
3. Resolve the 10 blocked byte-binary compare operands and suppress unused word
   lanes before expansion.
4. Add transitive routine scratch-pair clobber summaries for prepared
   call-result addresses.
5. Fold the four remaining direct increment sequences.
6. Revisit block placement only after the instruction-generating work above
   changes routine layouts.

Broad NIR work is not indicated by this listing. The largest opportunities are
6502 ABI placement, addressing selection, flag-aware combining, and
demand-driven byte/word expansion, all of which belong in MIR6502.
