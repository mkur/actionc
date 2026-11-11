; ===== PROC Strcat $3874..$38E5 =====
proc_Strcat:
        STA.A param_Strcat_s                 ; $3874: 8D BC 2E
        STX.A param_Strcat_s+1               ; $3877: 8E BD 2E
        STY.A param_Strcat_t                 ; $387A: 8C BE 2E
        LDA.Z $A3                            ; $387D: A5 A3
        STA.A param_Strcat_t+1               ; $387F: 8D BF 2E
        LDA.A param_Strcat_s                 ; $3882: AD BC 2E
        STA.Z $AE                            ; $3885: 85 AE
        STX.Z $AF                            ; $3887: 86 AF
        LDY #$00                             ; $3889: A0 00
        LDA.Z $AE                            ; $388B: A5 AE
        CLC                                  ; $388D: 18
        ADC ($AE),Y                          ; $388E: 71 AE
        TAX                                  ; $3890: AA
        LDA.Z $AF                            ; $3891: A5 AF
        ADC #$00                             ; $3893: 69 00
        INX                                  ; $3895: E8
        STX.Z $AC                            ; $3896: 86 AC
        STA.Z $AD                            ; $3898: 85 AD
        BNE loc_Strcat_1                     ; $389A: D0 02
        INC.Z $AD                            ; $389C: E6 AD
loc_Strcat_1:
        LDA.Z $AC                            ; $389E: A5 AC
        STA.Z $A0                            ; $38A0: 85 A0
        LDA.Z $AD                            ; $38A2: A5 AD
        STA.Z $A1                            ; $38A4: 85 A1
        TAX                                  ; $38A6: AA
        LDA.A param_Strcat_t                 ; $38A7: AD BE 2E
        STA.Z $AC                            ; $38AA: 85 AC
        CLC                                  ; $38AC: 18
        ADC #$01                             ; $38AD: 69 01
        STA.Z $A2                            ; $38AF: 85 A2
        LDA.A param_Strcat_t+1               ; $38B1: AD BF 2E
        STA.Z $AD                            ; $38B4: 85 AD
        ADC #$00                             ; $38B6: 69 00
        STA.Z $A3                            ; $38B8: 85 A3
        LDY #$00                             ; $38BA: A0 00
        LDA ($AC),Y                          ; $38BC: B1 AC
        STA.Z $A4                            ; $38BE: 85 A4
        LDA.Z $A0                            ; $38C0: A5 A0
        LDY.Z $A2                            ; $38C2: A4 A2
        JSR.A proc_MovePage                  ; $38C4: 20 28 2F ; MovePage
        LDA.A param_Strcat_s                 ; $38C7: AD BC 2E
        STA.Z $AC                            ; $38CA: 85 AC
        LDA.A param_Strcat_s+1               ; $38CC: AD BD 2E
        STA.Z $AD                            ; $38CF: 85 AD
        LDA.A param_Strcat_t                 ; $38D1: AD BE 2E
        STA.Z $AE                            ; $38D4: 85 AE
        LDA.A param_Strcat_t+1               ; $38D6: AD BF 2E
        STA.Z $AF                            ; $38D9: 85 AF
        LDY #$00                             ; $38DB: A0 00
        LDA ($AC),Y                          ; $38DD: B1 AC
        CLC                                  ; $38DF: 18
        ADC ($AE),Y                          ; $38E0: 71 AE
        STA ($AC),Y                          ; $38E2: 91 AC
        RTS                                  ; $38E4: 60
; ===== END PROC Strcat =====
