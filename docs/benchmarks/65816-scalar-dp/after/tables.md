# Measured 65816 comparison

Each cell is **actionc / vbcc**. Code includes worker/helper bodies, checked
entries and RTL. Cycles run from worker entry through RTL. Stack is the
observed peak below entry S; caller-owned arguments and return bytes are
reported separately in `results.csv`. `INVALID` means wrong memory output.

## Optimized

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 51 / 2 | 49 / 8 | 0 / 0 |
| add | 13, 41 | 62 / 8 | 72 / 21 | 0 / 0 |
| subtract | 13, 41 | 62 / 5 | 72 / 15 | 0 / 0 |
| constant_chain | 13 | 57 / 6 | 58 / 13 | 0 / 0 |
| maximum | 13, 41 | 90 / 12 | 90 / 21 | 2 / 0 |
| wide_shift | $12345678 | 377 / 57 | 650 / 147 | 14 / 2 |
| loop_rotation | 13 | 130 / 37 | 793 / 308 | 8 / 0 |
| sum_loop | 13 | 120 / 22 | 1092 / 344 | 6 / 0 |
| recursive_sum | 13 | 191 / 28 | 2987 / 699 | 190 / 67 |
| direct_calls | 13, 41 | 311 / 24 | 436 / 67 | 14 / 3 |
| byte_sum | $12FFFC, 16 | 218 / 35 | 4009 / 630 | 18 / 0 |
| record_field | $12FFFE | 80 / 10 | 111 / 24 | 8 / 0 |
| unlink | $130040 | 127 / INVALID (77) | 186 / INVALID (163) | 0 / INVALID (0) |
| forward_copy | $130000, $12FFF8, 8 | 284 / 42 | 2982 / 406 | 18 / 0 |

## Raw

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 51 / 5 | 49 / 20 | 0 / 2 |
| add | 13, 41 | 62 / 8 | 72 / 27 | 0 / 2 |
| subtract | 13, 41 | 62 / 8 | 72 / 27 | 0 / 2 |
| constant_chain | 13 | 147 / 9 | 193 / 25 | 0 / 2 |
| maximum | 13, 41 | 90 / 17 | 90 / 33 | 2 / 2 |
| wide_shift | $12345678 | 401 / 73 | 706 / 189 | 18 / 4 |
| loop_rotation | 13 | 170 / 65 | 1221 / 531 | 18 / 10 |
| sum_loop | 13 | 146 / 32 | 1587 / 533 | 14 / 4 |
| recursive_sum | 13 | 206 / 29 | 3402 / 759 | 190 / 67 |
| direct_calls | 13, 41 | 311 / 35 | 436 / 113 | 14 / 7 |
| byte_sum | $12FFFC, 16 | 244 / 68 | 4459 / 1528 | 16 / 6 |
| record_field | $12FFFE | 80 / 25 | 111 / 68 | 8 / 4 |
| unlink | $130040 | 189 / 121 | 321 / 251 | 6 / 10 |
| forward_copy | $130000, $12FFF8, 8 | 297 / 79 | 3207 / 1090 | 18 / 4 |
