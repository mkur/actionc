; ABI-compatible comparison implementation, not compiler-generated output.
; ca65 input, assembled in native A16/I16 state at $018000.
.include "action65816-native-v1.inc"
item = A816_DP_POINTER0_OFFSET
pred = A816_DP_POINTER1_OFFSET
succ = A816_DP_POINTER2_OFFSET
arg = A816_CALL_ENTRY_FIRST_ARGUMENT_OFFSET

; Keep the compiler's complete checked-entry sequence, with a zero-byte frame.
; Use its current inverse-branch/JML encoding for an equivalent comparison.
        tsc
        tax
        cmp A816_DP_STACK_CEILING_OFFSET
        bcs :+
        jml within
:       bne :+
        jml within
:       jml fault
within: sec
        sbc #0
        bcs :+
        jml fault
:       cmp A816_DP_STACK_FLOOR_OFFSET
        bcc fault
        jml body
fault:  lda #0
        jml $048000
body:   tcs

; Copy exactly three incoming bytes, at 4,S through 6,S, into private scratch.
        lda arg,s
        sta item
        lda arg+1,s
        sta item+1
        sep #$20
.a8

; pred = item->ln_Pred
        ldy #3
        lda [item],y
        sta pred
        iny
        lda [item],y
        sta pred+1
        iny
        lda [item],y
        sta pred+2

; succ = item->ln_Succ
        ldy #0
        lda [item],y
        sta succ
        iny
        lda [item],y
        sta succ+1
        iny
        lda [item],y
        sta succ+2

; pred->ln_Succ = succ
        ldy #0
        lda succ
        sta [pred],y
        iny
        lda succ+1
        sta [pred],y
        iny
        lda succ+2
        sta [pred],y

; succ->ln_Pred = pred
        ldy #3
        lda pred
        sta [succ],y
        iny
        lda pred+1
        sta [succ],y
        iny
        lda pred+2
        sta [succ],y

; Action! PROC: no result. No frame to release.
        rep #$20
.a16
        rtl
