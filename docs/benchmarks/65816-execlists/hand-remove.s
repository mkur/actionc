.p816
.a16
.i16
.segment "CODE"
.export Remove, Adapter
Remove:
    lda $04,s
    sta $00
    lda $05,s
    sta $01

    ldy #$0004
    lda [$00],y
    sta $04
    dey
    lda [$00],y
    sta $03

    ldy #$0001
    lda [$00],y
    tax
    lda [$00]
    sta [$03]
    sta $00
    txa
    sta $01
    sta [$03],y

    lda $03
    ldy #$0003
    sta [$00],y
    lda $04
    iny
    sta [$00],y
    rtl

; Test adapter: Calypsi argument lanes -> Action stack argument.
; Its fixed 36-cycle cost is excluded from the reported routine timing.
Adapter:
    pei ($02)
    pei ($00)
    jsl Remove
    pla
    pla
    rtl
