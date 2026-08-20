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
| IFP | `$D9AA` |
| FPI | `$D9D2` |
| FSUB | `$DA60` |
| FADD | `$DA66` |
| FMULT | `$DADB` |
| FDIV | `$DB28` |

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

## Arithmetic Vectors

With FR0=`1.25` and FR1=`2`:

| Routine | Result in FR0 |
| --- | --- |
| FADD | `40 03 25 00 00 00` (`3.25`) |
| FSUB | `BF 75 00 00 00 00` (`-0.75`) |
| FMULT | `40 02 50 00 00 00 00` (`2.5`) |
| FDIV | `3F 62 50 00 00 00 00` (`0.625`) |

FR1 is not preserved: the four routines leave different intermediate bytes in
it. A, X, Y, and status also have routine-specific observable results. Native
REAL lowering must therefore treat all processor registers, flags, FR0, FR1,
and the intervening FPP workspace as clobbered until a narrower audited effect
contract exists.

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
is held in compiler-generated frame storage because FPP calls are opaque and
may clobber registers, flags, FR0, FR1, and workspace.

## Compatibility Baseline

`REAL` must remain an ordinary identifier in the parser. The semantic
regression `historical_real_name_is_an_ordinary_six_byte_record_type` proves
that this historical declaration retains its record meaning and layout:

```action
TYPE REAL=[CARD r1,r2,r3]
```

The future modern-profile built-in type must live in an outer built-in scope so
this source declaration can shadow it.
