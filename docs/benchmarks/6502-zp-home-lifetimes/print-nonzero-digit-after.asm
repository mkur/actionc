; PrintNonzeroDigit, ZP home lifetimes on; 44 instruction bytes
; Optimized MIR6502, ActionCart, origin $3000; disassembled from qualified XEX.
48A8  86 A2     STX $A2
48AA  8D FA 33  STA $33FA
48AD  A9 00     LDA #$00
48AF  85 E0     STA $E0
48B1  AD FA 33  LDA $33FA
48B4  C5 A2     CMP $A2
48B6  90 0B     BCC $48C3
48B8  38        SEC
48B9  E5 A2     SBC $A2
48BB  8D FA 33  STA $33FA
48BE  E6 E0     INC $E0
48C0  4C B1 48  JMP $48B1
48C3  A5 E0     LDA $E0
48C5  F0 07     BEQ $48CE
48C7  A9 30     LDA #$30
48C9  05 E0     ORA $E0
48CB  20 32 36  JSR $3632
48CE  AD FA 33  LDA $33FA
48D1  85 A0     STA $A0
48D3  60        RTS
