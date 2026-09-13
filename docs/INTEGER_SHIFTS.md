# Signed integer shifts

`USE MATH.INTEGER AS BITS` imports portable arithmetic right shifts. The module
is embedded in the compiler and loads independently of MATH's REAL facade.

| Function | Value and result | Count |
| --- | --- | --- |
| `AsrI(value,count)` | INT, signed 16-bit | BYTE |
| `AsrLI(value,count)` | LONGINT, signed 32-bit | BYTE |

Both functions shift in copies of the sign bit. Count zero returns the input.
Counts at least the value width return -1 for negative inputs and 0 otherwise.
The count range is 0–255. Argument conversions follow the ordinary Action!
integer conversion rules: wider count expressions narrow to BYTE on entry.

Arithmetic right shift rounds toward negative infinity: shifting -3 right by
one returns -2. Signed integer division truncates toward zero, returning -1 for
`-3/2`. The helpers add no rounding bias and perform no saturation.

```action
MODULE EXAMPLE
USE MATH.INTEGER AS BITS

INT small
LONGINT wide

PROC Main()
  small=BITS.AsrI(-3,1)               ; -2
  wide=BITS.AsrLI(-2147483648,31)     ; -1
  wide=BITS.AsrLI(123456,255)         ; 0
RETURN
ENDMODULE
```

Existing `RSH` remains logical, including for signed operands; it inserts zeros
and returns zero for oversized counts. `LSH` already supplies fixed-width left
shift semantics, so there are no separate ASL functions.

The implementation uses ordinary Action! operations with an INLINE hint. For a
negative input it logically shifts the nonnegative complement, then complements
the result. This handles INT and LONGINT minimum values without negating them.
There are no new language operators, compiler intrinsics or target-specific
instructions in the module. Use a separate bias when an algorithm needs rounded
rescaling, such as JPEG's Descale operation.

## Support and validation

Both functions run in compatibility, optimized classic and MIR6502 modes with
cartridge or standalone runtime. Standalone numerical execution needs no ROMs.
Raw and optimized NIR also lower to MIR68k and both MIR65816 layouts; native
execution is not yet covered by a VM.

VM tests compare against mathematical floor division, covering 35 signed value
boundaries with 37 representative counts, every BYTE count (0–255) for positive
and negative inputs, and 256 deterministic random pairs. They also cover
constant counts, nested calls and side-effecting arguments, and verify
both return widths, argument call counts, unchanged inputs and memory guards.
Every mode/runtime uses the embedded module. The loader also compiles and runs
a CRLF copy of its body under a test module name, since embedded module names
are reserved; application files exercise both LF and CRLF.

```sh
cargo test --test integer_shifts
cd tools/vm-runtime-tests
cargo test --locked --test integer_shifts
```
