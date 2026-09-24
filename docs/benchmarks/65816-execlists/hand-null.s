.p816
.a16
.i16
.segment "CODE"
.export CapturedDP, CapturedStack, Indirect, Adapter
.proc CapturedDP
    lda $00
    ora $01
    beq null
    lda #0
    rtl
null:
    lda #1
    rtl
.endproc
.proc CapturedStack
    lda 4,s
    ora 5,s
    beq null
    lda #0
    rtl
null:
    lda #1
    rtl
.endproc
.proc Indirect
    ldy #1
    lda [$00],y
    ora [$00]
    beq null
    lda #0
    rtl
null:
    lda #1
    rtl
.endproc
Adapter:
    pei ($02)
    pei ($00)
    jsl CapturedStack
    plx
    plx
    rtl
