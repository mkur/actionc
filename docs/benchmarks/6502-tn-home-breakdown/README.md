# TN zero-page and inline homes

Optimized MIR6502 with reviewed assembly parameter-home elision gives TN the same home assignments with the cart and standalone runtimes at origin `$3000`. The count covers all 102 parameters and 87 locals in the shipped modern program and its included library. [TN.ACT](../../../samples/tn/modern/TN.ACT) · [LIB.ACT](../../../samples/tn/modern/LIB.ACT)

| Final placement | Parameters | Locals | Total homes | Mapped home bytes |
| --- | ---: | ---: | ---: | ---: |
| ZP pool | 40 | 22 | 62 | 90 (56 parameter, 34 local) |
| Inline in the load image | 9 | 34 | 43 | 82 (12 parameter, 70 local) |
| Deferred array storage after the load image | 0 | 9 | 9 | 2,784 |
| **Compiler-allocated memory homes** | **49** | **65** | **114** | **2,956** |
| ABI only, no memory home | 53 | 0 | 0 | 0 |
| Source-fixed address or alias | 0 | 22 | 0 | 0 |

The 62 ZP homes reuse **13 physical bytes** across 32 routines, in ranges `$A0–$A3`, `$E2–$E5` and `$EA–$EE`. Each source object counts once. `Strcpy.s` contributes one byte at `$A0` for the low half of its pointer. The other 52 memory homes occupy 2,866 distinct addresses. Byte totals use the named extent in the final storage map. For an array descriptor this measures the descriptor itself. Backing payloads, globals and compiler spills are outside the home totals.

## Application and library

| Source | ZP params | ZP locals | Inline params | Inline locals in image | Deferred arrays | ABI-only params | Fixed or aliased locals |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `TN.ACT` | 15 | 6 | 4 | 30 | 8 | 10 | 18 |
| `LIB.ACT` | 25 | 16 | 5 | 4 | 1 | 43 | 4 |

## Routines

The tables list every routine with a compiler-allocated memory home. The ZP columns give addresses. The inline and deferred columns give mapped bytes in parentheses. The address inventory includes all 189 declarations, including ABI-only parameters, 21 fixed-address locals, and the alias `View.ch` for global `h`. It gives both runtime addresses for each object. [Address inventory (CSV)](homes.csv)

- `Copy` uses eight inline scalar locals (10 bytes) and two deferred arrays, `lentab` (32 bytes) and `copytab` (16 bytes). `transf` at `$0358` and `memtop` at `$02E5` are source-fixed words.
- `SetWin` uses ZP for both parameters and for `panelDataOld`. Its seven inline locals occupy 32 mapped bytes. Its six deferred panel buffers and tables occupy 2,696 bytes.
- `SwapScr` declares four bytes of `OWN` state. `firstTime` remembers that initialization ran, `scrnum` remembers the selected screen, and `buffer` holds the address returned by the first allocation. Classic Atari also preserves these values with the original unmarked declarations. The other two inline bytes hold `a`, a loop cursor assigned from `buffer` before use on each swap. [SwapScr source](../../../samples/tn/modern/TN.ACT#L772-L800)
- `Position` and `Relpos` each use `$A0` for `x` and `$A1` for `y`. `Range` uses five pool bytes for its three parameters. Its four locals explicitly name `$A0–$A3` in the source.

### TN.ACT

| Routine | ZP parameters | ZP locals | Inline parameters | Inline locals | Deferred locals |
| --- | --- | --- | --- | --- | --- |
| `Attrib` | — | `cmd` `$E2` | — | — | — |
| `Convert` | `s` `$E2–$E3` | — | — | — | — |
| `Copy` | — | — | — | `mem` (2), `len` (2), `files` (1), `j` (1), `k` (1), `flag` (1), `diskswap` (1), `isopen` (1) | `lentab` (32), `copytab` (16) |
| `Draw` | `cnt` `$E2` | — | — | — | — |
| `Fnamecmp` | `s` `$A0–$A1`, `t` `$A2–$A3` | — | — | — | — |
| `Format` | — | — | — | `aux` (1), `last` (1), `fname` (4), `h` (2) | — |
| `FormatDirectoryRow` | — | `sectorEnd` `$E2` | — | — | — |
| `Handle` | — | — | — | `ch` (1), `command` (1), `d` (1), `p` (2) | — |
| `InitPanels` | — | — | — | `times` (1) | — |
| `Instr` | `s` `$A0–$A1`, `ch` `$A2` | — | — | — | — |
| `Inv` | `cnt` `$A2` | — | — | — | — |
| `MakeJmp` | — | — | `command` (1) | — | — |
| `NewDrive` | — | — | — | `h` (1) | — |
| `PrintFname` | `s` `$E3–$E4` | `i` `$E5` | — | — | — |
| `PrintNonzeroDigit` | `b` `$E3`, `divisor` `$E2` | — | — | — | — |
| `SetWin` | `n` `$E2`, `drvnum` `$E3` | `panelDataOld` `$E4–$E5` | — | `panelData` (2), `leftPanel` (9), `rightPanel` (9), `pathBuf` (2), `dirBuf` (2), `lp_names` (4), `rp_names` (4) | `lp_PathBuf` (47), `rp_PathBuf` (47), `lp_DirBuf` (1171), `rp_DirBuf` (1171), `lp_v` (130), `rp_v` (130) |
| `Sort` | — | `h` `$E2–$E3` | — | — | — |
| `Strcat` | — | — | `s` (2), `t` (2) | — | — |
| `Strcpy` | `s` `$A0` (low byte), `t` `$A2–$A3` | — | — | — | — |
| `SwapScr` | — | — | — | `firstTime` (1), `scrnum` (1), `buffer` (2), `a` (2) | — |
| `Tag` | `i` `$E2` | `a` `$E3` | — | — | — |
| `Xloop` | — | — | `cmd` (1) | `j` (1) | — |

### LIB.ACT

| Routine | ZP parameters | ZP locals | Inline parameters | Inline locals | Deferred locals |
| --- | --- | --- | --- | --- | --- |
| `_StrNam` | — | — | — | — | `fnam` (40) |
| `Alloc` | — | `captured` `$A2–$A3`, `request` `$E2–$E3` | — | — | — |
| `ArenaCursor` | — | `captured` `$A1–$A2` | — | — | — |
| `CloseWindow` | — | `buf` `$E4–$E5` | — | — | — |
| `DrawMenu` | `menu` `$EA–$EB` | — | — | — | — |
| `FindItem` | `menu` `$A2–$A3`, `ch` `$E2` | — | — | — | — |
| `Free` | — | `captured` `$A2–$A3`, `mark` `$E2–$E3` | — | — | — |
| `GetD` | — | — | `chn` (1) | — | — |
| `Init` | — | `h` `$E2–$E3` | — | — | — |
| `InputLine` | `string` `$E4–$E5`, `maxlen` `$EA`, `curpos` `$EB` | — | — | — | — |
| `Items` | `menu` `$A2–$A3` | `c` `$E2` | — | — | — |
| `MoveMenuBar` | `menu` `$A2–$A3`, `c` `$E2` | `p` `$E3` | — | — | — |
| `Ord` | `menu` `$E2–$E3`, `item` `$E4–$E5` | `c` `$A2` | — | — | — |
| `PopUp` | `spc` `$E4` | `ch` `$EA`, `item` `$E2–$E3` | `mp` (2), `c` (1) | `d` (1), `limit` (1), `menu` (2) | — |
| `Position` | `x` `$A0`, `y` `$A1` | — | — | — | — |
| `Print` | `s` `$E3–$E4` | `i` `$E5` | — | — | — |
| `Push` | — | `captured` `$E2–$E3` | — | `pushedWord` (2) | — |
| `Putchar` | `ch` `$E2` | — | — | — | — |
| `PutD` | — | — | `chn` (1), `ch` (1) | — | — |
| `Range` | `s` `$EA–$EB`, `max` `$EC`, `c` `$ED–$EE` | — | — | — | — |
| `Relpos` | `x` `$A0`, `y` `$A1` | — | — | — | — |
| `Window` | `x` `$E4`, `y` `$E5`, `dx` `$EA`, `dy` `$EB` | `buf` `$EC–$ED`, `h` `$E2` | — | — | — |

## Reviewed assembly parameters

Compared with revision `967ef595`, verified ABI entry contracts eliminate 39 parameter homes in 15 routines. This removes 54 bytes from each TN load image. The nine remaining inline parameters consume 12 bytes across `GetD`, `PutD`, `PopUp`, `Strcat`, `Xloop` and `MakeJmp`.

| Routine | Parameters now ABI only | Inline bytes removed |
| --- | --- | ---: |
| `Ascii` | `ch` | 1 |
| `Bget` | `chn`, `buf`, `len` | 5 |
| `Block` | `mode`, `dx`, `dy` | 3 |
| `Bput` | `chn`, `buf`, `len` | 5 |
| `Close` | `chn` | 1 |
| `GetImage` | `adr`, `dx`, `dy` | 4 |
| `Input` | `chn`, `buf`, `len` | 4 |
| `Internal` | `ch` | 1 |
| `MovePage` | `dst`, `src`, `len` | 5 |
| `MoveToArena` | `dst`, `src`, `len` | 5 |
| `Open` | `chn`, `s`, `aux1`, `aux2` | 5 |
| `PutImage` | `adr`, `dx`, `dy` | 4 |
| `Store` | `p`, `value` | 3 |
| `Value` | `p` | 2 |
| `Xio` | `chn`, `cmd`, `aux1`, `aux2`, `s` | 6 |

Both calls in `Convert` still pass the character to `Ascii` in A. `Ascii` starts with `TAY` and writes the converted result to `$A0`. Its `ch` parameter no longer occupies an inline byte. [Reviewed Ascii body](../../../samples/tn/modern/LIB.ACT#L236-L256)

## Measurement

The inventory joins optimized NIR declarations to the selected MIR6502 output’s `map.storage_symbols` by routine, role and name. Split parameter lanes are counted against their source parameter. The main load segment identifies which ordinary homes are in the image and which are in deferred storage. TN cart shrank from 9,588 to 9,534 XEX bytes and standalone from 9,727 to 9,673.
