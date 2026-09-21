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
| maximum | 13, 41 | 146 / 12 | 149 / 21 | 6 / 0 |
| wide_shift | $12345678 | 379 / 57 | 653 / 147 | 14 / 2 |
| loop_rotation | 13 | 277 / 37 | 2000 / 308 | 26 / 0 |
| sum_loop | 13 | 213 / 22 | 2465 / 344 | 16 / 0 |
| recursive_sum | 13 | 247 / 28 | 3819 / 699 | 190 / 67 |
| direct_calls | 13, 41 | 329 / 24 | 500 / 67 | 20 / 3 |
| byte_sum | $12FFFC, 16 | 311 / 35 | 5532 / 630 | 22 / 0 |
| record_field | $12FFFE | 82 / 10 | 114 / 24 | 8 / 0 |
| unlink | $130040 | 129 / INVALID (77) | 189 / INVALID (163) | 0 / INVALID (0) |
| forward_copy | $130000, $12FFF8, 8 | 354 / 42 | 3589 / 406 | 18 / 0 |

## Raw

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 63 / 5 | 71 / 20 | 4 / 2 |
| add | 13, 41 | 74 / 8 | 98 / 27 | 8 / 2 |
| subtract | 13, 41 | 74 / 8 | 98 / 27 | 8 / 2 |
| constant_chain | 13 | 191 / 9 | 311 / 25 | 6 / 2 |
| maximum | 13, 41 | 146 / 17 | 149 / 33 | 6 / 2 |
| wide_shift | $12345678 | 403 / 73 | 709 / 189 | 18 / 4 |
| loop_rotation | 13 | 256 / 65 | 1968 / 531 | 18 / 10 |
| sum_loop | 13 | 218 / 32 | 2596 / 533 | 14 / 4 |
| recursive_sum | 13 | 262 / 29 | 4229 / 759 | 190 / 67 |
| direct_calls | 13, 41 | 329 / 35 | 500 / 113 | 20 / 7 |
| byte_sum | $12FFFC, 16 | 316 / 68 | 5687 / 1528 | 16 / 6 |
| record_field | $12FFFE | 82 / 25 | 114 / 68 | 8 / 4 |
| unlink | $130040 | 191 / 121 | 324 / 251 | 6 / 10 |
| forward_copy | $130000, $12FFF8, 8 | 365 / 79 | 3769 / 1090 | 18 / 4 |
