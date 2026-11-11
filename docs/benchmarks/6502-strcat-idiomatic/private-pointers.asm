; Idiomatic handwritten comparison with stronger storage assumptions.
; Wrapper supplies proc_MovePage. Incoming Action ABI is unchanged.
; $AE/$AF hold s and $AC/$AD hold t, preserved by MovePage's instructions.
; The buffers must not alias pointer/ABI scratch, the stack, parameter homes
; or code. The buffers may overlap each other. Binary arithmetic (D=0).
; This routine has no Action routine-static parameter-home side effects.
strcat_private:
        sta $AE
        stx $AF
        sty $AC
        lda $A3
        sta $AD

        ldy #0
        lda $AE
        sec
        adc ($AE),y
        pha                     ; Keep destination low while forming source.
        lda $AF
        adc #0
        tax                     ; Destination high survives the remaining setup.

        lda ($AC),y             ; Read length before changing outgoing homes.
        sta $A4
        lda $AD
        sta $A3
        ldy $AC
        iny                     ; t+1, with high-byte repair on wrap.
        bne source_ready
        inc $A3
source_ready:
        pla
        jsr proc_MovePage

        ldy #0
        lda ($AE),y              ; Reread both lengths after the backwards copy.
        clc
        adc ($AC),y
        sta ($AE),y
        rts
