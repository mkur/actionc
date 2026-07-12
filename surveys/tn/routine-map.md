# TN Routine Map

Source: `corpora/tn/original/extracted/SRC/TN.ACT`
Original: `corpora/tn/original/extracted/TN.COM`

| Metric | Original | actionc | Delta |
| --- | ---: | ---: | ---: |
| Code segment | $2C00-$5B52 | $2C00-$5C07 | 181 bytes |
| RUNAD | $5A6F | $5B23 | 180 |

Inference anchor: last routine/RUNAD drift 180 bytes.
Original routine addresses below are inferred from internal `JSR`/`JMP` targets within +/- 64 bytes of the anchored actionc address.

| Routine | actionc | Expected original | Inferred original | Drift | Target uses |
| --- | ---: | ---: | ---: | ---: | ---: |
| r_1 | $2C02 | $2B4E |  |  | 0 |
| r_2 | $2C13 | $2B5F |  |  | 0 |
| r_3 | $2C35 | $2B81 |  |  | 0 |
| r_Mul | $2C59 | $2BA5 |  |  | 0 |
| r_Div | $2C93 | $2BDF | $2C02 | 145 | 3 |
| r_Mod | $2CF3 | $2C3F | $2C35 | 190 | 2 |
| r_Lsh | $2CFB | $2C47 | $2C59 | 162 | 3 |
| r_Rsh | $2D0A | $2C56 | $2C93 | 119 | 3 |
| r_Par | $2D19 | $2C65 |  |  | 0 |
| MovePage | $2D4A | $2C96 |  |  | 0 |
| Alloc | $2D5F | $2CAB |  |  | 0 |
| Free | $2D89 | $2CD5 | $2CF3 | 150 | 2 |
| Push | $2D9F | $2CEB | $2D0A | 149 | 1 |
| Pull | $2DC3 | $2D0F | $2D19 | 170 | 11 |
| CalcAdr | $2E0C | $2D58 | $2D5F | 173 | 5 |
| Position | $2E28 | $2D74 | $2D62 | 198 | 1 |
| Relpos | $2E2D | $2D79 | $2D89 | 164 | 2 |
| Internal | $2E38 | $2D84 | $2D8C | 172 | 1 |
| Ascii | $2E49 | $2D95 | $2D9F | 170 | 2 |
| Getchar | $2E5A | $2DA6 | $2DA2 | 184 | 1 |
| Putchar | $2E61 | $2DAD | $2DC3 | 158 | 2 |
| Print | $2E7C | $2DC8 |  |  | 0 |
| PrintE | $2EC8 | $2E14 | $2E0C | 188 | 5 |
| Block | $2EE2 | $2E2E | $2E2D | 181 | 36 |
| GetImage | $2F29 | $2E75 | $2E7C | 173 | 24 |
| PutImage | $2F5A | $2EA6 | $2E9A | 192 | 1 |
| Error | $2F8C | $2ED8 | $2ECD | 191 | 1 |
| _Cio | $2F8F | $2EDB | $2EE3 | 172 | 10 |
| _LodChn | $2F9B | $2EE7 | $2ECA | 209 | 6 |
| _StrNam | $2FC9 | $2F15 | $2F2A | 159 | 3 |
| GetD | $2FF7 | $2F43 | $2F5B | 156 | 4 |
| PutD | $300D | $2F59 | $2F8D | 128 | 1 |
| Open | $3024 | $2F70 | $2F90 | 148 | 9 |
| Close | $3040 | $2F8C | $2F9C | 164 | 8 |
| Input | $304B | $2F97 | $2FB8 | 147 | 1 |
| Bget | $306B | $2FB7 | $2FCA | 161 | 3 |
| Bput | $308B | $2FD7 | $2FCD | 190 | 1 |
| Xio | $30AB | $2FF7 | $2FF8 | 179 | 1 |
| SavePos | $30D4 | $3020 | $3024 | 176 | 1 |
| RestorePos | $30FC | $3048 | $304C | 176 | 1 |
| Window | $3123 | $306F | $306C | 183 | 1 |
| CloseWindow | $3265 | $31B1 | $31BF | 166 | 1 |
| Range | $32A4 | $31F0 | $31F2 | 178 | 1 |
| InputLine | $33AD | $32F9 | $32FC | 177 | 1 |
| Key | $34B6 | $3402 | $340D | 169 | 1 |
| Next | $3504 | $3450 | $3451 | 179 | 1 |
| Ord | $3545 | $3491 | $3486 | 191 | 1 |
| Items | $359C | $34E8 | $34FA | 162 | 5 |
| FindItem | $35DD | $3529 | $3531 | 172 | 2 |
| DrawMenu | $363F | $358B | $358A | 181 | 1 |
| MoveMenuBar | $3690 | $35DC | $35D6 | 186 | 1 |
| PopUp | $371D | $3669 | $3673 | 170 | 1 |
| Init | $3846 | $3792 | $376B | 219 | 1 |
| Strcpy | $396A | $38B6 | $38B2 | 184 | 1 |
| Strcat | $39A8 | $38F4 | $38F4 | 180 | 2 |
| Instr | $3A34 | $3980 | $399E | 150 | 2 |
| PrintFname | $3A8C | $39D8 | $39E6 | 166 | 1 |
| GetAnyKey | $3AD7 | $3A23 | $3A12 | 197 | 1 |
| Store | $3AE9 | $3A35 | $3A3F | 170 | 1 |
| Value | $3AF5 | $3A41 | $3A40 | 181 | 5 |
| CloseAll | $3B09 | $3A55 | $3A52 | 183 | 3 |
| Fnamecmp | $3B1C | $3A68 | $3A61 | 187 | 1 |
| Sort | $3BF5 | $3B41 | $3B32 | 195 | 1 |
| Path | $3D45 | $3C91 | $3C8C | 185 | 5 |
| Draw | $3DAA | $3CF6 | $3CF6 | 180 | 1 |
| Inv | $3E67 | $3DB3 | $3DAB | 188 | 1 |
| UpdDis | $3EA1 | $3DED | $3DF7 | 170 | 1 |
| DrawWinFrame | $3EB6 | $3E02 | $3E1D | 153 | 1 |
| Convert | $4000 | $3F4C | $3F4B | 181 | 1 |
| TaggedFiles | $4131 | $407D | $407C | 181 | 1 |
| IsTagged | $413E | $408A | $4079 | 197 | 2 |
| IsProtected | $4170 | $40BC | $40C9 | 167 | 1 |
| IsDirectory | $41B9 | $4105 | $40F6 | 195 | 1 |
| Tag | $4203 | $414F | $413F | 196 | 1 |
| TagAll | $42A3 | $41EF | $41EB | 184 | 1 |
| FindNext | $4313 | $425F | $4269 | 170 | 1 |
| SetWin | $43D9 | $4325 | $42FF | 218 | 1 |
| SwapWin | $48D2 | $481E | $4814 | 190 | 9 |
| Dir | $48FF | $484B | $484C | 179 | 1 |
| PrintB | $4909 | $4855 | $4854 | 181 | 1 |
| drives | $498C | $48D8 | $4900 | 140 | 1 |
| delcancel | $49B0 | $48FC |  |  | 0 |
| density | $49C1 | $490D |  |  | 0 |
| yesabort | $49E6 | $4932 |  |  | 0 |
| lockunlock | $49F3 | $493F |  |  | 0 |
| yescancel | $4A02 | $494E |  |  | 0 |
| GoTo | $4A11 | $495D | $4995 | 124 | 5 |
| Xloop | $4A3B | $4987 | $4998 | 163 | 1 |
| Delete | $4B5A | $4AA6 | $4AA2 | 184 | 1 |
| Format | $4C4D | $4B99 | $4B98 | 181 | 1 |
| NewDrive | $4DA6 | $4CF2 | $4CD8 | 206 | 1 |
| Attrib | $4EC7 | $4E13 | $4E02 | 197 | 1 |
| SwapScr | $4F46 | $4E92 | $4E84 | 194 | 2 |
| View | $501B | $4F67 | $4F84 | 151 | 3 |
| MkDir | $508E | $4FDA | $4FE6 | 168 | 1 |
| Rename | $5131 | $507D | $5099 | 152 | 1 |
| Copy | $5250 | $519C | $519D | 179 | 1 |
| Quit | $5573 | $54BF | $54B0 | 195 | 1 |
| Jmp | $55F6 | $5542 | $5555 | 161 | 1 |
| MakeJmp | $560F | $555B | $5558 | 183 | 2 |
| Handle | $5645 | $5591 | $55A8 | 157 | 2 |
| InitPannels | $5A29 | $5975 | $5975 | 180 | 2 |
| NavError | $5AA8 | $59F4 | $59F3 | 181 | 1 |
| NavInit | $5B23 | $5A6F | $5A6F | 180 | 0 |

## Tail Target Uses

| Target | Uses | Sites |
| ---: | ---: | --- |
| $5275 | 1 | JMP@$5370 |
| $528D | 1 | JMP@$5284 |
| $52DB | 1 | JMP@$527A |
| $53A5 | 1 | JMP@$538F |
| $53B2 | 1 | JMP@$5378 |
| $53D0 | 1 | JMP@$5487 |
| $5420 | 1 | JSR@$54F3 |
| $5433 | 1 | JMP@$541A |
| $548A | 1 | JMP@$53D8 |
| $54B0 | 1 | JMP@$549A |
| $54F9 | 1 | JMP@$54E8 |
| $5555 | 1 | JMP@$5534 |
| $5558 | 2 | JMP@$551A, JMP@$5552 |
| $5572 | 2 | JSR@$561E, JSR@$5821 |
| $5575 | 1 | JMP@$5572 |
| $55A8 | 2 | JSR@$5A6B, JSR@$5B4F |
| $55AB | 2 | JMP@$55A8, JMP@$5971 |
| $55B6 | 1 | JMP@$5963 |
| $55D1 | 1 | JMP@$55C9 |
| $55FC | 1 | JMP@$55EF |
| $562E | 1 | JMP@$5628 |
| $5631 | 1 | JMP@$5618 |
| $5663 | 2 | JMP@$5650, JMP@$565D |
| $5669 | 1 | JMP@$5638 |
| $5724 | 1 | JMP@$5750 |
| $5753 | 1 | JMP@$572B |
| $576D | 1 | JMP@$56AD |
| $5770 | 1 | JMP@$56A3 |
| $57A5 | 1 | JMP@$576D |
| $57A8 | 1 | JMP@$5670 |
| $57C0 | 1 | JMP@$57BA |
| $57C6 | 1 | JMP@$57B0 |
| $57E2 | 1 | JMP@$57D6 |
| $57E8 | 1 | JMP@$57CE |
| $5802 | 1 | JMP@$57EF |
| $5811 | 7 | JMP@$562E, JMP@$5666, JMP@$57A5, JMP@$57C3, JMP@$57E5, JMP@$57FF, JMP@$5809 |
| $5814 | 1 | JMP@$5610 |
| $5824 | 2 | JMP@$5811, JMP@$581B |
| $583C | 1 | JMP@$5832 |
| $585A | 2 | JMP@$584A, JMP@$5852 |
| $5870 | 1 | JMP@$5861 |
| $588C | 1 | JMP@$5877 |
| $58A6 | 1 | JMP@$58A0 |
| $58A9 | 1 | JMP@$5893 |
| $58D3 | 2 | JMP@$58B0, JMP@$58BA |
| $58E5 | 1 | JMP@$58DA |
| $592C | 1 | JMP@$58F4 |
| $592F | 1 | JMP@$58EC |
| $5952 | 1 | JMP@$594A |
| $5960 | 7 | JMP@$586D, JMP@$5889, JMP@$58A6, JMP@$58D0, JMP@$58E2, JMP@$592C, JMP@$593D |
| $5966 | 5 | JMP@$55BB, JMP@$562B, JMP@$5886, JMP@$58A3, JMP@$58CD |
| $5971 | 1 | JMP@$596B |
| $5975 | 2 | JSR@$5A50, JSR@$5B4C |
| $5978 | 1 | JMP@$5975 |
| $59A1 | 1 | JMP@$5991 |
| $59AB | 1 | JMP@$599E |
| $59AE | 1 | JMP@$5987 |
| $59B8 | 1 | JMP@$59AB |
| $59C3 | 1 | JMP@$597D |
| $59E5 | 1 | JMP@$59D7 |
| $59ED | 1 | JMP@$59E2 |
| $59F3 | 1 | JMP@$59CA |
| $5A29 | 1 | JMP@$5A1B |
| $5A43 | 1 | JMP@$5A4D |
| $5A6B | 1 | JMP@$5A58 |
| $5A6E | 1 | JMP@$59FB |
| $5A72 | 1 | JMP@$5A6F |
| $5AA2 | 1 | JSR@$5A28 |
| $5ABF | 1 | JMP@$5A9D |
| $5ACD | 1 | JMP@$5A86 |
| $5B0B | 1 | JMP@$5ADF |
| $5B45 | 1 | JMP@$5B19 |
