# Signed Q4.12

Import `MATH.Q4_12 AS Q` with USE. Raw INT values represent `raw / 4096`;
range is -8 through 7.999755859375 in steps of 1/4096. Storage is 16 bits,
including the sign in the four integer bits. All three compiler modes and
both runtimes are supported; standalone numerical execution needs no ROMs.

| Member | Meaning |
| --- | --- |
| One, Half, Epsilon | Raw 4096, 2048, 1 |
| MinValue, MaxValue | Raw -32768, 32767 |
| `FromInt(INT value)` | Scale an ordinary integer by 4096 and wrap to INT |
| `Trunc(INT value)` | Extract the ordinary integer part, toward zero |
| `FromRatio(INT numerator, denominator)` | Construct a raw value from ordinary integers |
| `Mul(INT left, right)` | Multiply and rescale toward zero |
| `Div(INT left, right)` | Scale the numerator before dividing, toward zero |
| `MulFloor(INT left, right)` | Multiply and rescale toward negative infinity |
| `SqrWide(INT value)` | Return the exact raw square as LONGCARD, without rescaling |

Functions return INT except SqrWide. Arithmetic uses signed LONGINT
intermediates and wraps only after rescaling. Zero division uses non-returning
Error(101), including a literal zero passed to Div or FromRatio. Raw addition,
subtraction, negation, and signed comparisons use ordinary INT operations at
the same scale. These are library conventions, not distinct compiler types.

Mul(-1,1) returns 0; MulFloor(-1,1) returns -1. Keeping the policies explicit
preserves [Q8.8's](FIXED_POINT_Q8_8.md) truncation rule while supporting
algorithms such as Oscar64's Mandelbrot recurrence that require floor rounding.
Action RSH is logical; use MulFloor for this signed calculation.

MulFloor extracts bits 12..27 of the full product. After narrowing to INT this
is identical to floor division by 4096 followed by wrapping: logical and
arithmetic shifts differ only in discarded high bits. Ordinary Mul and Trunc
retain truncation toward zero.

SqrWide returns a Q8.24 raw number: SqrWide(4096)=16777216, representing 1,
and SqrWide(-32768)=1073741824, representing 64. Keep this result wide for
radius tests; shifting it right by 12 converts its scale to Q4.12, with a
separate INT conversion if the final stored value should wrap to 16 bits.

FromInt(8) wraps to MinValue, Mul(6144,8192)=12288 (1.5 times 2 is 3), and
Trunc(-6144)=-1. A whole unscaled integer 1 is not raw One: adding raw 1
changes a value by 1/4096.

Run `cargo test --test fixed_q4_12` at the root and
`cargo test --locked --test fixed_q4_12` from tools/vm-runtime-tests.
The [Oscar64 Mandelbrot sample](../samples/graphics/mandelbrot/README.md)
combines Q4.12 MulFloor and wide squares in a shared kernel, with the original
Q8.8 coordinate probe and displays that sample every physical pixel: 160x192
on a standard Atari and 320x192 with direct VBXE palette colors. See its README
for build commands, provenance, and display adaptations, and the
[implementation plan](Q4_12_MANDELBROT_IMPLEMENTATION_PLAN.md) for acceptance checks.
