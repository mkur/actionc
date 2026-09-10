# Signed Q8.8 fixed point

`USE MATH.Q8_8 AS Q` imports the embedded fixed point library. It works in
Compatibility, Optimized classic, and MIR6502 with cartridge or standalone
linking. Numerical standalone programs need neither the OS ROM nor the Action!
cartridge. The module does not load the REAL library.

A value is an ordinary two-byte INT holding `raw / 256`. The eight integer
bits include the sign. Range is -128 through 127.99609375, with a step of
0.00390625. Raw 384 represents 1.5; raw -384 represents -1.5.

## Available API

| Member | Meaning |
| --- | --- |
| `Q.One` | Raw 256, representing 1 |
| `Q.Half` | Raw 128, representing 0.5 |
| `Q.Epsilon` | Raw 1, the smallest positive step |
| `Q.MinValue` | Raw -32768, representing -128 |
| `Q.MaxValue` | Raw 32767, representing 127.99609375 |
| `Q.FromInt(INT value)` | Returns INT raw `value * 256`, wrapping to 16 bits |
| `Q.Trunc(INT value)` | Returns the ordinary INT part, truncating toward zero |
| `Q.FromRatio(INT numerator, denominator)` | Constructs raw Q8.8 from an ordinary integer ratio |
| `Q.Mul(INT left, right)` | Multiplies raw Q8.8 values, then divides the full product by 256 |
| `Q.Div(INT left, right)` | Divides raw Q8.8 values, first multiplying the numerator by 256 |

All functions return INT. Mul, Div, and FromRatio truncate toward zero before
wrapping to 16 bits. For example, Mul(384,512) is raw 768 (1.5 times 2 is 3),
Mul(-1,1) is zero, and Div(-256,768) is raw -85. Mul(32767,512) wraps to raw -2;
Div(-32768,-256) wraps to raw -32768. RSH is logical and cannot express this
truncation rule for signed inputs.

FromRatio(3,2) constructs raw 384, representing 1.5. FromRatio and Div perform
the same raw calculation but describe different input interpretations. Their
zero denominator, including 0/0, invokes the existing non-returning Error(101).
The caller's result destination and subsequent effects are not executed.
Passing a literal zero to these functions is legal source and faults when the
call executes; direct invalid constant arithmetic retains its diagnostic.

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

The [complete sample](../samples/fixed-point/q8_8.act) exercises all five
functions and accumulates four steps of 1.5 to reach raw 1536 (6.0). Its
[README](../samples/fixed-point/README.md) gives build commands and exact
output. Integer printing in the sample requires the Atari OS; the arithmetic
library itself does not.

From the repository root run `cargo test --test fixed_q8_8`; from
`tools/vm-runtime-tests` run `cargo test --locked --test fixed_q8_8` for the
independent host-oracle execution tests.

The [implementation plan](SIGNED_Q8_8_LIBRARY_IMPLEMENTATION_PLAN.md) records
the scope and acceptance checks. Additional formats, native types, saturation,
round-to-nearest, REAL conversion, and fixed point decimal I/O remain deferred.
