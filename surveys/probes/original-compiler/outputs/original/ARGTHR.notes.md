# ARGTHR.COM observations

Source: `surveys/probes/original-compiler/argthr.act`

Original compiler output:

- `ARGTHR.COM`
- Atari binary load segment: `$3000..$30C4`
- `RUNAD` segment: `$02E2..$02E3 = $306F`

Probe intent:

- Confirm the function parameter-frame threshold for direct prologues versus
  `SArgs`.
- Compare 2-byte and 3-byte argument frames across `BYTE` and `CARD` functions.
- Confirm mixed scalar argument byte order remains left-to-right and
  little-endian.

Cases:

| Function | Argument bytes | Expected question |
| --- | --- | --- |
| `TwoBytes(BYTE a,b)` | 2 | direct stores or `SArgs`? |
| `ThreeBytes(BYTE a,b,c)` | 3 | first likely `SArgs` threshold |
| `OneCard(CARD a)` | 2 | direct stores for a 2-byte scalar? |
| `ByteCard(BYTE a,CARD b)` | 3 | mixed 3-byte frame behavior |
| `CardByte(CARD a,BYTE b)` | 3 | mixed 3-byte frame behavior |

Observed layout:

| Function | Storage / entry | Prologue |
| --- | --- | --- |
| `TwoBytes(BYTE a,b)` | params `$3008..$3009`, trampoline `$300A`, body `$300D` | direct stores |
| `ThreeBytes(BYTE a,b,c)` | params `$3019..$301B`, trampoline `$301C`, body `$301F` | `SArgs`, metadata `$19 $30 $02` |
| `OneCard(CARD a)` | params `$302B..$302C`, trampoline `$302D`, body `$3030` | direct stores |
| `ByteCard(BYTE a,CARD b)` | params `$3041..$3043`, trampoline `$3044`, body `$3047` | `SArgs`, metadata `$41 $30 $02` |
| `CardByte(CARD a,BYTE b)` | params `$3058..$305A`, trampoline `$305B`, body `$305E` | `SArgs`, metadata `$58 $30 $02` |
| `Main` | trampoline `$306F`, body `$3072` | `RUNAD=$306F` |

Conclusion:

- Original Action! uses direct callee stores for 1-byte and 2-byte argument
  frames.
- Original Action! switches to `SArgs` for 3-byte argument frames.
- The threshold appears to depend on flattened byte count, not source type
  spelling:
  - `BYTE,BYTE,BYTE` uses `SArgs`.
  - `BYTE,CARD` uses `SArgs`.
  - `CARD,BYTE` uses `SArgs`.
  - single `CARD` stays direct.

Direct 2-byte examples:

```text
TwoBytes:
  STX second_param
  STA first_param

OneCard:
  STX param_high
  STA param_low
```

3-byte `SArgs` examples:

```text
ThreeBytes:
  JSR $A0F5
  .BYTE $19, $30, $02

ByteCard:
  JSR $A0F5
  .BYTE $41, $30, $02

CardByte:
  JSR $A0F5
  .BYTE $58, $30, $02
```

Current actionc comparison:

- actionc emits direct parameter stores for all cases in this probe.
- actionc caller passes argument bytes in `A`, `X`, `Y` as expected.
- actionc should switch to original-style `SArgs` for 3-or-more-byte parameter
  frames if byte-for-byte compatibility is desired.
