# RETURNS.COM observations

Source: `surveys/probes/original-compiler/returns.act`

Original compiler output:

- `RETURNS.COM`
- Atari binary load segment: `$3000..$30DD`
- `RUNAD` segment: `$02E2..$02E3 = $307D`

Probe intent:

- Confirm function return ABI for:
  - `BYTE`
  - `CHAR`
  - `CARD`
  - `INT`
- Confirm signed return representation for negative `INT` values.
- Confirm how callers copy return values into globals.
- Confirm return ABI for functions with arguments.

Cases:

- `RetB()` returns byte value `18` / `$12`.
- `RetCh()` returns character `'A`.
- `RetC()` returns card value `$1234`.
- `RetI()` returns int value `-2`.
- `IncB(BYTE x)` returns `x + 1`.
- `AddC(CARD a,b)` returns `a + b`.
- `NegI(INT x)` returns `0 - x`.

Observed ABI shape:

- `BYTE` / `CHAR` return values use `$A0`.
- `CARD` / `INT` return values use `$A0` low byte and `$A1` high byte.
- Literal `CARD` / `INT` returns store the high byte first, then the low byte.
- Callers copy multi-byte return values high-byte-first, while preserving
  little-endian memory layout.
- Parameter frames with 1 or 2 argument bytes use direct stores.
- The 4-byte `CARD,CARD` frame uses the original `SArgs` prologue shape.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `b` | `$3000` | global `BYTE` |
| `bx` | `$3001` | global `BYTE` |
| `ch` | `$3002` | global `CHAR` |
| `w` | `$3003..$3004` | global `CARD` |
| `wx` | `$3005..$3006` | global `CARD` |
| `i` | `$3007..$3008` | global `INT` |
| `ix` | `$3009..$300A` | global `INT` |
| `RetB` | trampoline `$300B`, body `$300E` | returns `$12` in `$A0` |
| `RetCh` | trampoline `$3013`, body `$3016` | returns `$41` in `$A0` |
| `RetC` | trampoline `$301B`, body `$301E` | returns `$1234` in `$A1/$A0` |
| `RetI` | trampoline `$3027`, body `$302A` | returns `$FFFE` in `$A1/$A0` |
| `IncB` | param `$3033`, trampoline `$3034`, body `$3037` | direct 1-byte prologue |
| `AddC` | params `$3043..$3046`, trampoline `$3047`, body `$304A` | `SArgs`, metadata `$43 $30 $03` |
| `NegI` | params `$3062..$3063`, trampoline `$3064`, body `$3067` | direct 2-byte prologue |
| `Main` | trampoline `$307D`, body `$3080` | `RUNAD=$307D` |

Return instruction examples:

```text
RetC:
  LDA #$12
  STA $A1
  LDA #$34
  STA $A0
  RTS

RetI:
  LDA #$FF
  STA $A1
  LDA #$FE
  STA $A0
  RTS
```

Caller copy examples:

```text
JSR RetC
LDA $A1
STA w+1
LDA $A0
STA w

JSR RetI
LDA $A1
STA i+1
LDA $A0
STA i
```

Current actionc comparison:

- actionc returns byte/char through `$A0`.
- actionc returns card/int through `$A0/$A1`.
- actionc caller copies return bytes from `$A0+` into the destination.
- actionc emits low-byte-first stores for multi-byte literal returns and caller
  copies; original emits high-byte-first for these cases.
- actionc emits direct parameter prologues for all three parameterized
  functions in this probe; original uses direct prologues for `IncB` and `NegI`
  but `SArgs` for `AddC`.

Open follow-up:

- Confirm the exact `SArgs` threshold with a targeted 2-byte / 3-byte function
  probe. Current evidence suggests 1-2 argument bytes are direct, while 3+
  argument bytes use `SArgs`.
