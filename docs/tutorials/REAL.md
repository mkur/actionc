# Native REAL tutorial

## Contents

- [What native REAL provides](#what-native-real-provides)
- [Build your first REAL program](#build-your-first-real-program)
- [Write literals and constants](#write-literals-and-constants)
- [Calculate and compare](#calculate-and-compare)
- [Convert between integers and REAL](#convert-between-integers-and-real)
- [Pass REAL values to procedures](#pass-real-values-to-procedures)
- [Use the MATH library](#use-the-math-library)
- [Read, print, and convert REAL text](#read-print-and-convert-real-text)
- [Use arrays, pointers, and records](#use-arrays-pointers-and-records)
- [Understand precision and cost](#understand-precision-and-cost)
- [Choose a mode and runtime](#choose-a-mode-and-runtime)
- [Distinguish native REAL from Toolkit REAL](#distinguish-native-real-from-toolkit-real)
- [Troubleshoot common errors](#troubleshoot-common-errors)
- [Next examples and reference](#next-examples-and-reference)

## What native REAL provides

Native `REAL` is the Action 2027 floating-point value type. It stores an Atari
six-byte packed-decimal number and lets ordinary Action expressions perform
floating-point work:

```action
REAL x, y, result

x=1.25
y=2
result=x*y+0.5
```

The compiler supports REAL variables, constants, arithmetic, comparisons,
integer conversions, arrays, pointers, and record fields. The embedded `MATH`
and `SYS` modules add common mathematical operations, text conversion, input,
and output.

Native `REAL` is a modern-profile feature. Use `--mode optimized` for the
recommended classic backend or `--mode mir6502` to try the experimental
MIR6502 backend. The default `compatibility` mode deliberately does not add the
built-in type because historical Action! programs may define their own type
named `REAL`.

REAL arithmetic uses the floating-point package in the Atari OS ROM. This is
true for both the cartridge and standalone Action runtimes.

## Build your first REAL program

The complete example is
[`samples/real-basics.act`](../../samples/real-basics.act):

```action
MODULE REAL_BASICS

USE SYS

CONST REAL UnitPrice=1.25

BYTE quantity
REAL subtotal, total

PROC Main()
  quantity=4
  subtotal=UnitPrice*quantity
  total=subtotal+0.5

  SYS.PrintE("Total:")
  SYS.PrintRE(@total)
RETURN

ENDMODULE
```

Compile it without an Action! cartridge dependency:

```sh
actionc --mode optimized --runtime standalone \
  --output build/real-basics.xex samples/real-basics.act
```

Or compile and launch it in a configured emulator:

```sh
actionc-run --mode optimized --no-cart samples/real-basics.act
```

`quantity` is a `BYTE`, but multiplication by the REAL constant promotes it to
REAL automatically. `SYS.PrintRE` expects the address of a REAL, so the call
uses `@total`. The final `E` in `PrintRE` means that it also writes an
end-of-line.

If module syntax and `USE` are new to you, the
[modules tutorial](MODULES.md) introduces them independently.

## Write literals and constants

A number containing a decimal point or decimal exponent is a REAL literal:

```action
REAL fraction, small, large

fraction=0.125
small=4E-3
large=6.02E23
```

An integer literal stays an integer until context promotes it. Both assignments
below produce the same REAL value:

```action
REAL first, second

first=2
second=2.0
```

Use a decimal point or exponent when a source number is outside the Action
integer range:

```action
REAL population

population=1234567890.0
```

Typed constants provide immutable REAL values and can refer to earlier typed
REAL constants:

```action
CONST REAL Pi=3.141592654,
           TwoPi=6.283185307,
           FullTurn=TwoPi
```

A typed REAL constant initializer is a signed REAL literal or an earlier
`CONST REAL`, not a general arithmetic expression. Calculate other derived
values in executable code.

`CONST REAL` values have value semantics but no address of their own. Assign a
constant to a variable before passing it to a pointer-oriented routine:

```action
REAL angle

angle=Pi
```

## Calculate and compare

REAL expressions support binary `+`, `-`, `*`, and `/`, along with unary `+`
and `-`. Either operand may be an integer; it is promoted before the operation:

```action
BYTE count
REAL unitPrice, discount, total

count=3
unitPrice=2.5
discount=0.25
total=unitPrice*count-discount
```

The normal compound assignments work too:

```action
REAL balance, interest

balance==+interest
balance==*1.05
```

Comparisons produce an ordinary condition and may also mix REAL and integer
operands:

```action
REAL temperature

IF temperature<0 THEN
  temperature=-temperature
FI
```

Equality uses `=` and inequality uses `<>`, as elsewhere in Action!. `<=`,
`>`, and `>=` are also supported. A REAL used directly as a condition is false
when it is zero and true otherwise.

`MOD`, bitwise operators, and shifts are integer operations and reject REAL
operands. Convert deliberately or reformulate the calculation when one is
needed.

## Convert between integers and REAL

Assignment and mixed arithmetic promote `BYTE`, `CHAR`, `CARD`, and `INT`
values to REAL automatically:

```action
INT delta
REAL position

delta=-12
position=position+delta
```

The other direction must be explicit. Cast a REAL expression to the required
integer type:

```action
REAL measured
INT nearest
BYTE level

measured=12.6
nearest=INT(measured)
level=BYTE(measured)
```

A dynamic conversion follows Atari FPI behavior: it rounds to the nearest
integer, with an exact half rounded away from zero. The result must fit the
selected Action integer type.

The compiler diagnoses a statically known fractional or out-of-range cast
instead of silently changing it:

```action
; Both are compile-time errors.
nearest=INT(1.5)
level=BYTE(256.0)
```

When the source is intentionally dynamic, store or compute it in a REAL
variable before the cast. Use `MATH.Floor` when you need rounding toward
negative infinity rather than nearest-integer conversion.

## Pass REAL values to procedures

REAL values currently use a pointer-oriented procedure interface. By-value
REAL parameters and REAL function results are not supported yet. Declare
`REAL POINTER` parameters and pass addresses with `@`:

```action
PROC AddTax(REAL POINTER amount, taxRate, destination)
  destination^=amount^*(1+taxRate^)
RETURN

REAL price, rate, priceWithTax

PROC Main()
  price=19.95
  rate=0.23
  AddTax(@price,@rate,@priceWithTax)
RETURN
```

`amount^` dereferences a pointer to read its REAL value, and `destination^=`
writes all six bytes of the result. A separate destination parameter is the
REAL equivalent of a function result.

Do not write a by-value declaration such as `PROC Show(REAL value)`. The
compiler rejects it with a diagnostic suggesting `REAL POINTER`.

## Use the MATH library

Import `MATH` for portable, pointer-oriented numerical procedures and useful
constants:

```action
MODULE ROOT_EXAMPLE

USE SYS
USE MATH

REAL input, root, angle, sine

PROC Main()
  input=2
  MATH.Sqrt(@input,@root)

  angle=MATH.QuarterPi
  MATH.Sin(@angle,@sine)

  SYS.PrintRE(@root)
  SYS.PrintRE(@sine)
RETURN

ENDMODULE
```

The main procedure groups are:

- powers and logarithms: `Exp`, `Exp10`, `Ln`, `Log10`, and `Power`;
- elementary operations: `Abs`, `Sgn`, `Floor`, `Rnd`, and `Sqrt`/`Sqr`;
- trigonometry: `Sin`, `Cos`, `Tan`, and `Atan`/`Atn`.

Trigonometric arguments and results use radians. Constants include `Pi`,
`HalfPi`, `QuarterPi`, `TwoPi`, `E`, `Ln2`, `Ln10`, `Sqrt2`, `DegToRad`, and
`RadToDeg`. For example, convert degrees before calling `Sin`:

```action
angle=degrees*MATH.DegToRad
MATH.Sin(@angle,@sine)
```

Applications normally use `MATH`, not its Atari-specific provider module.
These routines do not need an Atari BASIC cartridge, but they do use the Atari
OS floating-point package.

## Read, print, and convert REAL text

The modern REAL I/O procedures are qualified members of `SYS`, so import the
module and keep the `SYS.` prefix:

```action
USE SYS

REAL value
STRING text(20)

PROC Main()
  SYS.Print("Enter a number: ")
  SYS.InputR(@value)

  SYS.StrR(@value,text)
  SYS.PrintE(text)
  SYS.PrintRE(@value)
RETURN
```

The available groups are:

- `PrintR` and `PrintRE` for the default device;
- `PrintRD` and `PrintRDE` for an explicit device number;
- `InputR` and `InputRD` for input;
- `StrR` to format a REAL into a string;
- `ValR` to parse a string into a REAL.

Reserve at least 20 bytes for a `StrR` destination. `ValR` and the input
routines expose the Atari OS conversion behavior for invalid text; validate
input at the application level when recovery matters.

Unlike traditional system-library names such as unqualified `PrintE`, REAL
I/O names are deliberately qualified-only. `USE SYS` plus `SYS.PrintRE` avoids
colliding with procedures in older source libraries.

## Use arrays, pointers, and records

Each REAL array element occupies six bytes. Arrays may be initialized with
integer and REAL values:

```action
REAL ARRAY samples(4)=[1.0 2.5 -3 4E-2]
REAL selected
BYTE index

index=2
selected=samples(index)
```

REAL pointers use the usual Action address and dereference syntax:

```action
REAL ARRAY values(3)
REAL POINTER current

current=values
current^=1.25
```

A record can contain REAL fields at their normal six-byte storage width:

```action
TYPE Reading=[BYTE channel REAL value BYTE status]

Reading latest

latest.channel=1
latest.value=18.75
latest.status=0
```

Global and local REAL variables, initialized storage, absolute storage, array
elements, pointer dereferences, and record fields all copy as complete REAL
values rather than as scalar byte or word fragments.

## Understand precision and cost

The Atari representation has a base-100 exponent and five packed-BCD mantissa
bytes. In practical terms:

- a REAL occupies six bytes;
- it keeps up to ten significant decimal digits;
- source conversion is deterministic and does not pass through host `f32` or
  `f64` values;
- the useful magnitude range is approximately `1E-98` through `1E99`;
- a literal below the representable range becomes zero, while an overflowing
  literal is a compile-time error.

Digits beyond the stored precision are discarded when a source literal is
encoded. Intermediate arithmetic follows the Atari OS FPP behavior, so do not
use host IEEE binary floating-point results as a bit-exact oracle.

REAL is substantially more expensive than integer arithmetic on a 6502. Most
arithmetic operations copy six-byte operands through the OS floating-point
workspace and call a ROM routine. Prefer `BYTE`, `CARD`, or `INT` for counters,
indexes, flags, and calculations whose range and scaling are naturally
integral.

The OS floating-point workspace is shared. Native REAL operations and the
`MATH`/`SYS` REAL procedures are not reentrant and are not safe against an
interrupt handler that also uses FPP. Keep FPP work out of such handlers, or
provide application-level exclusion around it.

## Choose a mode and runtime

The recommended native REAL build uses optimized mode:

```sh
actionc --mode optimized program.act
```

The experimental MIR6502 path supports the same language surface:

```sh
actionc --mode mir6502 program.act
```

The lower-level equivalent of optimized mode is `--profile modern --backend
classic`. You may also put a leading source annotation in a maintained program
when modern is always the intended profile:

```action
;@actionc profile modern
```

Runtime choice is independent of native REAL:

```sh
actionc --mode optimized --runtime cart program.act
actionc --mode optimized --runtime standalone program.act
```

`--runtime standalone` removes the Action! cartridge dependency. It does not
remove the Atari OS dependency selected by REAL arithmetic, conversion, and
math. Both runtime choices therefore need a compatible Atari OS ROM or
implementation.

## Distinguish native REAL from Toolkit REAL

The historical Action! Toolkit library defines a record type named `REAL` and
implements pointer-oriented procedures around that record:

```action
TYPE REAL=[CARD r1,r2,r3]
```

Native REAL is a compiler-provided modern type with the same six-byte Atari
data format, but it is not that source record and does not require
`INCLUDE "REAL.ACT"`. Including or declaring a nearer type named `REAL`
shadows the built-in symbol because `REAL` intentionally remains an identifier,
not a reserved keyword.

This preserves old source behavior. For new native-REAL code, omit the Toolkit
include, select a modern mode, use operators for core arithmetic, and import
`MATH` and qualified `SYS` procedures for the library surface.

## Troubleshoot common errors

### `unknown type REAL`

Select `--mode optimized` or `--mode mir6502`. Compatibility mode does not
install the native type. Also check that a local declaration has not shadowed
the built-in name.

### `by-value REAL parameters are not supported`

Change the parameter to `REAL POINTER`, pass `@value`, and dereference it with
`^`. Return computed REAL values through a destination pointer.

### `REAL requires an explicit conversion`

Use `BYTE(value)`, `CHAR(value)`, `CARD(value)`, or `INT(value)` as appropriate.
Then account for nearest-integer rounding and the target type's range.

### A REAL operator is rejected

Only `+`, `-`, `*`, `/`, unary signs, comparisons, and their supported
assignment forms apply directly to REAL. `MOD`, shifts, and bitwise operations
require integers.

### A `SYS` REAL procedure is unknown

Add `USE SYS` and call the qualified name, such as `SYS.PrintRE(@value)`.
These modern extensions are not added as unqualified compatibility aliases.

### Standalone output still needs an OS ROM

Standalone selects an Action runtime provider; it does not replace Atari FPP.
REAL arithmetic and the first-party REAL libraries still call the compatible
Atari OS floating-point package.

## Next examples and reference

- [`samples/real-basics.act`](../../samples/real-basics.act) is the complete
  introductory program from this tutorial.
- [`samples/modules/native-real-library.act`](../../samples/modules/native-real-library.act)
  uses `MATH.Exp10`, `SYS.StrR`, and REAL output.
- [`samples/graphics/sine-surface.act`](../../samples/graphics/sine-surface.act)
  is a larger graphics program using native REAL expressions, conversions,
  `MATH.Sqrt`, and `MATH.Sin`.
- [Modules and runtime usage](../Action_2027/MODULES_AND_RUNTIME_USAGE.md#native-real-library)
  is the detailed first-party library and runtime reference.
- [Native REAL implementation plan](../Action_2027/REAL_TYPE_IMPLEMENTATION_PLAN.md)
  records the representation, language contract, and compiler architecture.
- [Command-line usage](../../USAGE.md) documents compiler modes, runtimes, and
  source annotations.
