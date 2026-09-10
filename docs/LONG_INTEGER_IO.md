# 32-bit SYS conversion and I/O

Use `USE SYS` and qualified names for the `LONGCARD` (`LC`, unsigned) and
`LONGINT` (`LI`, signed) families. On Atari these routines support Compatibility,
Optimized classic and MIR6502, with either `--runtime cart` or
`--runtime standalone`.

| Operation | LONGCARD | LONGINT |
| --- | --- | --- |
| Print decimal | `SYS.PrintLC(value)` | `SYS.PrintLI(value)` |
| Print decimal and newline | `SYS.PrintLCE(value)` | `SYS.PrintLIE(value)` |
| Print to device | `SYS.PrintLCD(device,value)` | `SYS.PrintLID(device,value)` |
| Print to device and newline | `SYS.PrintLCDE(device,value)` | `SYS.PrintLIDE(device,value)` |
| Format into a string | `SYS.StrLC(value,destination)` | `SYS.StrLI(value,destination)` |
| Parse a string | `SYS.ValLC(source)` | `SYS.ValLI(source)` |
| Read a decimal line | `SYS.InputLC()` | `SYS.InputLI()` |
| Read a decimal line from a device | `SYS.InputLCD(device)` | `SYS.InputLID(device)` |

Values and function results are passed as full 32-bit integers. Device numbers
are BYTEs. Default-device routines use the current `DEVICE` (normally channel
0); `D` variants use the supplied channel. Newlines are ATASCII EOL (`$9B`).
The usual SYS character/string I/O rules and error reporting still apply.

```action
MODULE LONG_IO_EXAMPLE
USE SYS

PROC Main()
  LONGCARD count
  LONGINT balance
  STRING text(12)

  count=SYS.ValLC("4294967295")
  balance=SYS.ValLI("-2147483648")
  SYS.PrintLCE(count)
  SYS.PrintLIE(balance)
  SYS.StrLI(balance,text)
  SYS.PrintE(text)
RETURN
ENDMODULE
```

## String conversion

Strings have an Action! length byte, not a NUL terminator. Formatting produces
decimal digits without leading zeros, with a minus sign only for negative
LONGINT values. Zero prints as `0`; both signed limits and the unsigned maximum
are supported.

Reserve **11 bytes for StrLC** or **12 bytes for StrLI**, including the length
byte: `STRING unsignedText(11)` or `STRING signedText(12)`. Only the length byte
and actual characters are written; unused trailing storage is unchanged. As
with existing SYS string routines, the caller must provide sufficient space.

`ValLC` and `ValLI` accept surrounding ASCII spaces, an optional leading `+`,
and one or more decimal digits. `ValLI` also accepts `-`, including `-0`.
`ValLC` rejects any minus sign. Leading zeros are accepted, and the complete
length-prefixed string is checked, up to 255 characters.

Empty input, a sign without digits, internal spaces, trailing junk, NULs,
hexadecimal notation and decimal points are rejected. Parsing checks the
range **before** each multiply/add:

- `ValLC`: 0 through 4294967295.
- `ValLI`: -2147483648 through 2147483647.

Malformed text invokes **Error(102)**; well-formed text outside the destination
range invokes **Error(103)**. Syntax takes precedence: `4294967296x` reports 102,
not 103. The unsigned parser rejects any minus sign as invalid syntax (102).
See the [error-code table](ATARI_RUNTIME_ERRORS.md).
If a custom handler returns, a defensive guard clears decimal mode and loops;
the conversion does not return a partial/wrapped result or resume the caller.
These are checked conversions, not replacements for the historical `ValC` and
`ValI` parsing behavior. A nonfatal `TryVal` API is not included.

## Input

Input routines read one record through existing SYS line input, then use the
same checked parser. They accept up to 254 characters followed by ATASCII EOL.
A full buffer without EOL invokes **Error(104)** before parsing a truncated prefix.
The shared parser still accepts all 255 characters of a supplied string.

## Runtime and compatibility

The existing `PrintC`, `PrintI`, `StrC`, `StrI`, `ValC`, `ValI`, input variants,
and `PrintF` retain their 16-bit signatures. In particular, `PrintF` does not
gain wide arguments or new format specifiers. Passing a LONG to a narrow
integer parameter still narrows to its low bits; select an LC/LI routine to
retain all 32 bits.

The LC/LI names are not added to the implicit compatibility prelude. Importing
SYS selects only the routines used. Both classic profiles and MIR6502 preserve
the wide ABI, including when the argument is a small literal.

Conversion code lives in the compiler-owned `SYSLONG.ACT` runtime unit.
Standalone wrappers use the existing standalone string I/O; cartridge wrappers
in `SYSLONGC.ACT` call the cartridge's character/string entries. Cartridge ROM
entry points are unchanged. The existing dependency graph selects only the
needed routines, and arithmetic reuses the normal compiler helpers.

The Atari Error behavior is unchanged: the cartridge normally reports the
error; standalone delegates to DOSVEC and does not promise printed diagnostics.
See [Atari runtime errors](ATARI_RUNTIME_ERRORS.md).

For a complete demonstration, see
[`samples/long-integers/`](../samples/long-integers/).
