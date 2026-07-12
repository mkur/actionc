# RECORDS.COM observations

Source: `surveys/probes/original-compiler/records.act`

Original compiler output needed:

- `RECORDS.COM`
- Atari binary load segment: `$3000..$3085`
- `RUNAD` segment: `$02E2..$02E3 = $306B`

Probe intent:

- Confirm `TYPE` record field packing and offsets.
- Confirm global record storage layout.
- Confirm `TYPE POINTER` parameter passing and field access lowering.
- Confirm address passing for a record value when passed to a `TYPE POINTER`
  parameter.

Source revision note:

- The first version used `TYPE Pair=[BYTE tag CARD word CHAR POINTER ptr]`.
- Original Action! rejected that declaration with error 6, "Declaration error.
  Wrong declaration format."
- The current probe removes the pointer field and keeps a `Pair POINTER`
  parameter. This should isolate record value layout and pointer-to-record field
  access first.

Current actionc comparison:

- actionc now supports packed record storage and direct record field
  loads/stores for fundamental fields.
- actionc now supports pointer-to-record field access through `$AE/$AF` and
  `($AE),Y`.
- actionc now supports record-value-to-record-pointer argument passing:
  `Touch(rec)` passes the address of `rec`.
- The full `records.act` probe now compiles.
- A reduced direct-record smoke test emits `Pair rec` as three packed bytes and
  uses absolute loads/stores for `rec.tag` and `rec.word`.

Questions to answer from original output:

- Are fields packed in declaration order without padding? Yes.
- What offsets are assigned to `tag` and `word`? `tag=0`, `word=1`.
- Does `Touch(Pair POINTER rp)` receive `rec` as a two-byte pointer argument?
  Yes, low byte in `A`, high byte in `X`.
- Does field access use pointer temporary locations or direct absolute indexed
  addressing? Pointer field access uses `$AE/$AF` plus `($AE),Y`.
- Are pointer fields inside `TYPE` records unsupported by original Action!, or
  was a different declaration spelling needed? The manual grammar confirms
  `TYPE` fields are fundamental variable declarations only, so pointer fields
  are not normal record fields.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `gb` | `$3000` | result byte |
| `gw` | `$3001..$3002` | result card |
| `gp` | `$3003..$3004` | `CHAR POINTER` storage |
| `data` | `$3005..$3008` | inline `BYTE ARRAY data(4)` |
| `rec` | `$3009..$300B` | `Pair`, packed size 3 |
| `Touch.rp` | `$300C..$300D` | record pointer parameter |
| `Touch` | trampoline `$300E`, body `$3011` | direct 2-byte prologue |
| `Main` | trampoline `$306B`, body `$306E` | `RUNAD=$306B` |

Record field offsets:

| Field | Offset | Size |
| --- | --- | --- |
| `tag` | 0 | 1 byte |
| `word` | 1 | 2 bytes |

Call site:

```text
; Touch(rec)
LDX #$30
LDA #$09
JSR $300E
```

`Touch(Pair POINTER rp)` direct prologue:

```text
STX $300D
STA $300C
```

Representative field lowering:

```text
; rp.tag = $11
LDA $300C
STA $AE
LDA $300D
STA $AF
LDA #$11
LDY #$00
STA ($AE),Y

; rp.word = $2233
CLC
LDA $300C
ADC #$01
STA $AE
LDA $300D
ADC #$00
STA $AF
LDA #$22
INY
STA ($AE),Y
LDA #$33
DEY
STA ($AE),Y
```

Conclusions:

- Record fields are packed in declaration order without padding.
- A global record value reserves its packed byte size inline in the global
  storage area.
- Passing a record value to a `TYPE POINTER` parameter passes the record base
  address. In this probe `rec` is passed as `$3009`.
- Pointer-to-record field access computes `base + field_offset` into `$AE/$AF`
  and then uses `($AE),Y`.
- Multi-byte fields remain little-endian in storage. The original compiler
  often emits the high-byte instruction first (`Y=1`) and then the low-byte
  instruction (`Y=0`), matching other CARD/INT pointer access patterns.
- The manual confirms that normal `TYPE` records contain fundamental fields
  only. Pointer fields, array fields, and arrays of records are handled via
  manual "virtual record" layouts rather than direct declarations.
