# Measured 65816 comparison

Each cell is **actionc / vbcc**. Code includes worker/helper bodies, checked
entries and RTL. Cycles run from worker entry through RTL. Stack is the
observed peak below entry S; caller-owned arguments and return bytes are
reported separately in `results.csv`. `INVALID` means wrong memory output.

## Optimized

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 87 / 2 | 108 / 8 | 4 / 0 |
| add | 13, 41 | 98 / 8 | 135 / 21 | 8 / 0 |
| subtract | 13, 41 | 98 / 5 | 135 / 15 | 8 / 0 |
| constant_chain | 13 | 95 / 6 | 123 / 13 | 6 / 0 |
| maximum | 13, 41 | 234 / 12 | 210 / 21 | 6 / 0 |
| wide_shift | $12345678 | 379 / 57 | 653 / 147 | 14 / 2 |
| loop_rotation | 13 | 338 / 37 | 2223 / 308 | 26 / 0 |
| sum_loop | 13 | 276 / 22 | 2866 / 344 | 16 / 0 |
| recursive_sum | 13 | 333 / 28 | 4687 / 699 | 190 / 67 |
| direct_calls | 13, 41 | 377 / 24 | 611 / 67 | 20 / 3 |
| byte_sum | $12FFFC, 16 | 374 / 35 | 6011 / 630 | 22 / 0 |
| record_field | $12FFFE | 106 / 10 | 151 / 24 | 8 / 0 |
| unlink | $130040 | 129 / INVALID (77) | 189 / INVALID (163) | 0 / INVALID (0) |
| forward_copy | $130000, $12FFF8, 8 | 393 / 42 | 3823 / 406 | 18 / 0 |

## Raw

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 87 / 5 | 108 / 20 | 4 / 2 |
| add | 13, 41 | 98 / 8 | 135 / 27 | 8 / 2 |
| subtract | 13, 41 | 98 / 8 | 135 / 27 | 8 / 2 |
| constant_chain | 13 | 215 / 9 | 348 / 25 | 6 / 2 |
| maximum | 13, 41 | 234 / 17 | 210 / 33 | 6 / 2 |
| wide_shift | $12345678 | 403 / 73 | 709 / 189 | 18 / 4 |
| loop_rotation | 13 | 316 / 65 | 2182 / 531 | 18 / 10 |
| sum_loop | 13 | 278 / 32 | 2941 / 533 | 14 / 4 |
| recursive_sum | 13 | 345 / 29 | 5041 / 759 | 190 / 67 |
| direct_calls | 13, 41 | 377 / 35 | 611 / 113 | 20 / 7 |
| byte_sum | $12FFFC, 16 | 376 / 68 | 6098 / 1528 | 16 / 6 |
| record_field | $12FFFE | 106 / 25 | 151 / 68 | 8 / 4 |
| unlink | $130040 | 191 / 121 | 324 / 251 | 6 / 10 |
| forward_copy | $130000, $12FFF8, 8 | 401 / 79 | 3967 / 1090 | 18 / 4 |
