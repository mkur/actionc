# STRPASS.COM observations

Source: `surveys/probes/original-compiler/strpass.act`

Original compiler output needed:

- `STRPASS.COM`
- Atari binary load segment: `$3000..$309C`
- `RUNAD` segment: `$02E2..$02E3 = $3088`

Probe intent:

- Confirm `STRING` alias parameter ABI.
- Confirm `CHAR ARRAY` parameter ABI for initialized string arrays.
- Confirm whether `STRING` parameters are just `CHAR ARRAY` two-byte base
  pointers.
- Confirm dynamic indexing through a string parameter.

Current actionc comparison:

- Original-first probe for now.
- actionc parses/analyzes the source, but codegen does not yet support string
  / named `CHAR ARRAY` indexed reads or user calls with named array arguments.
- No actionc hex/listing was generated.

Questions to answer from original output:

- Does `Take(hello, 4)` pass the string storage base pointer in `A/X`? Yes.
- Does `TakeRaw(raw)` use the same array-pointer ABI? Yes.
- Does callee string indexing use `$AE/$AF` and `($AE),Y`? Yes.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `hello` | `$3000..$3005` | `$05 "HELLO"` |
| `raw` | `$3006..$300A` | `$04 "DATA"` |
| `a0` | `$300B` | result byte |
| `a1` | `$300C` | result byte |
| `ai` | `$300D` | result byte |
| `b0` | `$300E` | result byte |
| `b1` | `$300F` | result byte |
| `Take.s` | `$3010..$3011` | string pointer parameter |
| `Take.i` | `$3012` | byte parameter |
| `Take` | trampoline `$3013`, body `$3016` | uses `SArgs` |
| `TakeRaw.s` | `$3057..$3058` | array pointer parameter |
| `TakeRaw` | trampoline `$3059`, body `$305C` | direct 2-byte prologue |
| `Main` | trampoline `$3088`, body `$308B` | `RUNAD=$3088` |

Call sites:

```text
; Take(hello, 4)
LDY #$04
LDX #$30
LDA #$00
JSR $3013

; TakeRaw(raw)
LDX #$30
LDA #$06
JSR $3059
```

`Take` has three argument bytes, so its entry jumps into an `SArgs` prologue:

```text
$3013: JMP $3016
$3016: JSR $A0F5
$3019: .BYTE $10,$30,$02 ; frame=$3010, arg_bytes_minus_1=2
```

After `SArgs`, `Take.s` is stored at `$3010..$3011` and `Take.i` at `$3012`.

`TakeRaw` has only a two-byte pointer argument, so it uses direct stores:

```text
$305C: STX $3058
$305F: STA $3057
```

Conclusions:

- `STRING` is ABI-compatible with `CHAR ARRAY` at call boundaries. Both are
  passed as a two-byte base pointer, low byte in `A`, high byte in `X`.
- A string/character-array initializer stores a length byte at element 0.
  `CHAR ARRAY raw(4)="DATA"` reserves/stores `$04 "DATA"` at `$3006..$300A`.
- Constant indexing and dynamic indexing through a string parameter both copy
  the effective address to `$AE/$AF`, set `Y=0`, then read with `LDA ($AE),Y`.
- Dynamic byte indexing uses `base_low + index` with carry into the high byte:

```text
CLC
LDA $3010
ADC $3012
STA $AE
LDA $3011
ADC #$00
STA $AF
LDA ($AE),Y
```
