# BOOLS.COM observations

Source: `surveys/probes/original-compiler/bools.act`

Original compiler output:

- `BOOLS.COM`
- Atari binary load segment: `$3000..$3120`
- `RUNAD` segment: `$02E2..$02E3 = $3012`

Probe intent:

- Capture original branch code shape for equality and inequality.
- Capture unsigned `CARD` comparison branch shape.
- Capture signed `INT` comparison branch shape.
- Capture condition treatment for scalar `AND` and `OR` expressions.

Current actionc comparison:

- actionc now generates `outputs/actionc/bools.hex`,
  `outputs/actionc/bools.lst`, and `outputs/actionc/bools.com`.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.
- The bitwise `AND`/`OR` condition fragments match the original local shape:
  compute the bitwise result, store it in `$AE`, reload `$AE`, then branch on
  non-zero.
- Byte equality uses `EOR`, signed `<`/`>=` use subtract/sign-flag branching,
  unsigned `CARD` comparisons use subtract/carry branching, and bitwise
  conditions use `$AE` materialization as in the original.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `a` | `$3000` | global byte |
| `b` | `$3001` | global byte |
| `eqv` | `$3002` | equality result marker |
| `nev` | `$3003` | inequality result marker |
| `ltu` | `$3004` | unsigned less-than marker |
| `geu` | `$3005` | unsigned greater/equal marker |
| `lts` | `$3006` | signed less-than marker |
| `ges` | `$3007` | signed greater/equal marker |
| `andv` | `$3008` | bitwise `AND` condition marker |
| `orv` | `$3009` | bitwise `OR` condition marker |
| `cu` | `$300A..$300B` | global `CARD` |
| `cv` | `$300C..$300D` | global `CARD` |
| `si` | `$300E..$300F` | global `INT` |
| `sj` | `$3010..$3011` | global `INT` |
| `Main` | trampoline `$3012`, body `$3015` | `RUNAD=$3012` |

Observed condition shapes:

Byte equality uses `EOR` and branches on zero:

```text
LDA a
EOR b
BEQ true
```

Byte inequality uses the same `EOR` and branches on non-zero:

```text
LDA a
EOR b
BNE true
```

Unsigned `CARD` comparison uses low-byte `CMP`, then high-byte `SBC`, carrying
the low-byte comparison into the high-byte subtract:

```text
LDA cu
CMP cv
LDA cu+1
SBC cv+1
BCC unsigned_less_true
```

Unsigned `>=` uses the same subtract shape and branches on carry set:

```text
LDA cu
CMP cv
LDA cu+1
SBC cv+1
BCS unsigned_greater_equal_true
```

Current compatible `actionc` uses this same local shape for unsigned 16-bit
comparisons.

Signed `INT` comparison uses the same two-byte subtract shape, but branches on
the high-byte result sign. This probe does not cover signed overflow edge cases.

```text
LDA si
CMP sj
LDA si+1
SBC sj+1
BMI signed_less_true
```

Signed `>=` branches on plus:

```text
LDA si
CMP sj
LDA si+1
SBC sj+1
BPL signed_greater_equal_true
```

Scalar `AND` / `OR` in an `IF` condition are bitwise expressions, not
short-circuit control-flow operators. Original Action! materializes the result
in `$AE`, reloads it, and branches on non-zero:

```text
LDA a
AND b
STA $AE
LDA $AE
BNE true

LDA a
ORA b
STA $AE
LDA $AE
BNE true
```

Questions to answer from original output:

- Confirm signed comparison overflow behavior with a dedicated edge-case probe,
  e.g. `-32768 < 1`, `32767 < -1`, and equal negative values.
- Add a separate byte/card equality probe if byte-for-byte equality shape for
  16-bit `=` / `#` becomes important.
