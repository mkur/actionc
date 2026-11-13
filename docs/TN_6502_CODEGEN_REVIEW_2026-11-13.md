# TN 6502 code-generation review — 2026-11-13

## Scope and evidence

This is a static review of the supplied Action! compiler assembly listing,
compared with the maintained TOMS Navigator sources:

- [TN.ACT](https://github.com/mkur/actionc/blob/c00f37a84b8c0943c81565723420ace2c0d4f8d2/samples/tn/modern/TN.ACT)
- [LIB.ACT](https://github.com/mkur/actionc/blob/c00f37a84b8c0943c81565723420ace2c0d4f8d2/samples/tn/modern/LIB.ACT)

Source reference: `c00f37a84b8c0943c81565723420ace2c0d4f8d2`
(`Shrink TN through shared formatting and compact dispatch`).

Reviewed input:

- Supplied filename: `Wklejony tekst(1).txt`.
- SHA-256: `b9060b063167799480a4c2b8b017efd462ad50b5b377971f4586bb79b03e2b9d`.
- 5,488 lines; re-originable MADS assembly listing.
- Origin: `$2C00`; runtime: `cart`; no embedded runtime.
- Main segment: `$2C00–$5303`, inclusive.

The listing was not regenerated for this review. The source reference identifies
the sources inspected, not a verified compiler build revision or command line.
The listing header does not identify the optimization profile/backend.
All addresses and measurements below refer to this supplied artifact.

This review uses static instruction analysis, not an emulator benchmark or a
complete correctness qualification. It is separate from the
[2026-11-10 review](TN_6502_CODEGEN_REVIEW_2026-11-10.md); findings from that older
artifact should not automatically be applied to this one.

## Overall assessment

**The code generation is competent but uneven, with substantial room for
improvement, especially in pointer handling and loops.** It already performs
useful optimizations, but several important routines spend more time moving
addresses and temporary values around than doing their actual work.

The emitted main segment occupies **9,988 bytes**, excluding deferred buffers
and load metadata. Classification using the listing's routine/machine markers
and data directives gives:

| Main-segment content | Bytes |
| --- | ---: |
| Compiler-generated instructions | 8,291 |
| Instructions from inline assembly | 615 |
| Data and embedded descriptors | 1,082 |
| Total | 9,988 |

These are artifact measurements, not an estimate of future savings.

## What is already good

- **Tail-call optimization is widespread.** There are 33 direct
  `JMP proc_…` sites, including straightforward tail calls in `SavePos`,
  `CloseWindow`, and `UpdDis`. This count includes all such sites, not only
  optimizer-created tail calls.
- **Register preservation information is being used.** The generated
  `Putchar` is just **15 bytes** (`$317F–$318D`). It keeps the character in Y
  across `CalcAdr`, then uses the result of `Internal` directly from A.
- **Basic arithmetic lowering is reasonable.** There are efficient
  `INC`/`DEC` sequences, shifts for multiplication or division by two, and
  sensible carry propagation.
- **Control-flow expressions generally branch directly**, without always
  constructing a Boolean in memory.

An important distinction: `MovePage`, `Block`, `GetImage`, `PutImage`, and
the compact `MakeJmp` dispatcher are handwritten `ASM` in the source.
Their quality should not be credited to automatic code generation.

## Main missed opportunities

| Area | Evidence in this listing | Improvement |
| --- | --- | --- |
| Pointer residency | `Fnamecmp` reconstructs two zero-page pointers on every character comparison | Keep the working pointers in zero page throughout the loop |
| Dead temporary stores | Six spill locations in `FormatDirectoryRow` and `SetWin` are written but never explicitly read | Remove obsolete captures and their storage after checking escape/alias facts |
| Redundant loads | `Convert` reads the same character twice before calling `Ascii` | Reuse the value already in A |
| Constant multiplication | `DrawWinFrame` and `SwapWin` call `MultI` for multiplication by 20 | Generate shifts and addition, particularly when only the low byte is needed |
| Address calculation | `Init` computes `i+24` at runtime to index `quickmul` | Fold the constant into the indexed base address |
| Range and flag knowledge | Unnecessary portions of word comparisons and repeated overflow checks | Use proven ranges and carry/zero flag state |

### 1. Fnamecmp: a substantial speed opportunity

The function occupies **185 bytes** (`$3B3D–$3BF5`). Its equal-character loop
at `$3BB5` repeatedly performs:

```asm
LDA param_Fnamecmp_s
STA $AE
LDA param_Fnamecmp_s+1
STA $AF

LDA param_Fnamecmp_t
STA $AC
LDA param_Fnamecmp_t+1
STA $AD
```

That is **20 bytes and 28 cycles of pointer setup per character**, before
comparing anything. It then increments both pointers in absolute memory and
reconstructs them again. There are no calls inside this comparison loop.

If the working pointers can be promoted into zero page, with Y initialized
to zero outside the loop, the ordinary equal-character path changes as follows:

| Work per iteration | Current | Resident zero-page pointers |
| --- | ---: | ---: |
| Reload two pointer pairs | 28 cycles | 0 cycles |
| Initialize Y to zero | 2 cycles | 0 cycles |
| Indirect load and comparison | 10 cycles | 10 cycles |
| Unequal-character branch, not taken | 2 cycles | 2 cycles |
| Increment two pointers without low-byte wrap | 18 cycles | 16 cycles |
| Jump back | 3 cycles | 3 cycles |
| Total | **63 cycles** | **31 cycles** |

These are calculated NMOS 6502 timings, assuming neither pointer's low byte
wraps and no branch-page penalty. They exclude function setup and the final
unequal-character path. The candidate requires the parameter homes to be
unobservable where their updates are eliminated, and must obey the ABI and
scratch-allocation contracts.

**Roughly halving this inner loop's cost is realistic.** Because `Sort`
calls the comparator repeatedly, this is a more valuable target than shaving
instructions from occasional UI operations. It does not establish a twofold
speedup for the complete sort or application.

### 2. FormatDirectoryRow: temporary management needs another cleanup pass

This routine occupies **309 bytes** (`$40C9–$41FD`).

The following compiler-generated locations have stores but no explicit reads
anywhere in the supplied listing:

| Spill | Storage address | Store instruction |
| --- | --- | --- |
| `spill_FormatDirectoryRow_12` | `$2F06` | `$40E5` |
| `spill_FormatDirectoryRow_13` | `$2F07` | `$40ED` |
| `spill_FormatDirectoryRow_26` | `$2F08` | `$4113` |
| `spill_FormatDirectoryRow_27` | `$2F09` | `$4119` |
| `spill_SetWin_110` | `$2F44` | `$43AD` |
| `spill_SetWin_111` | `$2F45` | `$43B3` |

For example, the conversion loop captures `global_s` into spills, then
separately reloads it into `$AC/$AD`. The captured values have no explicit
consumers.

One plausible explanation is that a later optimization eliminated pointer
restoration without eliminating its earlier capture. This is an inference
from the final listing, not a verified diagnosis of pass ordering.

**Running dead-store elimination after those transformations would be a
particularly useful, relatively contained improvement.** The implementation
must establish that the compiler-owned spills are not observable through
indirect accesses or opaque code; absence of explicit loads alone is not a
general alias proof.

There is also repeated pointer setup around `Internal`, even though its
source explicitly declares preservation of `$AC/$AD` and `$AE/$AF`.
Keeping pointers resident across calls and loop iterations would help further.
This also requires proving that the indirect row writes do not overwrite the
pointer's backing storage or compiler scratch.

### 3. Small, demonstrably unnecessary sequences remain

In `Convert`, at `$3F34`:

```asm
LDA ($AC),Y
BEQ loc_Convert_3
LDA ($AC),Y
JSR proc_Ascii
```

The second load is unnecessary for this ordinary-RAM character access:
neither the branch nor anything else changes A. This occurs twice in the
function, at `$3F38` and `$3F72`, costing **2 bytes and 5–6 cycles each time
the redundant load executes**. A general transformation must retain genuinely
volatile or otherwise observable reads.

In `PrintNonzeroDigit`, at `$44A3`:

```asm
CMP param_PrintNonzeroDigit_divisor
BCC loc_PrintNonzeroDigit_2
SEC
SBC param_PrintNonzeroDigit_divisor
```

The `SEC` at `$44A8` is unnecessary because falling through `BCC` already
establishes carry set. Removing it saves one byte and two cycles per executed
subtraction.

These are small individually, but demonstrate that local register-value and
flag tracking still have useful work to do.

### 4. Constant arithmetic and indexed addressing

`DrawWinFrame` calls `MultI` at `$3E2D` to multiply `winnum` by 20.
`SwapWin` does the same at `$4485`. Both consume the low byte of the product.

A shifts-and-addition sequence can calculate that byte for every byte input,
using `20*x = 16*x + 4*x` modulo 256. This does not require assuming that
`winnum` is always zero or one. The variable dimension multiplication in
`Window` is a different case; a runtime helper is reasonable there.

In `Init`, the high-byte table store at `$39BE–$39CD` stages a value in
`$E2` and computes `i+24` explicitly. Y already contains `i` from the
preceding low-byte store. The loop establishes `i < 24`, allowing the
indexed operand `global_quickmul+24,Y` to represent the offset without an
eight-bit-wrap ambiguity.

Both are general instruction-selection opportunities, not reasons to add
TN-specific special cases.

## Preserve the required semantics

The `CMP #$FF` checks in ascending byte loops are not automatically mistakes.
They can prevent an inclusive loop from wrapping and continuing forever when
its limit is 255. Removing them requires proving the relevant range and
preserving any observable final index value.

Pointer reloads across arbitrary memory-writing calls can be necessary.
Explicit clobber/preservation annotations are valuable, but register
preservation alone does not prove that all referenced memory is unchanged.

Other simplifications are more direct: the unsigned comparison
`captured < $2000` in `ArenaCursor` only needs the high byte, yet this
listing also contains low-byte comparison machinery.

## Suggested development priorities

1. **Late dead-store elimination and redundant-load/flag cleanup.**
   Relatively inexpensive improvements with clear examples in this artifact.
2. **Keep pointers in zero page across loops and known calls.**
   The largest demonstrated speed opportunity.
3. **Improve constant arithmetic and indexed addressing.**
   Multiplication by 20 and `quickmul+24,Y` are concrete targets.
4. **Add range-driven loop simplification and selective inlining.**
   Useful once the above are solid.

Implementation should follow [AGENTS.md](../AGENTS.md): use verified structured
facts, conservative memory effects, and general compiler behavior.
Target addressing, register/flag use, zero-page placement, and helper selection
belong to MIR6502; source semantics and normalized facts should retain their
existing ownership. This review does not establish which existing pass should
own every proposed transformation.

Expected benefit: **meaningful size reductions and large improvements in
selected CPU-bound routines**. This listing does not justify predicting a
particular whole-program percentage or a twofold application speedup.
Disk I/O and existing handwritten graphics routines will limit the overall
effect. The remaining opportunity is nevertheless considerably greater than
a few cosmetic peephole changes.

## Validation performed

- Compared the supplied listing with the two source files at the pinned revision.
- Counted bytes using emitted address/byte comments and routine/machine markers.
- Checked the six spill symbols and their numeric storage addresses for explicit
  references throughout the listing.
- Calculated the `Fnamecmp` ordinary equal-character path from standard NMOS
  6502 instruction timings under the assumptions stated above.
- Checked this document's source and repository links.

No compiler implementation or sample source was changed, no candidate rewrite
was assembled or executed, and no compiler or emulator test suite was run for
this documentation-only review.
