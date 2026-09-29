; Native A16/X16 entry. DP: item=$80..$82, chain=$83..$85, first=$86..$88.
; Zero local frame. Incoming chain=$04..$06,S; item=$07..$09,S.
0117D1  A3 07       LDA $07,S
0117D3  85 80       STA $80
0117D5  A3 08       LDA $08,S
0117D7  85 81       STA $81
0117D9  A3 04       LDA $04,S
0117DB  85 83       STA $83
0117DD  A3 05       LDA $05,S
0117DF  85 84       STA $84
0117E1  A7 83       LDA [$83]
0117E3  85 86       STA $86
0117E5  E2 20       SEP #$20
0117E7  A0 02 00    LDY #$0002
0117EA  B7 83       LDA [$83],Y
0117EC  85 88       STA $88
0117EE  C2 20       REP #$20
0117F0  A5 83       LDA $83
0117F2  A0 03 00    LDY #$0003
0117F5  97 80       STA [$80],Y
0117F7  E2 20       SEP #$20
0117F9  A5 85       LDA $85
0117FB  C8          INY
0117FC  C8          INY
0117FD  97 80       STA [$80],Y
0117FF  C2 20       REP #$20
011801  A5 86       LDA $86
011803  87 80       STA [$80]
011805  E2 20       SEP #$20
011807  A5 88       LDA $88
011809  A0 02 00    LDY #$0002
01180C  97 80       STA [$80],Y
01180E  C2 20       REP #$20
011810  A5 80       LDA $80
011812  A0 03 00    LDY #$0003
011815  97 86       STA [$86],Y
011817  E2 20       SEP #$20
011819  A5 82       LDA $82
01181B  C8          INY
01181C  C8          INY
01181D  97 86       STA [$86],Y
01181F  C2 20       REP #$20
011821  A5 80       LDA $80
011823  87 83       STA [$83]
011825  E2 20       SEP #$20
011827  A5 82       LDA $82
011829  A0 02 00    LDY #$0002
01182C  97 83       STA [$83],Y
01182E  C2 20       REP #$20
011830  6B          RTL
