# Default ZP homes on TN

Optimized MIR6502 compiled the shipped TN and TNDBG at origin `$3000`.
The compiler revisions are `9ba0a77a` (baseline) and `e46b66f7` (default homes).
Both compiled the same checkout of the Action! sources. The sizes include the
complete Atari load file and RUNAD.

## Images, execution and compiler time

Each arrow goes from the baseline compiler to the default-home compiler. A single
number means both produced the same result.

| Program | Runtime | XEX bytes | Panel cycles | 3-file copy cycles | 20-file copy cycles | Median compile seconds |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| TN | cart | 9,584 → 9,588 | 1,020 | 38,042 → 38,038 | 245,164 → 245,148 | 23.59 → 23.45 |
| TN | standalone | 9,723 → 9,727 | 963 | 39,271 | 251,639 → 251,659 | 23.41 → 23.20 |
| TNDBG | cart | 12,602 → 12,601 | 1,026 | 41,247 | 258,867 → 258,863 | 29.44 → 29.42 |
| TNDBG | standalone | 12,743 → 12,742 | 969 | 42,485 | 265,338 | 29.38 → 29.48 |

All four median compile times changed by less than 1%. The largest change
was TN standalone, which compiled 0.21 seconds faster. TNDBG standalone took
0.10 seconds longer. Panel cycles matched throughout. The twenty-file TN
copy saved 16 cycles on cart and took 20 more on standalone.
[Individual timings and XEX hashes](measurements.json)

The panel measurement covers the first no-refresh switch after tagging and
navigation. Both MyDOS setups give the same count. The copy cases use files
of 700, 64 and zero bytes with a 256-byte arena, and twenty 17-byte files with
a 512-byte arena. The tests check the returned stack, the contents of each file, the
number of opens, partial reads, disk prompts, and the restored panel. Cycles
cover the call stub through its return on the same VM, with the same file
contents and CIO hooks. The compiler timings use release binaries with tracing
off. Each program/runtime pair has one warm-up and three timed builds per
compiler, alternating compiler order.

To rerun the VM comparison on the checked-out compiler, run this from the repository root.

```sh
ACTIONC_TN_HOME_CYCLES=1 cargo test --release --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test tn_dispatch default_home_panel_and_copy_cycles -- --ignored --nocapture
```

## Remaining inline homes in Copy

| Local | First call with an unknown returning write set | MIR site in TN |
| --- | --- | --- |
| `mem`, `len`, `files` | `Close(1)` | block 14, operation 8 |
| `j` | First `SwapWin()` after `j = drive` | block 0, operation 2 |
| `k` | `SwapWin()` inside the destination loop | block 22, operation 10 |
| `flag`, `isopen` | `Window()` in the source disk prompt | block 9, operation 0 |
| `diskswap` | Second `SwapWin()` | block 4, operation 0 |

`Convert(v(j))` writes the ordinary `fname` array. The three values live
through that store now reach the check at `Close`. That call can reach CIO and
a returning error handler. Its current review supplies entry requirements
without a return guarantee for a private pool byte.

The first `SwapWin` passes zero as the refresh argument to `SetWin`.
The two nine-byte `MovePage` calls copy ordinary panel records. The selected
callee summary also includes the refresh branch with CIO. Later, `Bget`
returns through CIO before `copytab(files) = j` reads the file index. Moving
`j` requires a byte that survives both uses of the local.
[Panel switch and copy loop](../../../samples/tn/modern/TN.ACT)

## Final source homes

Cart and standalone select the same homes. TN places 40 parameters and 22
locals from 32 routines into 13 physical bytes. TNDBG places 48 parameters
and 26 locals from 39 routines into 14 physical bytes. The table counts a
parameter with a resident byte lane once. Only the low byte of `Strcpy.s`
has a memory home.

TN keeps the same set of resident objects as the baseline. `ArenaCursor.captured`
moves from `$A0–$A1` to `$A1–$A2`. In `Ord`, `item` moves from
`$A2–$A3` to `$E4–$E5` and `c` moves from `$E4` to `$A2`.
Copying the incoming high byte of `item` from `$A3` to `$E5` adds
four instruction bytes. TNDBG also gains `Dbg.prev` at `$EC`. Its
lifetime is disjoint from that of the pointer `Dbg.p` at `$EC–$ED`.
The net TNDBG image is one byte smaller.

The placement check compares each proposed batch against the currently
selected image for that compilation. For TN cart the final batch reduced
9,967 bytes to 9,588. The previous compiler chose a different placement for
`Ord` and produced the 9,584-byte baseline image.

| Routine | ZP parameters | ZP locals | Program |
| --- | --- | --- | --- |
| `Alloc` | — | `captured $A2–$A3`, `request $E2–$E3` | Both |
| `ArenaCursor` | — | `captured $A1–$A2` | Both |
| `Attrib` | — | `cmd $E2` | Both |
| `CloseWindow` | — | `buf $E4–$E5` | Both |
| `Convert` | `s $E2–$E3` | — | Both |
| `Dbg` | — | `p $EC–$ED`, `sptr $EE–$EF`, `prev $EC`, `ch $EA` | TNDBG |
| `Draw` | `cnt $E2` | — | Both |
| `DrawMenu` | `menu $EA–$EB` | — | Both |
| `FindItem` | `menu $A2–$A3`, `ch $E2` | — | Both |
| `Fnamecmp` | `s $A0–$A1`, `t $A2–$A3` | — | Both |
| `FormatDirectoryRow` | — | `sectorEnd $E2` | Both |
| `Free` | — | `captured $A2–$A3`, `mark $E2–$E3` | Both |
| `Init` | — | `h $E2–$E3` | Both |
| `InputLine` | `string $E4–$E5`, `maxlen $EA`, `curpos $EB` | — | Both |
| `Instr` | `s $A0–$A1`, `ch $A2` | — | Both |
| `Inv` | `cnt $A2` | — | Both |
| `Items` | `menu $A2–$A3` | `c $E2` | Both |
| `MoveMenuBar` | `menu $A2–$A3`, `c $E2` | `p $E3` | Both |
| `Ord` | `menu $E2–$E3`, `item $E4–$E5` | `c $A2` | Both |
| `PopUp` | `spc $E4` | `ch $EA`, `PopUp::block0::block1::item $E2–$E3` | Both |
| `Position` | `x $A0`, `y $A1` | — | Both |
| `Print` | `s $E3–$E4` | `i $E5` | Both |
| `PrintDbgByte` | `label $E2–$E3`, `b $EA` | — | TNDBG |
| `PrintDbgCard` | `label $E2–$E3`, `c $EA–$EB` | — | TNDBG |
| `PrintFname` | `s $E3–$E4` | `i $E5` | Both |
| `PrintHexB` | `b $E3` | — | TNDBG |
| `PrintHexC` | `c $E4–$E5` | — | TNDBG |
| `PrintHexDigit` | `b $E2` | — | TNDBG |
| `PrintNonzeroDigit` | `b $E3`, `divisor $E2` | — | Both |
| `PrintRawEntry` | `p $EA–$EB` | — | TNDBG |
| `Push` | — | `captured $E2–$E3` | Both |
| `Putchar` | `ch $E2` | — | Both |
| `Range` | `s $EA–$EB`, `max $EC`, `c $ED–$EE` | — | Both |
| `Relpos` | `x $A0`, `y $A1` | — | Both |
| `SetWin` | `n $E2`, `drvnum $E3` | `panelDataOld $E4–$E5` | Both |
| `Sort` | — | `h $E2–$E3` | Both |
| `Strcpy` | `s $A0`, `t $A2–$A3` | — | Both |
| `Tag` | `i $E2` | `a $E3` | Both |
| `Window` | `x $E4`, `y $E5`, `dx $EA`, `dy $EB` | `buf $EC–$ED`, `h $E2` | Both |

## Validation

MIR6502 library tests had 1,591 passed and 5 failed. Across all 35 MIR6502 integration
targets, 159 passed and 22 failed after focused reruns. All 22 remaining
integration failures also reproduced on the baseline checkout. The 5 unit
failures are recorded in the earlier home-admission qualification.
The four affected VM targets had 43 passed and 1 failed. The remaining VM failure
is the existing directional-reader placement assertion. The TN workloads,
entry-local and retained-local VM cases, handoff controls, and MADS LF/CRLF
relocation check passed. Enabling the home trace produced a byte-identical
TN cart image. [Check inventory](checks.json) ·
[Earlier unit results](../6502-home-admission/checks.json) ·
[Directional-reader baseline](../6502-runtime-contracts/README.md)
