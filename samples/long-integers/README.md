# 32-bit integer examples

[`long-integers.act`](long-integers.act) demonstrates unsigned `LONGCARD` and
signed `LONGINT` with factorial, Fibonacci, signed division/remainder, the full
32-bit limits, and explicit widening before arithmetic. It uses the standard
`SYS.PrintLCE`/`SYS.PrintLIE` decimal printers; `PrintC`/`PrintI` still take only
16-bit arguments. See the [32-bit SYS I/O reference](../../docs/LONG_INTEGER_IO.md)
for printing, string conversion, and input routines.

Atari execution requires MIR6502; both standalone and cartridge-linked runtimes
are covered. Build from the repository root:

```sh
actionc samples/long-integers/long-integers.act --mode mir6502 --runtime standalone \
  --output build/long-integers.xex
```

Load the resulting XEX in an Atari emulator or on an Atari. Expected output:

```text
LONGCARD / LONGINT (32-bit)

LONGCARD: unsigned
0! = 1
12! = 479001600
F(0) = 0
F(47) = 2971215073
maximum = 4294967295

LONGINT: signed
-F(46) = -1836311903
minimum = -2147483648
maximum = 2147483647
-70000 / 3 = -23333
-70000 MOD 3 = -1

Widen before arithmetic:
CARD $FFFF+1 = 0
LONGCARD($FFFF)+1 = 65536
```

The routines are exact for factorial inputs 0–12 and Fibonacci inputs 0–47;
larger results overflow 32 bits. The final two lines show that a wide destination
does not widen a narrow expression: cast an operand **before** the addition.
