# TN MIR6502 post-Slice-6 final-listing audit

Date: 2026-07-25.

Revision: `466332a` (`mir6502: improve call and pointer destination placement`).

Scope: `samples/tn/modern/TN.ACT`, modern profile, comparing MIR6502 with the
modern/classic backend.

## Result

The post-Slice-6 artifacts are reproducible. MIR6502 remains 155 bytes smaller
than modern/classic overall:

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| XEX bytes | 10,290 | 10,445 | -155 |
| Main segment | 10,278 | 10,433 | -155 |
| Instruction rows | 4,221 | 4,236 | -15 |
| Code bytes | 9,416 | 9,408 | +8 |
| Data bytes | 862 | 1,025 | -163 |
| `LDA` + `STA` instructions | 2,023 | 1,910 | +113 |
| `LDA` + `STA` bytes | 4,754 | 4,562 | +192 |
| `LDA` + `STA` instruction share | 47.9% | 45.1% | +2.8 points |
| `LDA` + `STA` code-byte share | 50.5% | 48.5% | +2.0 points |

Listing accounting is exact:

- MIR6502: 9,416 code bytes + 862 data bytes = 10,278 main-segment
  bytes.
- Modern/classic: 9,408 code bytes + 1,025 data bytes = 10,433
  main-segment bytes.

Across 103 matched routine ranges, MIR6502 code is 41 bytes smaller. Its
MIR6502-only `<program>` range is 49 unreachable code bytes produced by
incorrectly lowering compile-time `SET` directives as executable stores. That
dead range accounts exactly for the final eight-byte code deficit.

The matched routine differences split into 214 positive bytes and 255 negative
bytes. The largest remaining positive differences are:

| Routine | MIR6502 code | Classic code | Difference |
| --- | ---: | ---: | ---: |
| `Handle` | 847 | 819 | +28 |
| `Window` | 321 | 301 | +20 |
| `PopUp` | 272 | 253 | +19 |
| `Draw` | 153 | 136 | +17 |
| `MakeJmp` | 45 | 34 | +11 |
| `Tag` | 99 | 91 | +8 |
| `Ord` | 77 | 69 | +8 |
| `Copy` | 668 | 660 | +8 |
| `SwapScr` | 198 | 190 | +8 |
| `Path` | 82 | 75 | +7 |

The largest MIR6502 wins are `SetWin` (-43), `Init` (-41), `Convert` (-33),
`InputLine` (-13), `Rename` (-12), and `Delete` (-11). The remaining work is
therefore concentrated; it is not a general whole-program regression.

## Residual pattern census

### Known-callee word results are copied low-byte first

There are 15 exact sequences of this form:

```text
call known word-returning routine
load $A0
store destination low
load $A1
store destination high
```

They occur in `Window`, `CloseWindow`, `Ord`, `Items`, `FindItem`, `DrawMenu`,
`MoveMenuBar`, four sites in `PopUp`, `Format`, two sites in `SwapScr`, and
`MkDir`.

For these known callees, routine exit-state analysis already proves that `A`
contains the `$A1` high lane at return. Emitting:

```text
store destination high from A
load $A0
store destination low
```

removes one two-byte `LDA $A1` at each site. The exact initial ceiling is 30
static bytes. This should extend the existing known-callee exit-state and
destination-placement workflow, with physical-alias and store-order proofs.

Nineteen calls in total are followed immediately by `LDA $A0`; the other four
sites include truth testing or non-canonical result consumption and should be
handled separately.

### Inclusive unsigned comparisons use two branches

The final MIR6502 listing contains 22 adjacent conditional-branch pairs
produced by inclusive unsigned comparisons:

```text
CMP rhs
BCC true
BNE false
```

Ten compare against constants below 255. They can use the equivalent
single-branch form `value < constant + 1`, saving two bytes each without range
assumptions. The ten sites are in `Init` (2), `DrawWinFrame` (1), `SetWin` (5),
`SwapScr` (1), and `Handle` (1).

The remaining direct-memory comparisons may use operand reversal when loading
the other operand into `A` is legal and profitable. `Window` alone has three
such loop headers. The total upper ceiling is 44 bytes; the constant-only first
slice has an exact 20-byte ceiling.

### Four-byte SArgs routines can use a shadow frame

TN has 11 SArgs routines:

| Argument bytes | Routines |
| ---: | ---: |
| 3 | 3 |
| 4 | 7 |
| 5 | 1 |

The seven four-byte routines are `Window`, `InputLine`, `Ord`, `PopUp`,
`Strcpy`, `Strcat`, and `Fnamecmp`.

For a proven-safe lifetime, a four-byte callee can replace the six-byte SArgs
call and metadata with:

```text
STA $A0
STX $A1
STY $A2
```

Argument byte three already resides in `$A3`. This prologue is also six bytes,
does not clobber registers or flags, avoids the helper loop, and permits
zero-page parameter accesses without private parameter backing.

The initial promising TN cases are:

- all four incoming bytes in `Window`;
- all four incoming bytes in `Strcpy`;
- a partial shadow in `PopUp`;
- the read-only `item` bytes in `Ord`.

`InputLine` and `Fnamecmp` modify relevant parameters. `Strcat` needs its
parameters after `$A0-$A3` have been repurposed for an outgoing call.

The conservative static estimate is 20-30 bytes, plus removal of one SArgs loop
execution per call. Eligibility must be proved per byte using physical
`$A0-$A3` liveness, known-callee effects, address escape, and writes; source
read-only or leaf status alone is insufficient.

### Indexed word placement still has two small gaps

Two final sequences, one in `Draw` and one in `Handle`, load an indexed word
low byte first, stage it through `$A0`, load the high byte into `X`, and reload
the low byte into `A`. Loading the high byte first, transferring it to `X`,
then loading the low byte removes the `$A0` store/reload pair. The exact ceiling
is six bytes.

`MakeJmp` has the one remaining scaled-`(zp),Y` candidate blocked solely by
live carry. Its computed `(c - 1) * 2` index can retain the `ASL` result in `Y`
while propagating the `ASL` carry through `LDA`/`STA` into `ADC #0` for the
pointer high byte. The current routine also contains a duplicate high-lane
store and a dead accumulator load before a machine block whose structured
effects read no registers. Together these account for most of its +11-byte
difference.

### CFG and condition-code provenance remain visible

`PopUp` still routes a `FindItem` result through a late truth-test block:

```text
LDA $A0
JMP test
...
test:
LDA $A0
BEQ ...
```

The second load is value-redundant, but its Z/N result is currently considered
live. Carrying exact flag provenance with the edge accumulator fact would
allow the earlier load's flags to feed the branch. Better block placement
could also remove the jump.

`Draw` has an appended block cluster for its initial bounded-end calculation,
and `Handle` has several similar long control-flow paths. Branch relaxation is
otherwise exhausted: the listing has 28 branch-over-`JMP` patterns and none
has an in-range final target. The next control-flow work must improve block
selection/layout or flag provenance rather than rerun displacement relaxation.

### Residual homes are no longer the dominant backing problem

Final home telemetry reports:

| Home class | Cells | Stores | Reloads |
| --- | ---: | ---: | ---: |
| Private zero page | 41 | 42 | 43 |
| RAM spill | 13 | 18 | 20 |

The listing contains 11 spill data labels and 21 adjacent `STA m; LDA m`
pairs. Most adjacent pairs are loop-entry joins, where the backedge still needs
the load; they are not safe block-local deletions.

The more useful gap is definition-sensitive memory liveness. For example,
`Window` retains a final `STX` to a spill after the reload was removed, and
`MakeJmp` retains traffic associated with only one of several definitions of
the same home. Current rejection telemetry reports many candidates as
`home-definition-live` because liveness is still coarse at the home level.
Any follow-up should prove an individual store definition dead rather than
weakening whole-home checks.

### The apparent program initializer is dead compile-time `SET` code

The MIR6502-only `<program>` range is 49 bytes of absolute stores produced from
source `SET` operations, but it is not a startup initializer:

- TN's RUNAD is `$53C4` (`NavInit`), not `$2F18` (`<program>`);
- the final listing contains no call or jump to `$2F18`;
- a focused `SET $E=...`/`SET $F=...` probe likewise emits `<program>` before
  `Main` while RUNAD points directly to `Main`;
- modern/classic applies the same `SET` directives while laying out the object
  and emits no executable stores.

The original compiler evidence also classifies these operations as
compile-time mutations. `SET $0E` and `SET $0491` change compiler code/storage
cursors, `SET $04E4..$04EE` changes the compiler's helper-target table, and
`SET symbol=*` patches generated storage after layout. The latter is already
represented correctly by NIR `ProgramEndWord`.

The bug is in `NirLowerer::program`: after applying compatible layout effects,
it still sends every non-helper `SET` through `set_op`, which converts a
numeric address into an executable absolute `Store`. The verifier has encoded
the same mistaken contract by requiring legacy `SET` operations to become
absolute stores. MIR6502 then faithfully emits those stores in a synthetic
routine that normal run-address selection never enters.

The narrow TN-safe correction is to consume compatible compiler-control
`SET`s during lowering and stop adding their executable stores. A post-fix
build confirms that it removes the complete 49-byte range and reduces the XEX
from 10,290 to 10,241 bytes. The helper-target compatibility directives still
leave a zero-width `<program>` map range at `$2F18`, but it emits no bytes.

The new listing accounts for 9,362 instruction-form bytes and 867 data-form
bytes. Five bytes changed listing classification after relocation of symbolic
machine-block contents, so the instruction/data split does not move by exactly
49 bytes even though the loaded segment does. Against the unchanged
modern/classic result, MIR6502 is now 204 bytes smaller overall and 46
instruction-form bytes smaller.

This should be fixed at the SemIR-to-NIR boundary, not with store coalescing or
generic absolute-memory DSE. Remaining forms need explicit compile-time
contracts:

- keep helper-target overrides as non-executable program metadata rather than
  block operations;
- keep `SET symbol=*` as a post-layout data patch;
- model `SET *=value` as a source-ordered compile-time output write (the
  original compiler writes the value at the current code pointer);
- define or diagnose arbitrary compile-time memory writes that are neither
  compiler controls nor addresses in the generated image.

## Recommended execution order

1. **Known-callee word-result lane-first placement.** Fifteen exact sites,
   approximately 30 bytes, using infrastructure already implemented in
   Slices 4-6.
2. **Single-branch lowering for inclusive constant comparisons.** Ten
   immediately safe sites and a 20-byte ceiling; follow with profitable
   direct-memory operand reversal.
3. **Four-byte SArgs shadow frames.** Start with full-shadow `Window` and
   `Strcpy`, then add partial per-byte capture. Expected 20-30 bytes plus the
   largest runtime improvement in this list.
4. **Finish indexed-word destination selection.** High-byte-first A/X loads at
   two sites, then the carry-preserving computed-index case in `MakeJmp`.
5. **Definition-sensitive private-store elimination.** Target the audited
   `Window` and `MakeJmp` definitions before considering a broad pass.
6. **Call-result truth branching and block layout.** Begin with the isolated
   `PopUp` result-test shape; do not start with a general layout rewrite.
7. **Remove executable lowering of compile-time `SET`.** First delete the dead
   TN compiler-control stores, then move helper overrides to non-executable
   metadata and specify the remaining output-patch forms.

These bounded families have a combined plausible first-pass ceiling of roughly
100-140 bytes. They improve the 10K trajectory but do not alone establish a
sub-10,000-byte result; another audit will still be required after their
interaction and fixed-point effects are measured.

## Artifacts

Artifacts are in `target/tn-post-slice6-audit/`.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `TN-mir6502.xex` | 10,290 | `6879485b54010b6aa79adadfe2b10be6db3de785be3dd65a65ea9b5bf5aa45c2` |
| `TN-mir6502.lst` | 137,021 | `0b4bcf8dbda5dfb7975513999ecf390d1425ae6e42dfb137d55012315527a101` |
| `TN-materialized.mir` | 149,994 | `31c4d307d58cb90b13e82e9735ca455b0c37878d0e4b4344d91e8fcce72ccd11` |
| `TN-pre.mir` | 130,090 | `58f8a8488af6086e88e6652bdd7ee569fa7b7154d216f89850263b2e727e9207` |
| `TN-mir6502.map` | 10,967 | `1945190628df12c6e66c816be5229a1e57acbc3933491612e14530b48a0cc115` |
| `TN-classic.xex` | 10,445 | `3caefd677ab3d1489e39fcc0200126b442a15278b26a9cb5351434a1c8674f39` |
| `TN-classic.lst` | 136,233 | `668fb6ad3376a4449c6d79d2ae3050ca4deb415325002642cc244c9404750552` |

## Reproduction

```sh
mkdir -p target/tn-post-slice6-audit

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/tn/modern/TN.ACT \
    > target/tn-post-slice6-audit/TN-mir6502.lst \
    2> target/tn-post-slice6-audit/TN-mir6502.peepholes

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-listing \
  samples/tn/modern/TN.ACT \
  > target/tn-post-slice6-audit/TN-classic.lst

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-post-slice6-audit/TN-pre.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-post-slice6-audit/TN-materialized.mir

for backend in mir6502 classic; do
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend "$backend" --emit-map \
    samples/tn/modern/TN.ACT \
    > "target/tn-post-slice6-audit/TN-$backend.map"

  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend "$backend" --emit-load \
    samples/tn/modern/TN.ACT \
    > "target/tn-post-slice6-audit/TN-$backend.xex"

  cargo run --quiet --bin actionc-listing-quality -- \
    "target/tn-post-slice6-audit/TN-$backend.lst" \
    > "target/tn-post-slice6-audit/TN-$backend.quality"
done
```
