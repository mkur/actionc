; Handwritten comparison, not compiler output or a replacement for TN.
; Wrapper supplies param_Strcat_s, param_Strcat_t and proc_MovePage.
; Action ABI: s=A/X, t=Y/$A3. Binary arithmetic (D=0).
; Retain ordinary parameter homes and reload them after the helper.
strcat_same_abi:
        sta param_Strcat_s
        stx param_Strcat_s+1
        sty param_Strcat_t
        lda $A3
        sta param_Strcat_t+1

        lda param_Strcat_s
        sta $AE
        stx $AF

        ldy #0
        lda $AE
        sec                     ; Include the final +1 in the low-byte add.
        adc ($AE),y
        sta $A0                 ; Capture the destination directly.
        lda $AF
        adc #0
        tax
        stx $A1

        lda param_Strcat_t
        sta $AC
        clc
        adc #1
        sta $A2
        lda param_Strcat_t+1
        sta $AD
        adc #0
        sta $A3
        lda ($AC),y             ; Y is still zero.
        sta $A4
        lda $A0
        ldy $A2
        jsr proc_MovePage

        lda param_Strcat_s      ; The copy can overwrite these homes.
        sta $AC
        lda param_Strcat_s+1
        sta $AD
        lda param_Strcat_t
        sta $AE
        lda param_Strcat_t+1
        sta $AF
        ldy #0
        lda ($AC),y
        clc
        adc ($AE),y
        sta ($AC),y
        rts
