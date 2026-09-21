# Measured 65816 comparison

Each cell is **actionc / vbcc**. Code includes worker/helper bodies, checked
entries and RTL. Cycles run from worker entry through RTL. Stack is the
observed peak below entry S; caller-owned arguments and return bytes are
reported separately in `results.csv`. `INVALID` means wrong memory output.

## Optimized

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 87 / 2 | 108 / 8 | 4 / 0 |
| add | 13, 41 | 116 / 8 | 162 / 21 | 8 / 0 |
| subtract | 13, 41 | 116 / 5 | 162 / 15 | 8 / 0 |
| constant_chain | 13 | 112 / 6 | 148 / 13 | 6 / 0 |
| maximum | 13, 41 | 234 / 12 | 210 / 21 | 6 / 0 |
| wide_shift | $12345678 | 379 / 57 | 653 / 147 | 14 / 2 |
| loop_rotation | 13 | 401 / 37 | 2624 / 308 | 26 / 0 |
| sum_loop | 13 | 311 / 22 | 3542 / 344 | 16 / 0 |
| recursive_sum | 13 | 368 / 28 | 5363 / 699 | 190 / 67 |
| direct_calls | 13, 41 | 412 / 24 | 688 / 67 | 20 / 3 |
| byte_sum | $12FFFC, 16 | 405 / 35 | 6747 / 630 | 22 / 0 |
| record_field | $12FFFE | 106 / 10 | 151 / 24 | 8 / 0 |
| unlink | $130040 | 129 / INVALID (77) | 189 / INVALID (163) | 0 / INVALID (0) |
| forward_copy | $130000, $12FFF8, 8 | 410 / 42 | 4023 / 406 | 18 / 0 |

## Raw

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 87 / 5 | 108 / 20 | 4 / 2 |
| add | 13, 41 | 116 / 8 | 162 / 27 | 8 / 2 |
| subtract | 13, 41 | 116 / 8 | 162 / 27 | 8 / 2 |
| constant_chain | 13 | 427 / 9 | 658 / 25 | 6 / 2 |
| maximum | 13, 41 | 234 / 17 | 210 / 33 | 6 / 2 |
| wide_shift | $12345678 | 403 / 73 | 709 / 189 | 18 / 4 |
| loop_rotation | 13 | 385 / 65 | 2634 / 531 | 18 / 10 |
| sum_loop | 13 | 313 / 32 | 3617 / 533 | 14 / 4 |
| recursive_sum | 13 | 380 / 29 | 5717 / 759 | 190 / 67 |
| direct_calls | 13, 41 | 412 / 35 | 688 / 113 | 20 / 7 |
| byte_sum | $12FFFC, 16 | 411 / 68 | 6930 / 1528 | 16 / 6 |
| record_field | $12FFFE | 106 / 25 | 151 / 68 | 8 / 4 |
| unlink | $130040 | 191 / 121 | 324 / 251 | 6 / 10 |
| forward_copy | $130000, $12FFF8, 8 | 418 / 79 | 4167 / 1090 | 18 / 4 |
