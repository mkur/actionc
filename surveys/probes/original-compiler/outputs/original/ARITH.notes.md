# ARITH.COM observations

Source: `surveys/probes/original-compiler/arith.act`

Original compiler output:

- `ARITH.COM`
- Atari binary load segment: `$3000..$3128`
- `RUNAD` segment: `$02E2..$02E3 = $300E`

Probe intent:

- Capture original code shape for byte and card bitwise operators.
- Capture original code shape for `CARD` add/sub and one-bit shifts.
- Capture runtime helper setup for signed `INT` multiply/divide/modulo.
- Compare operand staging in `$84/$85` and result use from `A/X`.

Current actionc comparison:

- actionc generated `outputs/actionc/arith.hex` and
  `outputs/actionc/arith.lst`.
- actionc uses the cartridge helper addresses for compatible multiply,
  divide, modulo, left shift, and right shift.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `b` | `$3000` | global `BYTE` |
| `c` | `$3001` | global `BYTE` |
| `xw` | `$3002..$3003` | global `CARD` |
| `yw` | `$3004..$3005` | global `CARD` |
| `cw` | `$3006..$3007` | global `CARD` |
| `si` | `$3008..$3009` | global `INT` |
| `sj` | `$300A..$300B` | global `INT` |
| `sk` | `$300C..$300D` | global `INT` |
| `Main` | trampoline `$300E`, body `$3011` | `RUNAD=$300E` |

Conclusions:

- Byte bitwise operations are direct `AND` / `ORA` / `EOR` on `A`.
- `CARD` add/sub and bitwise operations are inlined byte-by-byte.
- `CARD` constant assignment uses high-byte-first stores.
- `CARD` add/sub result stores are low-byte-first in this probe.
- `CARD` bitwise result stores are low-byte-first in this probe.
- Original uses runtime helper calls for `CARD LSH 1` and `CARD RSH 1`; it does
  not inline the single-bit shifts here. Current compatible `actionc` now does
  the same.
- Signed `INT` multiply/divide/modulo use the same operand/result ABI as the
  standalone runtime helper block:
  - left operand low/high in `A/X`,
  - right operand low/high in `$84/$85`,
  - result low/high returned in `A/X`.

Observed helper calls:

| Operation | Helper call | Notes |
| --- | --- | --- |
| `cw = xw LSH 1` | `JSR $B5C0` | count in `$84` |
| `cw = xw RSH 1` | `JSR $A0E6` | count in `$84` |
| `sk = si * sj` | `JSR $A000` | same as actionc compatible multiply |
| `sk = si / sj` | `JSR $A090` | cartridge-resident div helper |
| `sk = si MOD sj` | `JSR $A0DE` | cartridge-resident mod/rem helper |

Helper call setup shape:

```text
; right operand sj -> $84/$85, high first
LDA sj+1
STA $85
LDA sj
STA $84

; left operand si -> A/X
LDA si+1
TAX
LDA si

JSR helper

; result A/X -> sk, low first
STA sk
TXA
STA sk+1
```

Shift helper setup shape:

```text
LDA #$01
STA $84
LDA xw+1
TAX
LDA xw
JSR shift_helper
STA cw
TXA
STA cw+1
```

Questions to answer from original output:

- Confirm whether the cartridge helper addresses `$B5C0`, `$A0E6`, `$A090`,
  and `$A0DE` are stable across runtime/library configurations, or whether
  saved standalone builds should target the `$04E4..$04EC` vector table.

Current diff classification:

- VM output is `$3000-$3128`; current `actionc` output is also `$3000-$3128`.
- After routing compatible constant shifts through the cartridge helpers, the
  code length matches the original.
- Compatible constant stores now use `Y` for stable byte/value-1 and high-byte
  0/1 cases. The executable code now matches the original ARITH probe.
- Remaining differences are storage/initial memory bytes.
- Compatible runtime helper setup now uses direct `LDX #imm` for constant left
  high bytes, matching the helper setup shape used by the original compiler.
