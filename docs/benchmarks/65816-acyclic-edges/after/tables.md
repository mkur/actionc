# Measured 65816 comparison

Each cell is **actionc / vbcc**. Code includes worker/helper bodies, checked
entries and RTL. Cycles run from worker entry through RTL. Stack is the
observed peak below entry S; caller-owned arguments and return bytes are
reported separately in `results.csv`. `INVALID` means wrong memory output.

## Optimized

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 59 / 2 | 63 / 8 | 4 / 0 |
| add | 13, 41 | 70 / 8 | 90 / 21 | 8 / 0 |
| subtract | 13, 41 | 70 / 5 | 90 / 15 | 8 / 0 |
| constant_chain | 13 | 65 / 6 | 73 / 13 | 6 / 0 |
| maximum | 13, 41 | 90 / 12 | 94 / 21 | 6 / 0 |
| wide_shift | $12345678 | 377 / 57 | 650 / 147 | 14 / 2 |
| loop_rotation | 13 | 148 / 37 | 1116 / 308 | 26 / 0 |
| sum_loop | 13 | 120 / 22 | 1212 / 344 | 16 / 0 |
| recursive_sum | 13 | 191 / 28 | 2987 / 699 | 190 / 67 |
| direct_calls | 13, 41 | 319 / 24 | 466 / 67 | 20 / 3 |
| byte_sum | $12FFFC, 16 | 218 / 35 | 4009 / 630 | 22 / 0 |
| record_field | $12FFFE | 80 / 10 | 111 / 24 | 8 / 0 |
| unlink | $130040 | 127 / INVALID (77) | 186 / INVALID (163) | 0 / INVALID (0) |
| forward_copy | $130000, $12FFF8, 8 | 284 / 42 | 2982 / 406 | 18 / 0 |

## Raw

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 59 / 5 | 63 / 20 | 4 / 2 |
| add | 13, 41 | 70 / 8 | 90 / 27 | 8 / 2 |
| subtract | 13, 41 | 70 / 8 | 90 / 27 | 8 / 2 |
| constant_chain | 13 | 155 / 9 | 223 / 25 | 6 / 2 |
| maximum | 13, 41 | 90 / 17 | 94 / 33 | 6 / 2 |
| wide_shift | $12345678 | 401 / 73 | 706 / 189 | 18 / 4 |
| loop_rotation | 13 | 172 / 65 | 1226 / 531 | 18 / 10 |
| sum_loop | 13 | 146 / 32 | 1587 / 533 | 14 / 4 |
| recursive_sum | 13 | 208 / 29 | 3467 / 759 | 190 / 67 |
| direct_calls | 13, 41 | 319 / 35 | 466 / 113 | 20 / 7 |
| byte_sum | $12FFFC, 16 | 244 / 68 | 4459 / 1528 | 16 / 6 |
| record_field | $12FFFE | 80 / 25 | 111 / 68 | 8 / 4 |
| unlink | $130040 | 189 / 121 | 321 / 251 | 6 / 10 |
| forward_copy | $130000, $12FFF8, 8 | 297 / 79 | 3207 / 1090 | 18 / 4 |
