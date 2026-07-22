# TN MIR6502 listing reanalysis

Date: 2026-07-22.

Revision: `06e4f23` (`mir6502: elide unobserved public ABI shadow stores`).

Scope: `samples/tn/modern/TN.ACT`, modern profile, with the MIR6502 backend
compared directionally with the modern/classic backend.

## Result

The clean MIR6502 load file is 12,098 bytes. Modern/classic emits 10,445
bytes, leaving a 1,653-byte gap. MIR6502 is four bytes smaller than the 12,102
bytes recorded by the earlier public-ABI analysis.

The source hash is
`097df477534d50b9aaec1d733b8d6a66f6792e00cd7703e46331c2d5425f8797`.
The `Cargo.lock` hash is
`02e7e9e564916b19fe9aad0dc7d2efd44fe9fc58cf56132022ebddbbc4a754fd`.

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| Load file | 12,098 | 10,445 | +1,653 |
| Recognized instructions | 4,952 | 4,338 | +614 |
| Recognized instruction bytes | 11,094 | 9,714 | +1,380 |
| Recognized data bytes | 424 | 285 | +139 |
| `LDA` + `STA` instructions | 2,426 | 1,910 | +516 |
| `LDA` + `STA` instruction share | 49.0% | 44.0% | +5.0 points |
| `JMP` | 197 | 158 | +39 |
| `JSR` | 369 | 368 | +1 |
| Branch-over-`JMP` veneers | 35 | 28 | +7 |

The listing-quality parser may interpret bytes inside machine/data procedures
as instructions. XEX sizes are authoritative; instruction counts are
comparative evidence.

## Reproduction

```sh
mkdir -p target/tn-listing-reanalysis-20260722

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/tn/modern/TN.ACT \
    > target/tn-listing-reanalysis-20260722/TN-mir6502.lst \
    2> target/tn-listing-reanalysis-20260722/TN-mir6502.peepholes

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-listing-reanalysis-20260722/TN-materialized.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-listing-reanalysis-20260722/TN-pre.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-map \
  samples/tn/modern/TN.ACT \
  > target/tn-listing-reanalysis-20260722/TN-mir6502.map

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/tn/modern/TN.ACT \
  > target/tn-listing-reanalysis-20260722/TN-mir6502.xex

cargo run --quiet --bin actionc-listing-quality -- \
  target/tn-listing-reanalysis-20260722/TN-mir6502.lst \
  > target/tn-listing-reanalysis-20260722/TN-mir6502.quality

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-listing \
  samples/tn/modern/TN.ACT \
  > target/tn-listing-reanalysis-20260722/TN-classic.lst

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-load \
  samples/tn/modern/TN.ACT \
  > target/tn-listing-reanalysis-20260722/TN-classic.xex

cargo run --quiet --bin actionc-listing-quality -- \
  target/tn-listing-reanalysis-20260722/TN-classic.lst \
  > target/tn-listing-reanalysis-20260722/TN-classic.quality
```

## Artifact manifest

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `TN-pre.mir` | 132,194 | `616449da61a62e5068d5c1549996b16dce48d69f178af2ce35f1f585e7bb565c` |
| `TN-materialized.mir` | 162,151 | `ef59cf1c7a1737685d02e966bfb5afb1c39d91abbad8786aef1bbe1894efbe61` |
| `TN-mir6502.lst` | 158,764 | `a2769bcaf646a431330473633b28ed5ec7c9a4b3a62cca032fc8a6f3fffc3c8c` |
| `TN-mir6502.map` | 10,967 | `740c10540e25bf1656122e2e12278528652a45e61e9760943964a9e792217803` |
| `TN-mir6502.peepholes` | 308,369 | `1243c9caef09ff6f7ef1e0011b0d1811336dd62cf30ee8bbb8b53212bd17ac3a` |
| `TN-mir6502.quality` | 3,102 | `5178d1cb1d56304cd92eb95016140b576d6e8ff7c6a64bd1d7a7dc09dec12ddb` |
| `TN-mir6502.xex` | 12,098 | `19de06ecd51e49c06edfda9ace7f6a3148f38b8dafabfce301426f6512a42ecd` |
| `TN-classic.lst` | 136,233 | `668fb6ad3376a4449c6d79d2ae3050ca4deb415325002642cc244c9404750552` |
| `TN-classic.quality` | 2,721 | `62cc59f313283e5ac72f27674aa7dd26071c8197a2443ff49c7e49f756a7a794` |
| `TN-classic.xex` | 10,445 | `3caefd677ab3d1489e39fcc0200126b442a15278b26a9cb5351434a1c8674f39` |

## Routine concentration

The largest matched-routine differences are:

| Routine | MIR6502 bytes | Classic bytes | Difference |
| --- | ---: | ---: | ---: |
| `Handle` | 1,038 | 839 | +199 |
| `Copy` | 856 | 723 | +133 |
| `Tag` | 174 | 93 | +81 |
| `Draw` | 198 | 137 | +61 |
| `Fnamecmp` | 235 | 183 | +52 |
| `Xloop` | 266 | 221 | +45 |
| `Range` | 218 | 176 | +42 |
| `View` | 132 | 98 | +34 |
| `InputLine` | 247 | 218 | +29 |
| `Key` | 76 | 49 | +27 |
| `Inv` | 78 | 53 | +25 |
| `Strcat` | 142 | 118 | +24 |
| `Window` | 335 | 311 | +24 |

Routine size alone is not the priority signal. `Handle` and `Copy` contain
several different families, while most of `Tag`'s gap is one bounded indexed
read/modify/write failure.

## Ranked opportunities

### 1. Select scaled `(zp),Y` word-element addressing

MIR6502 still emits 34 `ASL/PHP/.../PLP` full-address calculations. Their
routine distribution is:

| Routine | Sites |
| --- | ---: |
| `Sort` | 6 |
| `Copy`, `Handle`, `SetWin` | 4 each |
| `Xloop` | 3 |
| `Draw` | 2 |
| 11 other routines | 1 each |

The current common sequence computes `base + 2*index` into a pointer pair,
then resets Y to zero. The classic shape leaves `low(2*index)` in Y and adds
the scale carry only to the pointer high byte. A representative conversion
saves five static bytes and roughly fourteen cycles before considering reuse.
Classic emits only three `PHP/PLP` pairs in the whole TN listing, demonstrating
that almost all of these source cases admit the scaled-Y strategy.

The gross 34-site ceiling is about 170 bytes. A more conservative
implementation target is 120-155 bytes after Y/flag lifetime, two-address
consumers, and variant sequence shapes are accounted for. This remains the
largest bounded next slice.

### 2. Extend public-ABI demand through known machine calls

After entry-prefix shadow-store elision, pre-materialized MIR still contains 66
`$A0-$A2` shadow-home mentions across 35 calls:

| Callee | Calls | Shadow-home mentions |
| --- | ---: | ---: |
| `Block` | 10 | 20 |
| `Xio` | 7 | 14 |
| `Open` | 4 | 12 |
| `Close` | 7 | 7 |
| `PutD` | 3 | 3 |
| `Input`, `Bget`, `Bput` | 1 each | 3 each |
| `GetD` | 1 | 1 |

Some are required, but the largest removable group is already visible.
`Block` receives dx/dy in X/Y, stores X into its private working byte, calls
`CalcAdr`, and later consumes Y directly. `CalcAdr` is declared to preserve A,
X, and Y and does not read `$A1/$A2`. The current local proof stops at its
`JSR`, so all 20 `$A1/$A2` lanes are retained; classic emits none of those
mirrors. These lanes commonly cost a duplicate load plus a zero-page store,
making `Block` alone an approximately 80-byte opportunity.

Known-call machine-effect summaries should be transitive and conservative at
recursion, indirect calls, unresolved targets, and incomplete machine blocks.
Per-byte splitting of word shadow homes is a smaller companion improvement:
several CIO wrappers overwrite one byte of a pair before reading only the
other byte.

Expected impact: roughly 80 bytes from the proven `Block` group, with more
available after the CIO cases are proved lane by lane.

### 3. Fuse direct indexed byte read/modify/write

`Tag` still implements the two branches of `tagged(winnum)-=1` and
`tagged(winnum)+=1` by:

1. constructing `tagged + winnum` in a zero-page pointer;
2. loading indirectly and spilling the byte;
3. reconstructing the fixed base pointer;
4. reloading the index and spilled byte;
5. subtracting or adding one and storing indirectly.

The fixed base and identical byte index permit:

```asm
LDX winnum
DEC tagged,X       ; or INC tagged,X
```

The two branches account for approximately 60-65 bytes of `Tag`'s 81-byte
gap. The combine belongs in MIR6502 and must prove base/index identity plus the
required register and final-flag contract.

### 4. Select dual-indirect byte comparisons

`Fnamecmp` is 52 bytes larger than classic. Three times, MIR6502 prepares one
pointer pair, loads and spills a byte, retargets the same pair, loads and
spills the other byte, then reloads both spills for comparison.

Classic retains both addresses in `$AC/$AD` and `$AE/$AF` and compares the
bytes directly:

```asm
LDY #$00
LDA ($AE),Y
EOR ($AC),Y        ; equality
```

The ordered case uses `CMP ($AC),Y`. A two-address compare selector with pair
liveness and memory-order proofs should save about 30 bytes across the three
sites and remove their transient byte homes.

### 5. Finish compare and lane-demand combines

Current telemetry reports 12 `binary-to-compare` residual lanes:

| Routine | Lanes |
| --- | ---: |
| `Copy` | 3 |
| `Draw`, `DrawWinFrame`, `Handle` | 2 each |
| `FindNext`, `GetAnyKey`, `Sort` | 1 each |

It also reports four lanes materialized despite having no consumer, in
`Init`, `DrawWinFrame`, `Tag`, and `SwapWin`. `Tag` visibly constructs the high
lane of a widened XOR and overwrites A without using it. Resolve unique,
dominance-safe producer chains before compare selection and consult lane demand
before word expansion.

Expected impact: tens of bytes across narrow, independently testable slices.

### 6. Reuse known-call summaries for prepared addresses

Only five call-result effective-address preservation candidates remain, down
from ten in the previous analysis: one in `Putchar` and four in `SetWin`. All
five are rejected because a known direct call is still conservatively assumed
to clobber the prepared pointer pair.

The transitive routine-effect summaries needed by opportunity 2 should also
answer this proof. Unknown, recursive, external, and indirect calls retain the
current conservative fallback.

### 7. Finish the remaining increment forms

The direct `Handle` and `InitPanels` byte increments are now selected, as shown
by two `direct-inc-dec-update` applications. Remaining work is one
register-live `InputLine` byte case and two word-pointer increments in
`Strcat`. A profitable replacement may reload A when its value, but not its
carry, remains live after a byte increment.

This is now a small cleanup slice rather than a leading opportunity.

## Lower-priority observations

The 35 MIR6502 branch-over-`JMP` veneers have no currently relative-reachable
targets. Local branch relaxation is exhausted. Block placement could move some
targets into range, but the 105-byte gross ceiling assumes every veneer
disappears and is unrealistic.

The current home plan has 105 physical temp-home cells: 72 virtual zero-page
and 33 RAM. They receive 119 stores and 159 reloads. The remaining lanes are
mostly call-live, coupled, multi-use, terminator-live, or tied to unsupported
consumers. Earlier coloring work showed that sharing homes saves storage more
readily than instructions; the producer/consumer and addressing selections
above attack the causes of traffic instead.

Post-home telemetry records 320 blocked rewrites: 289 live home definitions
and 31 live registers. No blocker indicates an alias/effects precision failure.
These stores must not be deleted merely to recover size; their upstream value
shapes should be improved instead.

Four adjacent `JSR; RTS` pairs remain, but each RTS is also reached by another
path. Tail-jumping would improve cycles and stack traffic without saving a
static byte.

## Recommended implementation order

1. Port scaled `(zp),Y` word-element selection to MIR6502.
2. Add transitive known-machine-call effects, first recovering `Block` shadow
   lanes and then reusing the summary for prepared-address preservation.
3. Add the direct indexed byte `INC/DEC` combine.
4. Add dual-indirect byte compare selection.
5. Resolve binary-to-compare producer chains and suppress unused word lanes.
6. Finish the remaining direct byte/word increment forms.
7. Revisit block placement only after these instruction-generating changes
   alter routine layouts.

The listing does not indicate a need for another NIR-wide optimization phase
or another rewrite-framework refactor. The leading work is target-specific
addressing, ABI/effect summarization, and consumer selection in MIR6502.
