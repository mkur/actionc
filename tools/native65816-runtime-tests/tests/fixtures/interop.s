; Independent ca65 caller/callees for the published v1 ABI.
; Argument offsets and literal expectations are handwritten, not obtained from
; compiler ABI calculations. Only the exported code addresses are supplied.
start:
  tsc
  sec
  sbc #1
  tcs
  sep #$20
  .a8
  lda #0
  sta 1,s
  rep #$20
  .a16
  jsl ACTION_MAIN
  tsc
  inc a
  tcs

  ; BYTE, CARD, data pointer, LONGINT: L=12, O=13, offsets 0/2/4/8.
  tsc
  sec
  sbc #13
  tcs
  lda #0
  sta 1,s
  sta 3,s
  sta 5,s
  sta 7,s
  sta 9,s
  sta 11,s
  sep #$20
  .a8
  sta 13,s
  lda #$12
  sta 1,s
  lda #$ab
  sta 7,s
  rep #$20
  .a16
  lda #$3456
  sta 3,s
  lda #$789a
  sta 5,s
  lda #$f012
  sta 9,s
  lda #$bcde
  sta 11,s
  jsl ACTION_MIXED
  tay
  tsc
  clc
  adc #13
  tcs
  tya
  sta f:$007000
  txa
  sta f:$007002

  ; All scalar result lanes, including the zero bits of narrow results.
  tsc
  dec a
  tcs
  sep #$20
  .a8
  lda #$b7
  sta 1,s
  rep #$20
  .a16
  jsl ECHO_BYTE
  tay
  tsc
  inc a
  tcs
  tya
  sta f:$007004

  .macro invoke24 target, low, bank, output
    tsc
    sec
    sbc #3
    tcs
    lda #low
    sta 1,s
    sep #$20
    .a8
    lda #bank
    sta 3,s
    rep #$20
    .a16
    jsl target
    tay
    tsc
    clc
    adc #3
    tcs
    tya
    sta f:output
    txa
    sta f:output+2
  .endmacro
  invoke24 ECHO_ADDRESS, $4567, $83, $007008
  invoke24 ECHO_SIZE, $abcd, $ef, $00700c
  invoke24 ECHO_POINTER, $789a, $ab, $007010

  tsc
  sec
  sbc #3
  tcs
  lda #$fedc
  sta 1,s
  sep #$20
  .a8
  lda #0
  sta 3,s
  rep #$20
  .a16
  jsl ECHO_WORD
  tay
  tsc
  clc
  adc #3
  tcs
  tya
  sta f:$007006

  tsc
  sec
  sbc #5
  tcs
  lda #$cdef
  sta 1,s
  lda #$89ab
  sta 3,s
  sep #$20
  .a8
  lda #0
  sta 5,s
  rep #$20
  .a16
  jsl ECHO_LONG
  tay
  tsc
  clc
  adc #5
  tcs
  tya
  sta f:$007014
  txa
  sta f:$007016
  stp
  nop

  .res $1000-(*-start), $ea
; $041000: native ABI leaf. No pushes, so local stack peak is zero.
; Snapshot all argument bytes, then destroy the entire call-clobbered DP area.
AsmMixed:
  sep #$20
  .a8
  .repeat 13, n
    lda 4+n,s
    sta f:$007100+n
  .endrepeat
  ldx #63
  lda #$5a
scratch:
  sta 0,x
  dex
  bpl scratch
  rep #$20
  .a16
  ldy #$dead
  ldx #$89ab
  lda #$cdef
  rtl
  .res $1100-(*-start), $ea
; $041100: confirms the zero-argument alignment byte is present and zero.
AsmEmpty:
  sep #$20
  .a8
  lda 4,s
  sta f:$007202
  rep #$20
  .a16
  inc $7200
  lda #$aaaa
  ldx #$bbbb
  ldy #$cccc
  rtl
  nop
