# Atari FPP Oracle for Native REAL

This note records the executable baseline for Action 2027 native `REAL`.
`tools/vm-runtime-tests/src/atari_fpp_oracle.rs` invokes the AltirraOS XL/XE
floating-point package directly with a small machine-code harness. It does not
exercise either actionc backend, so these values can be used as an independent
codec and lowering oracle.

The exact compiler-side codec lives in `src/atari_real.rs`. It parses decimal
digits without using host binary floating point, requires full-token
consumption, canonicalizes zero, and diagnoses values above the Atari range.
The VM gate compares that codec directly with AFP across ordinary, signed,
fractional, exponent, truncation, overflow-boundary, and underflow cases.

## Confirmed Interface

| Purpose | Address |
| --- | ---: |
| FR0 | `$00D4` |
| FR1 | `$00E0` |
| CIX | `$00F2` |
| INBUFF | `$00F3` |
| AFP | `$D800` |
| FASC | `$D8E6` |
| IFP | `$D9AA` |
| FPI | `$D9D2` |
| FSUB | `$DA60` |
| FADD | `$DA66` |
| FMULT | `$DADB` |
| FDIV | `$DB28` |
| EXP | `$DDC0` |
| EXP10 | `$DDCC` |
| LOG | `$DECD` |
| LOG10 | `$DED1` |

FR0 and FR1 are six-byte packed-decimal values. Zero is six zero bytes. A
nonzero value stores sign and a biased base-100 exponent in the first byte,
followed by five packed-BCD mantissa bytes.

## AFP Vectors

The VM gate currently confirms:

| Text | FR0 bytes |
| --- | --- |
| `0` | `00 00 00 00 00 00` |
| `1` | `40 01 00 00 00 00` |
| `-1` | `C0 01 00 00 00 00` |
| `.5` | `3F 50 00 00 00 00` |
| `1.25` | `40 01 25 00 00 00` |
| `10` | `40 10 00 00 00 00` |
| `100` | `41 01 00 00 00 00` |
| `1234567890` | `44 12 34 56 78 90` |
| `9.999999999E97` | `70 99 99 99 99 99` |
| `1E99` | `71 10 00 00 00 00` |
| `1E-98` | `0F 01 00 00 00 00` |
| `1E-99` | `00 00 00 00 00 00` |

AFP canonicalizes `-0` to zero. Extra mantissa digits are discarded rather
than rounded by the probed routine. For example, `1.2345678904`,
`1.2345678905`, and `1.234567895` all produce `40 01 23 45 67 89`.

AFP accepts at most two exponent digits. On `1E100`, it consumes the `1E10`
prefix, leaves the last digit unconsumed (`CIX=4`), and produces the value for
`1E10`. The compiler must require the entire real token to be valid rather than
copy this prefix-accepting API behavior. Invalid leading input produces the
probed sentinel `7F 00 00 00 00 00` with `CIX=0`; compiler source should
receive a diagnostic instead of storing that sentinel.

## FASC and Transcendental Vectors

FASC writes through INBUFF and terminates its ATASCII result by setting bit 7
on the final character. The oracle confirms minimal spellings including `0`,
`1.25`, `-1.25`, `100`, `1E+20`, and `1E-20`. A library adapter must clear the
terminator bit before constructing an Action length-prefixed string.

For a normalized packed REAL, the longest scientific spelling uses a minus
sign, ten mantissa digits, a decimal point, and four exponent characters
(`E`, a sign and two digits). Fixed notation can add a leading zero before
the point and omits the exponent. The maximum is **16 characters**, so
`SYS.StrR` can write **17 bytes** including the Action length. The REAL
output wrappers' 20-byte strings accommodate that result. Both Atari OS-B
and the bundled AltirraOS produced `-1.234567891E+33` from
`D0 12 34 56 78 91`. On each ROM a direct probe checked 991 canonical
cases spanning zero, both signs, all supported exponents, full mantissas and
trailing zeroes. [AltirraOS FASC](https://github.com/atari800/atari800/blob/bbe287d6d2c233bc8bad92ed2b2637f6a3859eb6/emuos/src/mathpack.s#L390) ·
[FPP oracle tests](../../tools/vm-runtime-tests/src/atari_fpp_oracle.rs)

OS-B has an important exception for raw storage. With the malformed encodings
`3F 00 00 00 00 00` and `BF 00 00 00 00 00`, its trailing-zero scan can
underflow X to `$FF`. It marks `$067F` and returns without a terminator in
the documented output workspace. The Action wrapper can then read beyond
that workspace and continue copying. A numeric pointer and a six-byte
declaration alone cannot establish the 17-byte write bound.
[Atari OS-B ROM](https://github.com/sidneycadot/Atari8bit_OSB-NTSC-ROM/blob/10272c5e558457423e37f1e126e940c3d4cc6a2f/atariosb-ntsc.rom) ·
[SYS conversion source](../../embedded/runtime/SYSREAL.ACT#L13)

The transcendental entry points consume and replace FR0. Their results retain
the ROM package's decimal approximations; they are not host-math identities:

| Call | FR0 bytes |
| --- | --- |
| `EXP(0)` | `3F 99 99 99 99 98` |
| `EXP10(2)` | `40 99 99 99 99 98` |
| `LOG(1)` | `3B 04 60 51 70 18` |
| `LOG10(100)` | `40 02 00 00 00 00` |

These routines share FR0, CIX, INBUFF, and additional FPP workspace. Calls are
therefore treated as non-reentrant and not interrupt-safe unless the caller
saves and restores the complete workspace around every possible interruption.

## Arithmetic Vectors

With FR0=`1.25` and FR1=`2`:

| Routine | Result in FR0 |
| --- | --- |
| FADD | `40 03 25 00 00 00` (`3.25`) |
| FSUB | `BF 75 00 00 00 00` (`-0.75`) |
| FMULT | `40 02 50 00 00 00 00` (`2.5`) |
| FDIV | `3F 62 50 00 00 00 00` (`0.625`) |

FR1 is not portable preserved state. The bundled AltirraOS FADD happens to
leave it unchanged for this vector, while FSUB changes its first byte and
FMULT uses the full value destructively. The original Atari routines have a
different byte-level clobber pattern, so compiler correctness must use the
union documented below rather than observations from one ROM and one input.

## Audited Core-Service Effects

The audit covers the six entry points emitted by native `REAL` lowering. The
original-package column comes from the byte-by-byte compatibility matrix in
[AltirraOS `mathpack.s`](https://github.com/atari800/atari800/blob/bbe287d6d2c233bc8bad92ed2b2637f6a3859eb6/emuos/src/mathpack.s).
The AltirraOS column follows the complete helper call graph at that same
revision, which is the source revision for the bundled ROM. A VM bus-write
probe against that ROM additionally confirmed that representative successful
calls remain within these sets and do not write page-five FPP scratch.

| Service | Original math pack may modify | AltirraOS may modify | Portable union |
| --- | --- | --- | --- |
| IFP `$D9AA` | `$D4-$D9`, `$F8-$F9` | `$D4-$D9` | `$D4-$D9`, `$F8-$F9` |
| FPI `$D9D2` | `$D4-$D9`, `$EC`, `$F5`, `$F7-$FA` | `$D4-$D9` | `$D4-$D9`, `$EC`, `$F5`, `$F7-$FA` |
| FADD `$DA66` | `$D4-$DA`, `$E0-$E5`, `$F7-$F9` | `$D4-$D9` | `$D4-$DA`, `$E0-$E5`, `$F7-$F9` |
| FSUB `$DA60` | `$D4-$DA`, `$E0-$E5`, `$F7-$F9` | `$D4-$D9`, `$E0` | `$D4-$DA`, `$E0-$E5`, `$F7-$F9` |
| FMULT `$DADB` | `$D4-$E0`, `$E6-$EE`, `$F5-$F7` | `$D4-$EB` | `$D4-$EE`, `$F5-$F7` |
| FDIV `$DB28` | `$D4-$E0`, `$E6-$EE`, `$F5`, `$F7` | `$D4-$E0`, `$E6-$EC` | `$D4-$E0`, `$E6-$EE`, `$F5`, `$F7` |

The conventional names for these bytes—FR0, FRE, FR1, FR2, FRX, and the
ZTEMP variables—are also recorded in
[cc65's Atari equates](https://github.com/cc65/cc65/blob/master/asminc/atari.inc).
The complete package owns zero page `$D4-$FF`; other entry points, especially
polynomial and transcendental functions, additionally use page-five scratch.

The processor and call effects shared by these six services are:

- A, X, Y, N, Z, C, and V are volatile. MIR therefore clobbers A, X, Y, and
  its aggregate flags state.
- The interrupt-disable flag is preserved. Decimal mode is not a portable
  preserved value: IFP can return with it set, and arithmetic routines use and
  clear it on path-dependent exits. Both compiler backends therefore append
  `CLD` to every emitted core-service call and expose binary mode as the
  post-call compiler invariant.
- Internal JSR and PHA operations use hardware-stack bytes transiently, but SP
  is restored and the public stack-depth delta is zero.
- The routines call only internal math-pack helpers. They do not invoke CIO,
  SIO, Action runtime code, or a general OS service, and perform no arbitrary
  indirect program-memory writes.

MIR deliberately represents both reads and writes as the complete package
workspace `$D4-$FF`, rather than encoding the smaller per-service unions. This
is a stable contract for compatible replacement ROMs and remains narrow enough
to prove that ordinary params, locals, spills, non-overlapping globals, and the
`$AA-$AF` pointer scratch are preserved. Calls are non-opaque, do not set
`may_call_os`, and reserve the workspace from virtual zero-page allocation.

## Integer Conversion Vectors

IFP interprets the low two bytes of FR0 as an unsigned 16-bit magnitude. The
oracle confirms exact conversions for 0, 1, 255, 256, 32768, and 65535. FPI
likewise returns an unsigned word in FR0 and rounds a nonnegative magnitude to
the nearest integer: `1.25` becomes 1 and `1.5` becomes 2. The oracle covers
the full unsigned boundary through 65535.

The compiler owns signed Action `INT` adaptation around these unsigned OS
routines. It converts an integer's magnitude with IFP and applies the sign to
the packed REAL result. In the reverse direction it clears the REAL sign,
calls FPI, and applies two's-complement sign to the returned word. Sign state
is held in compiler-generated frame storage because FPP calls may clobber
registers, flags, FR0, FR1, and workspace. The audited structured effect proves
that the frame byte itself survives the call.

## SYS Integer String Formatting

`CToStr` places an unsigned word in FR0, calls IFP and FASC, and copies the
ATASCII digits from INBUFF to the counted string at `$0550`. A sweep of all
65,536 magnitudes on Atari OS-B and the bundled AltirraOS confirmed one to five
decimal digits, bit 7 set on the final digit, and a balanced stack. FASC pointed
INBUFF to `$0580` or `$0581` on both ROMs. The probe entered with decimal mode
clear. [Atari OS-B ROM](https://github.com/sidneycadot/Atari8bit_OSB-NTSC-ROM/blob/10272c5e558457423e37f1e126e940c3d4cc6a2f/atariosb-ntsc.rom) · [AltirraOS `mathpack.s`](https://github.com/atari800/atari800/blob/bbe287d6d2c233bc8bad92ed2b2637f6a3859eb6/emuos/src/mathpack.s#L390)

| ROM | Zero-page writes during IFP and FASC | Page-five writes |
| --- | --- | --- |
| Atari OS-B | `$D4–$DA`, `$F3/$F4`, `$F7/$F8` | `$057F–$058A` |
| Bundled AltirraOS | `$D4–$D9`, `$F3/$F4`, `$F7–$F9` | `$0580–$058A` |

FASC uses bytes beyond the final displayed digit while constructing the result.
The SYS review covers both documented FPP RAM windows, `$00D4–$00FF` and
`$057E–$05FF`, plus the hardware stack and `$0550–$0555` for the Action counted
magnitude. [Atari System Reference Manual, Chapter 11](https://www.atarionline.pl/biblioteka/materialy_ksiazkowe/Atari%20System%20Reference%20Manual.pdf)

`StrB`, `StrC`, and `StrI` copy to a caller-supplied destination. Their maximum
output sizes are 4, 6, and 7 bytes, including the length byte and the possible
sign. The destination must be proven below the loaded image and hardware range,
with no overlap with the wrapper's ABI scratch or formatter workspace. A write
through a pointer into its own pointer cells can redirect the remaining copy.

## Compatibility Baseline

`REAL` must remain an ordinary identifier in the parser. The semantic
regression `historical_real_name_is_an_ordinary_six_byte_record_type` proves
that this historical declaration retains its record meaning and layout:

```action
TYPE REAL=[CARD r1,r2,r3]
```

The future modern-profile built-in type must live in an outer built-in scope so
this source declaration can shadow it.
