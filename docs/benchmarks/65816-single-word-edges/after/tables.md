# Measured 65816 comparison

Each cell is **actionc / vbcc**. Code includes worker/helper bodies, checked
entries and RTL. Cycles run from worker entry through RTL. Stack is the
observed peak below entry S; caller-owned arguments and return bytes are
reported separately in `results.csv`. `INVALID` means wrong memory output.

## Optimized

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 63 / 2 | 71 / 8 | 4 / 0 |
| add | 13, 41 | 74 / 8 | 98 / 21 | 8 / 0 |
| subtract | 13, 41 | 74 / 5 | 98 / 15 | 8 / 0 |
| constant_chain | 13 | 71 / 6 | 86 / 13 | 6 / 0 |
| maximum | 13, 41 | 110 / 12 | 111 / 21 | 6 / 0 |
| wide_shift | $12345678 | 379 / 57 | 653 / 147 | 14 / 2 |
| loop_rotation | 13 | 186 / 37 | 1314 / 308 | 26 / 0 |
| sum_loop | 13 | 146 / 22 | 1595 / 344 | 16 / 0 |
| recursive_sum | 13 | 211 / 28 | 3291 / 699 | 190 / 67 |
| direct_calls | 13, 41 | 329 / 24 | 500 / 67 | 20 / 3 |
| byte_sum | $12FFFC, 16 | 244 / 35 | 4476 / 630 | 22 / 0 |
| record_field | $12FFFE | 82 / 10 | 114 / 24 | 8 / 0 |
| unlink | $130040 | 129 / INVALID (77) | 189 / INVALID (163) | 0 / INVALID (0) |
| forward_copy | $130000, $12FFF8, 8 | 310 / 42 | 3225 / 406 | 18 / 0 |

## Raw

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 63 / 5 | 71 / 20 | 4 / 2 |
| add | 13, 41 | 74 / 8 | 98 / 27 | 8 / 2 |
| subtract | 13, 41 | 74 / 8 | 98 / 27 | 8 / 2 |
| constant_chain | 13 | 191 / 9 | 311 / 25 | 6 / 2 |
| maximum | 13, 41 | 110 / 17 | 111 / 33 | 6 / 2 |
| wide_shift | $12345678 | 403 / 73 | 709 / 189 | 18 / 4 |
| loop_rotation | 13 | 212 / 65 | 1604 / 531 | 18 / 10 |
| sum_loop | 13 | 174 / 32 | 2032 / 533 | 14 / 4 |
| recursive_sum | 13 | 226 / 29 | 3701 / 759 | 190 / 67 |
| direct_calls | 13, 41 | 329 / 35 | 500 / 113 | 20 / 7 |
| byte_sum | $12FFFC, 16 | 272 / 68 | 5003 / 1528 | 16 / 6 |
| record_field | $12FFFE | 82 / 25 | 114 / 68 | 8 / 4 |
| unlink | $130040 | 191 / 121 | 324 / 251 | 6 / 10 |
| forward_copy | $130000, $12FFF8, 8 | 321 / 79 | 3405 / 1090 | 18 / 4 |
