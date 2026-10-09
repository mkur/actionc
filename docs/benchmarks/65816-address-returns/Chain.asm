; optimized-release
; M_MYDOSFILE_CHAIN_94A5D226 — 21 bytes, 12 instructions, zero frame/spills/peak
045DA2  A3 04        LDA $04,S
045DA4  18           CLC
045DA5  69 52 00     ADC #$0052
045DA8  A8           TAY
045DA9  E2 20        SEP #$20
045DAB  A3 06        LDA $06,S
045DAD  69 00        ADC #$00
045DAF  C2 20        REP #$20
045DB1  29 FF 00     AND #$00FF
045DB4  AA           TAX
045DB5  98           TYA
045DB6  6B           RTL

; optimized-guarded
; M_MYDOSFILE_CHAIN_94A5D226 — 21 bytes, 12 instructions, zero frame/spills/peak
057FB2  A3 04        LDA $04,S
057FB4  18           CLC
057FB5  69 52 00     ADC #$0052
057FB8  A8           TAY
057FB9  E2 20        SEP #$20
057FBB  A3 06        LDA $06,S
057FBD  69 00        ADC #$00
057FBF  C2 20        REP #$20
057FC1  29 FF 00     AND #$00FF
057FC4  AA           TAX
057FC5  98           TYA
057FC6  6B           RTL

; raw-guarded
; M_MYDOSFILE_CHAIN_94A5D226 — 21 bytes, 12 instructions, zero frame/spills/peak
05FB2E  A3 04        LDA $04,S
05FB30  18           CLC
05FB31  69 52 00     ADC #$0052
05FB34  A8           TAY
05FB35  E2 20        SEP #$20
05FB37  A3 06        LDA $06,S
05FB39  69 00        ADC #$00
05FB3B  C2 20        REP #$20
05FB3D  29 FF 00     AND #$00FF
05FB40  AA           TAX
05FB41  98           TYA
05FB42  6B           RTL
