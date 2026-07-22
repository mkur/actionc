# TN MIR6502 listing reanalysis

Date: 2026-07-22.

Historical revision: `06e4f23` (`mir6502: elide unobserved public ABI shadow
stores`). Reanalysis baseline revision: `43992fc` (`test: cover canonical
Action ABI boundaries`).

Correction: the original compiler does not mirror argument bytes 0-2 from
A/X/Y into `$A0-$A2`, including for current-location `=*` routines. The
remaining homes described below are lowering defects, not conservative ABI
requirements. Historical sizes and counts are retained; the corresponding
recommendation is corrected in place.

Scope: `samples/tn/modern/TN.ACT`, modern profile, with the MIR6502 backend
compared directionally with the modern/classic backend.

## Current reanalysis after logical-condition CFG lowering

Measured revision: `ceecf13` (`nir: lower logical loop conditions to CFG`).

The MIR6502 load file is now 10,854 bytes. Modern/classic remains 10,445
bytes, leaving a 409-byte gap, or 3.9 percent. Deferred local-array storage and
logical-condition CFG lowering reduced the previous 11,554-byte result by 700
bytes in total.

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| Load file | 10,854 | 10,445 | +409 |
| Recognized instructions | 4,476 | 4,338 | +138 |
| Recognized instruction bytes | 10,130 | 9,714 | +416 |
| `LDA` + `STA` instructions | 2,246 | 1,910 | +336 |
| `LDA` + `STA` instruction share | 50.2% | 44.0% | +6.2 points |
| `JMP` | 161 | 158 | +3 |
| `JSR` | 369 | 368 | +1 |
| Branch-over-`JMP` veneers | 30 | 28 | +2 |

The listing-quality parser undercounts long `.BYTE` declarations and may
interpret bytes in machine/data procedures as instructions. XEX sizes are
authoritative. After deferred storage and logical-CFG home removal, an
independent label-based data census finds 762 declared data bytes in MIR6502
and 970 in classic. MIR6502's remaining load-file excess is therefore entirely
in code and layout, partly offset by its smaller loadable data region.

Current generated artifacts:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `TN-pre.mir` | 130,090 | `58f8a8488af6086e88e6652bdd7ee569fa7b7154d216f89850263b2e727e9207` |
| `TN-materialized.mir` | 156,561 | `206b4d4a484faa1775f0f9599f2c52855690b456af04c0db8653e1feb45a0838` |
| `TN-mir6502.lst` | 143,552 | `7f649cee52108afb2bad5b4ee3b6038d46998ebe2596fad4ad43f5168de81f3e` |
| `TN-mir6502.map` | 10,967 | `d91ddcb8321fb42efa6bdc7943c08c7d4a8775b7482b509777e6c21b728dcf34` |
| `TN-mir6502.peepholes` | 294,595 | `9f38447a6f1d26110a7fa8404720ce10cf1a45bf315b4f131bbe120410121359` |
| `TN-mir6502.xex` | 10,854 | `55c6f65fb6a147f5f29081d5f6c1212103698bc07806dbbd91f2d4213e5625b5` |
| `TN-classic.xex` | 10,445 | `3caefd677ab3d1489e39fcc0200126b442a15278b26a9cb5351434a1c8674f39` |

### Current ranked opportunities

#### 1. Defer uninitialized local-array backing — completed

The initial audit identified these four `SetWin` objects:

| Backing object | Bytes |
| --- | ---: |
| `lp_PathBuf` | 47 |
| `rp_PathBuf` | 47 |
| `lp_v` | 130 |
| `rp_v` | 130 |
| Subtotal | 354 |

Classic excludes this uninitialized backing from the saved image. MIR6502
already had a `DeferredData` segment and used it for larger arrays, but its size
policy left these objects in `LoadData`. That made 354 bytes the initial bounded
estimate. Descriptors and address initialization remain loadable; only backing
whose contents are not initialized by Action semantics is deferred.

Implemented by `c1cbc8f` (structured MIR storage classes) and `f45dcff`
(deferred placement). The structured classification exposed another 104 bytes
that the listing audit had omitted:

| Routine and backing | Bytes |
| --- | ---: |
| `_StrNam.fnam` | 40 |
| `SetWin.leftPanel`, `SetWin.rightPanel` | 16 |
| `Copy.lentab`, `Copy.copytab` | 48 |
| Additional subtotal | 104 |

All nine previously inline uninitialized local arrays now join the two existing
1,171-byte deferred `SetWin` directory buffers. The MIR6502 XEX fell from
11,554 to 11,096 bytes, an exact 458-byte reduction with no code-size drift.
The gap to the 10,445-byte classic output is now 651 bytes, or 6.2 percent.
The resulting XEX SHA-256 is
`8f5366e52b233038ec20eb2d0df28f65b310a8a1be4d747a2edc4a255a932e0e`.

#### 2. Preserve logical conditions as NIR control flow — completed

Eleven surviving compare-to-binary lanes are concentrated in `Handle` (six)
and `Copy` (five). They are not ordinary arithmetic residue: nested `AND` and
`OR` conditions are lowered to byte boolean values, stored in homes, combined,
and finally compared with zero.

The comparable classic code branches directly. The inspected sequences account
for approximately 86 bytes in `Handle` and 70 bytes in `Copy` beyond the direct
branch shapes. A realistic target is 130-160 bytes after allowing for labels
and paths shared with surrounding code.

SemIR retains logical structure and source evaluation order. NIR condition
lowering should turn that structure into explicit short-circuit blocks before
MIR expansion, including call-containing conditions. MIR6502 should not move
calls or reconstruct source-language short-circuit meaning from flattened
boolean arithmetic.

Implemented in three semantic slices: `c2445d5` lowers two-term logical `IF`
conditions, `75f63d3` recursively lowers mixed and nested trees while keeping
calls in reached right-hand blocks, and `ceecf13` applies the same lowering to
`WHILE` and `DO`/`UNTIL`.

The result exceeded the 130-160-byte estimate. TN fell from 11,096 to 10,854
bytes, an exact 242-byte reduction. The matched routine ranges account for 230
bytes: `Handle` -102, `Copy` -92, `Convert` -12, `InputLine` -9, `SetWin` -9,
and `NewDrive` and `SwapScr` -3 each. Removing transient homes accounts for the
other 12 loadable bytes. Recognized instructions fell by 91 and `JMP` by 32.

Telemetry confirms that all eleven compare-to-binary lanes are gone. Final
temp homes fall from 90 (59 ZP, 31 RAM) to 77 (58 ZP, 19 RAM), and emitted
spill labels fall from 26 to 17. The broader gain comes from lowering all
eligible conditions, not only the initially counted `Handle` and `Copy`
sequences, and from exposing direct branch layout plus dead homes.

#### 3. Fuse word and pointer carry chains into final locations

`Key`, `Next`, and `Strcat` still build word intermediates in homes and then
copy them into pointer pairs, return locations, or call argument locations.
Their current matched-range differences are +27, +22, and +24 bytes
respectively. Selecting the complete low/high carry chain into its final
physical destination should recover approximately 75-80 bytes across this
family.

This is the next MIR6502 instruction-selection extension after indexed
word-plus-constant call arguments. It must preserve the low-to-high carry edge
as one coupled operation rather than independently optimizing byte lanes.

#### 4. Retain prepared pointers and known result locations

`Range` is 42 bytes larger than classic and repeatedly reloads pointer pairs
around indirect comparisons and read/modify/write paths. Path-sensitive
location facts should retain a prepared pointer when all relevant paths and
effects preserve it.

There are also five known call-result sites in `Putchar` and `SetWin` where the
callee contract says `A=$A0`, but generated code reloads `$A0` before storing
the result. Consuming the declared result location should save about 10 bytes.
This is known-call result-state propagation, not caller-side `$A0-$A2`
argument shadowing.

The combined realistic opportunity is about 40-50 bytes.

#### 5. Extend multi-use value-location planning

The final materialization census has 90 temp homes: 59 in zero page and 31 in
RAM. It emits 77 ZP stores and 87 reloads, plus 27 RAM stores and 57 reloads.
The encoded access traffic plus RAM backing occupies about 611 bytes, but this
is a gross burden rather than an achievable saving: many values genuinely
cross calls, joins, or register clobbers.

After the structural work above, rerun the census and plan multi-use values
against A, X, Y, pointer pairs, final ABI locations, and homes. Producer and
next-consumer constraints should choose the location; a general register
allocator should not be the first response to values that need not have been
materialized or byte-split.

### Current lower-priority findings

- Three remaining binary-to-compare origins are home-free numeric bitwise
  conditions and should be treated as closed. The eleven compare-to-binary
  logical lanes are eliminated.
- Scaled `(zp),Y` selection applies at 30 sites. Only two distinct blocked
  sites remain (`MakeJmp`, flags live; `Handle`, home live).
- There are 30 branch-over-`JMP` veneers, but none currently has a
  branch-reachable final target. Revisit placement after code-generating
  optimizations move boundaries.
- Cross-routine RAM-home pooling has a maximum static backing benefit of 31
  bytes and does not remove access instructions. Home creation remains the
  higher priority.
- Twenty-eight adjacent same-home `STA`/`LDA` pairs remain versus sixteen in
  classic. The excess is useful cleanup evidence, not a leading target.
- Four `JSR; RTS` pairs remain in both backends and do not explain the gap.

After the completed 458-byte deferred-storage and 242-byte logical-CFG results,
the remaining gap is 409 bytes. The word/pointer carry-chain opportunity is now
the leading bounded target at roughly 75-80 bytes; all estimates should be
remeasured against the new control-flow layout.

## Previous reanalysis baseline after scaled-Y and ABI correction

At that revision, the MIR6502 load file is 11,706 bytes. Modern/classic remains
10,445 bytes, leaving a 1,261-byte gap. Removing the invented caller homes
saved 249 bytes from the 11,955-byte scaled-Y baseline; together, scaled-Y
selection and the ABI correction saved 392 bytes from the 12,098-byte
historical result below.

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| Load file | 11,706 | 10,445 | +1,261 |
| Recognized instructions | 4,710 | 4,338 | +372 |
| Recognized instruction bytes | 10,703 | 9,714 | +989 |
| Recognized data bytes | 423 | 285 | +138 |
| `LDA` + `STA` instructions | 2,340 | 1,910 | +430 |
| `LDA` + `STA` instruction share | 49.7% | 44.0% | +5.7 points |
| `JMP` | 194 | 158 | +36 |
| `JSR` | 369 | 368 | +1 |
| Branch-over-`JMP` veneers | 33 | 28 | +5 |

Pre-materialized MIR has 342 call operations and zero `$A0-$A2` call-home
mentions. The authored machine records are byte-identical to the pre-fix MIR,
including 62 intentional `$A0-$A2` scratch-address mentions. The corrected
load-file SHA-256 is
`77f2c1a7374fbb5e936e8019784e2e86bb6789bb71dd31d94deb0d3b81ae5526`.

Artifacts at that revision:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `TN-pre.mir` | 130,693 | `80e250efaf287ac48057f14eaf89b395373ab4eac3beb64301070b841492b372` |
| `TN-materialized.mir` | 158,517 | `751500aa6873b540fbf6950e39414463854b12919c90d4a8c18dcbe988943e2e` |
| `TN.lst` | 153,158 | `0496fcd334ad1d4db1388ad28f6d1ad423d05fc2c3e735fe9b1e3820b9268854` |
| `TN.map` | 10,967 | `d3f63f256b7b937d3613da2d94a294864469feb9386ee06a3b6e195bf8cf5088` |
| `TN.peepholes` | 306,825 | `bf39e465bd827bd2916aed591acadad18466267f5493fd34748f0ff6fb79170b` |
| `TN.quality` | 3,367 | `7bd35781f892af224d906b9faf34f6551a796c3bdfa73b51ed7b35700b0b9e83` |
| `TN.xex` | 11,706 | `77f2c1a7374fbb5e936e8019784e2e86bb6789bb71dd31d94deb0d3b81ae5526` |

## Previous ranked backlog at `43992fc`

The invalid ABI traffic is gone, but the remaining opportunity counts are
otherwise stable. The recommended order is now:

1. Fuse the two direct indexed byte read/modify/write sites in `Tag` into
   indexed `INC`/`DEC`; this remains the clearest roughly 50-byte target.
2. Select dual-indirect byte comparisons in `Fnamecmp`; three sites account
   for roughly 30 bytes and several transient homes.
3. Resolve the 12 `binary-to-compare` residual lanes and suppress the four
   values whose high lanes have no consumer. Completed in `a1c9516`, `71aebf3`,
   and `06699c8`; see the measured result in section 5.
4. Add transitive known-call register/scratch effects for the five prepared
   call-result address candidates in `Putchar` and `SetWin`. This is independent
   of argument placement; unknown and indirect calls remain conservative.
5. Finish the register-live byte increment in `InputLine` and the two word
   pointer increments in `Strcat`.
6. Treat the four residual full scale-two address calculations separately:
   two use different source/destination indexes in `Sort`, one feeds an opaque
   machine block in `MakeJmp`, and one has a wider live window in `Handle`.
7. Revisit block placement only after the instruction-generating changes move
   routine and branch boundaries.

At the revision measured above, the listing still had 28 adjacent `STA`/`LDA`
pairs, 33 far veneers, four `JSR; RTS` pairs, the 12 compare lanes, and five
call-address candidates reported below. The compare and unused-lane work is now
complete as described in section 5. None of these findings justifies
reintroducing caller shadow analysis or another broad NIR phase.

## Historical result before scaled-Y and ABI correction

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

## Historical artifact manifest

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

## Historical opportunity analysis

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

### 2. Remove invented caller-side `$A0-$A2` mirrors

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

All 66 mentions are invalid as Action call homes. Byte offsets 0-2 are passed
only in A, X, and Y; the fixed argument area begins at `$A3`. Current-location
placement changes the entry address, not the argument ABI.

`Block` demonstrates why interpreting its machine-code `$A0` access as caller
demand is wrong. It receives mode/dx/dy in A/X/Y, executes `PHA` and `STX $A0`,
and later reads the value it explicitly saved. The caller must not prepopulate
`$A0`. `Open` and `Xio` similarly execute their own `STX $A1` before reading
that scratch byte. The byte-exact original `STRNAM.COM` callers initialize only
A/X/Y and `$A3+`.

The fix is therefore to remove public/current-location mirroring from
`call_plan`, delete the compensating shadow-demand pass, and make tests require
the canonical A/X/Y then `$A3+` sequence. Machine-code `$A0-$A2` loads and
stores remain unchanged as authored instructions.

Expected impact: removal of all 66 residual call-home mentions. The exact TN
byte reduction must be measured after materialization because register staging
and layout changes make a per-lane estimate imprecise.

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

#### Implementation result

This opportunity was completed in three independent slices:

| Slice | Commit | Change | TN bytes | Raw `binary-to-compare` origins |
| --- | --- | --- | ---: | ---: |
| Baseline | `abb8bbc` | Dual-indirect compare work complete | 11,604 | 12 |
| 1 | `a1c9516` | Fuse direct-load byte binary/compare chains | 11,593 | 8 |
| 2 | `71aebf3` | Narrow carry-aware byte-add/word-compare in `Draw` | 11,561 | 6 |
| 3 | `06699c8` | Prune independently dead high lanes using typed lane demand | 11,554 | 6 |

The total load-file reduction is 50 bytes. Slice 1 resolves the four profitable
load/binary/compare chains in `Sort`, `FindNext`, `Copy`, and `Handle`. Slice 2
removes both coupled `Draw` lanes by branching on addition carry before the
low-byte comparison. Slice 3 suppresses all four reported unused high lanes;
the aggregate `consumer-unused`, `home-demand-retained-unused-lanes`, and
`home-plan-materialize-unused-lanes` counters no longer appear. The visible
savings are three bytes in `Init` and four bytes in `Tag`; the dead result moves
in `DrawWinFrame` and `SwapWin` emitted no machine instructions.

The remaining raw count of six is attribution, not six outstanding homes. It
consists of one origin in `GetAnyKey`, two in `DrawWinFrame`, two in `Copy`, and
one in `Handle`. Every one has final decision `elide-a`, fate `elided-plan`, and
zero stores and reloads. They are already accumulator-selected and should not
be targeted merely to make the pre-plan origin counter reach zero. The final
load-file SHA-256 is
`96f0456220d5fe53bfc82ea8969c32a2bcb8f4794d8f38f5c8e6582e81a0ae5f`.

### 6. Reuse known-call summaries for prepared addresses

Only five call-result effective-address preservation candidates remain, down
from ten in the previous analysis: one in `Putchar` and four in `SetWin`. All
five are rejected because a known direct call is still conservatively assumed
to clobber the prepared pointer pair.

This remains a separate routine-effect problem. Known direct-call summaries
can answer it, while unknown, recursive, external, and indirect calls retain
the current conservative fallback. It must not be coupled to `$A0-$A2` argument
mirrors, which are absent from the ABI.

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

## Historical recommended implementation order

This list is superseded by the updated ranked backlog above. Its completed
scaled-Y and ABI entries are retained to explain the measurements and design
decisions that led to the current result.

1. Port scaled `(zp),Y` word-element selection to MIR6502.
2. Remove caller-side `$A0-$A2` shadow generation and its compensating demand
   analysis; retain canonical A/X/Y and `$A3+` argument placement.
3. Add the direct indexed byte `INC/DEC` combine.
4. Add dual-indirect byte compare selection.
5. Resolve binary-to-compare producer chains and suppress unused word lanes.
6. Finish the remaining direct byte/word increment forms.
7. Revisit block placement only after these instruction-generating changes
   alter routine layouts.

The listing does not indicate a need for another NIR-wide optimization phase
or another rewrite-framework refactor. The leading work is target-specific
addressing, the MIR6502 ABI correction, independent call-effect summaries, and
consumer selection.
