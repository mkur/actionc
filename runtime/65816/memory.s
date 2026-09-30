; Reentrant native-v2 memory helpers. Included in the host's code segment.
; Entry/exit: native, M=X=0, decimal clear. D/DBR/S/I are unchanged.
; Scratch $80..$8f belongs to the calling domain; IRQ/NMI must use their own DP.
; No pushes or calls: these leaves need neither a stack frame nor a guard.
; Long indirect indexed word accesses carry through bank boundaries. Chunks
; stop before Y wraps; odd tails access exactly one byte, never a neighbor.
.ifndef A816_ABI_VERSION
    .include "action65816-native-v2.inc"
.endif
.smart
.export a816_memory_move, a816_memory_move_end
.export a816_memory_clear, a816_memory_clear_end
.export a816_memory_fill, a816_memory_fill_end
.a16
.i16

AM_DEST = A816_DP_SCRATCH_OFFSET
AM_SOURCE = AM_DEST+3
AM_LEFT = AM_DEST+6
AM_CHUNK = AM_DEST+10
AM_VALUE = AM_DEST+12
AM_BACK = AM_DEST+14

.macro am_pointer target, slot
    lda slot,s
    sta target
    sep #$20
    lda slot+2,s
    sta target+2
    rep #$20
.endmacro

.macro am_add pointer, amount
    lda pointer
    clc
    adc amount
    sta pointer
    sep #$20
    lda pointer+2
    adc #0
    sta pointer+2
    rep #$20
.endmacro

.macro am_sub pointer, amount
    lda pointer
    sec
    sbc amount
    sta pointer
    sep #$20
    lda pointer+2
    sbc #0
    sta pointer+2
    rep #$20
.endmacro

.macro am_add_length pointer
    lda pointer
    clc
    adc AM_LEFT
    sta pointer
    sep #$20
    lda pointer+2
    adc AM_LEFT+2
    sta pointer+2
    rep #$20
.endmacro

a816_memory_move:
    am_pointer AM_DEST, 4
    am_pointer AM_SOURCE, 7
    am_pointer AM_LEFT, 10
    lda #0
    sta AM_BACK
    lda AM_DEST+2
    and #$ff
    sta AM_CHUNK
    lda AM_SOURCE+2
    and #$ff
    cmp AM_CHUNK
    bcc am_backward
    bne am_move_chunk
    lda AM_DEST
    cmp AM_SOURCE
    beq am_move_return
    bcc am_move_chunk
am_backward:
    inc AM_BACK
    am_add_length AM_DEST
    am_add_length AM_SOURCE
am_move_chunk:
    lda AM_LEFT+2
    and #$ff
    bne am_move_large
    lda AM_LEFT
    bne am_move_count
am_move_return:
    rtl
am_move_large:
    lda #$fffe
am_move_count:
    sta AM_CHUNK
    tax
    lda AM_BACK
    bne am_move_reverse
    ldy #0
am_move_forward_word:
    cpx #2
    bcc am_move_forward_tail
    lda [AM_SOURCE],y
    sta [AM_DEST],y
    iny
    iny
    dex
    dex
    bne am_move_forward_word
am_move_forward_tail:
    cpx #0
    beq am_move_forward_done
    sep #$20
    lda [AM_SOURCE],y
    sta [AM_DEST],y
    rep #$20
am_move_forward_done:
    am_add AM_DEST, AM_CHUNK
    am_add AM_SOURCE, AM_CHUNK
    bra am_move_progress
am_move_reverse:
    am_sub AM_DEST, AM_CHUNK
    am_sub AM_SOURCE, AM_CHUNK
    ldy AM_CHUNK
am_move_reverse_word:
    cpx #2
    bcc am_move_reverse_tail
    dey
    dey
    lda [AM_SOURCE],y
    sta [AM_DEST],y
    dex
    dex
    bne am_move_reverse_word
am_move_reverse_tail:
    cpx #0
    beq am_move_progress
    dey
    sep #$20
    lda [AM_SOURCE],y
    sta [AM_DEST],y
    rep #$20
am_move_progress:
    am_sub AM_LEFT, AM_CHUNK
    brl am_move_chunk
a816_memory_move_end:

a816_memory_clear:
    lda #0
    sta AM_VALUE
    bra am_fill_setup
a816_memory_clear_end:
; Clear tail-enters Fill's shared body without an additional return address.
a816_memory_fill:
    lda 7,s
    and #$ff
    sta AM_VALUE
    xba
    ora AM_VALUE
    sta AM_VALUE
am_fill_setup:
    am_pointer AM_DEST, 4
    am_pointer AM_LEFT, 8
am_fill_chunk:
    lda AM_LEFT+2
    and #$ff
    bne am_fill_large
    lda AM_LEFT
    bne am_fill_count
    rtl
am_fill_large:
    lda #$fffe
am_fill_count:
    sta AM_CHUNK
    tax
    ldy #0
    lda AM_VALUE
am_fill_word:
    cpx #2
    bcc am_fill_tail
    sta [AM_DEST],y
    iny
    iny
    dex
    dex
    bne am_fill_word
am_fill_tail:
    cpx #0
    beq am_fill_progress
    sep #$20
    sta [AM_DEST],y
    rep #$20
am_fill_progress:
    am_add AM_DEST, AM_CHUNK
    am_sub AM_LEFT, AM_CHUNK
    bra am_fill_chunk
a816_memory_fill_end:
