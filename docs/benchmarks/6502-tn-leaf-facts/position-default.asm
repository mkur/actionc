; Position
3002  85 FF     STA $FF
3004  86 FE     STX $FE
3006  60        RTS

; Check
3007  A9 00     LDA #$00
3009  A2 80     LDX #$80
300B  20 02 30  JSR $3002
300E  D0 05     BNE $3015
3010  A9 01     LDA #$01
3012  85 A0     STA $A0
3014  60        RTS
3015  A9 00     LDA #$00
3017  85 A0     STA $A0
3019  60        RTS

; Main
301A  60        RTS

