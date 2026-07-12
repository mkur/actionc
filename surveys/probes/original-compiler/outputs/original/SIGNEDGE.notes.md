# SIGNEDGE.COM observations

Source: `surveys/probes/original-compiler/signedge.act`

Original compiler output:

- `SIGNEDGE.COM`
- Atari binary load segment: `$3000..$3139`
- `RUNAD` segment: `$02E2..$02E3 = $3012`

Probe intent:

- Confirm signed `INT` comparison behavior around overflow-sensitive edges.
- Compare positive-vs-negative and negative-vs-positive orderings.
- Confirm equality/inequality shape for negative `INT` values.

Cases:

| Result | Condition | Expected truth |
| --- | --- | --- |
| `r1` | `32767 < -1` | false |
| `r2` | `-1 < 32767` | true |
| `r3` | `-32768 < 1` | true |
| `r4` | `1 < -32768` | false |
| `r5` | `32767 >= -1` | true |
| `r6` | `-1 >= 32767` | false |
| `r7` | `-1 = -1` | true |
| `r8` | `-32768 # 32767` | true |

Current actionc comparison:

- actionc generated `outputs/actionc/signedge.hex` and
  `outputs/actionc/signedge.lst`.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.
- compatible actionc now emits the original subtract/sign-flag shape for signed
  `<` / `>=`, including the original overflow behavior.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `lo` | `$3000..$3001` | `INT`, initialized to `$8000` |
| `hi` | `$3002..$3003` | `INT`, initialized to `$7FFF` |
| `neg` | `$3004..$3005` | `INT`, initialized to `$FFFF` |
| `pos` | `$3006..$3007` | `INT`, initialized to `$0001` |
| `sameNeg` | `$3008..$3009` | `INT`, initialized to `$FFFF` |
| `r1..r8` | `$300A..$3011` | result markers |
| `Main` | trampoline `$3012`, body `$3015` | `RUNAD=$3012` |

Conclusions:

- Original Action! does not emit explicit sign-difference or overflow handling
  for signed `INT <` / `>=`.
- Signed `<` is lowered as low-byte `CMP`, high-byte `SBC`, then `BMI`.
- Signed `>=` is lowered as low-byte `CMP`, high-byte `SBC`, then `BPL`.
- Compatible actionc follows this byte shape, so it also follows the original
  overflow behavior instead of the mathematically correct ordering for these
  edge cases.

Observed signed `<` shape:

```text
LDA left
CMP right
LDA left+1
SBC right+1
BMI true
```

Observed signed `>=` shape:

```text
LDA left
CMP right
LDA left+1
SBC right+1
BPL true
```

Observed original outcomes:

| Result | Condition | Mathematical truth | Original outcome |
| --- | --- | --- | --- |
| `r1` | `32767 < -1` | false | true |
| `r2` | `-1 < 32767` | true | true |
| `r3` | `-32768 < 1` | true | false |
| `r4` | `1 < -32768` | false | true |
| `r5` | `32767 >= -1` | true | false |
| `r6` | `-1 >= 32767` | false | false |

16-bit equality/inequality:

- Equality compares low bytes first, then high bytes if needed.
- Inequality branches true as soon as either byte differs.

Observed equality-ish shape for `neg = sameNeg`:

```text
LDA neg
EOR sameNeg
BNE false
ORA neg+1
EOR sameNeg+1
BEQ true
```

Observed inequality shape for `lo # hi`:

```text
LDA lo
EOR hi
BNE true
ORA lo+1
EOR hi+1
BNE true
```

The generated load file is exact against the VM capture.
