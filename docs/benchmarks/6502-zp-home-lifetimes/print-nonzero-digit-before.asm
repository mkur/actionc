; PrintNonzeroDigit, ZP home lifetimes off; 47 instruction bytes
; Optimized MIR6502, ActionCart, origin $3000; disassembled from qualified XEX.
48A9  8E FB 33  STX $33FB
48AC  8D FA 33  STA $33FA
48AF  A9 00     LDA #$00
48B1  85 E0     STA $E0
48B3  AD FA 33  LDA $33FA
48B6  CD FB 33  CMP $33FB
48B9  90 0C     BCC $48C7
48BB  38        SEC
48BC  ED FB 33  SBC $33FB
48BF  8D FA 33  STA $33FA
48C2  E6 E0     INC $E0
48C4  4C B3 48  JMP $48B3
48C7  A5 E0     LDA $E0
48C9  F0 07     BEQ $48D2
48CB  A9 30     LDA #$30
48CD  05 E0     ORA $E0
48CF  20 33 36  JSR $3633
48D2  AD FA 33  LDA $33FA
48D5  85 A0     STA $A0
48D7  60        RTS
