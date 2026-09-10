# Signed Q8.8 fixed point

`USE MATH.Q8_8 AS Q` imports the embedded fixed point library. It works in
Compatibility, Optimized classic, and MIR6502 with cartridge or standalone
linking. Numerical standalone programs need neither the OS ROM nor the Action!
cartridge. The module does not load the REAL library.

A value is an ordinary two-byte INT holding `raw / 256`. The eight integer
bits include the sign. Range is -128 through 127.99609375, with a step of
0.00390625. Raw 384 represents 1.5; raw -384 represents -1.5.

## Available API

Slice 1 supplies constants and integer conversions. Multiplication, division,
and ratio construction are planned in
[slice 2](SIGNED_Q8_8_LIBRARY_IMPLEMENTATION_PLAN.md).

| Member | Meaning |
| --- | --- |
| `Q.One` | Raw 256, representing 1 |
| `Q.Half` | Raw 128, representing 0.5 |
| `Q.Epsilon` | Raw 1, the smallest positive step |
| `Q.MinValue` | Raw -32768, representing -128 |
| `Q.MaxValue` | Raw 32767, representing 127.99609375 |
| `Q.FromInt(INT value)` | Returns INT raw `value * 256`, wrapping to 16 bits |
| `Q.Trunc(INT value)` | Returns the ordinary INT part, truncating toward zero |

```action
MODULE EXAMPLE
USE MATH.Q8_8 AS Q

PROC Main()
  INT value,whole
  value=Q.FromInt(3) ; raw 768, representing 3
  value=value+Q.Half ; raw 896, representing 3.5
  whole=Q.Trunc(value) ; ordinary integer 3
RETURN
ENDMODULE
```

Trunc(-384) is -1. FromInt(128) wraps to raw -32768. Addition, subtraction,
negation, and signed comparisons use ordinary INT operators on values at the
same scale. Negating MinValue wraps back to MinValue. An unscaled integer 1
added to a raw value changes it by one fractional step; use One to add 1.0.

INT does not distinguish raw fixed point from ordinary integers. Keep the
scale explicit in variable names and conversions. Intermediates use LONGINT,
and narrowing occurs after scaling. No native fixed point type, implicit
scale conversion, saturation, or new interrupt/reentrancy guarantee is added.

From the repository root run `cargo test --test fixed_q8_8`; from
`tools/vm-runtime-tests` run `cargo test --locked --test fixed_q8_8` for the
independent host-oracle execution tests.
