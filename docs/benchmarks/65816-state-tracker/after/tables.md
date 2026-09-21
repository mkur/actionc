# Measured 65816 comparison

Each cell is **actionc / vbcc**. Code includes worker/helper bodies, checked
entries and RTL. Cycles run from worker entry through RTL. Stack is the
observed peak below entry S; caller-owned arguments and return bytes are
reported separately in `results.csv`. `INVALID` means wrong memory output.

## Optimized

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 61 / 2 | 66 / 8 | 4 / 0 |
| add | 13, 41 | 72 / 8 | 93 / 21 | 8 / 0 |
| subtract | 13, 41 | 72 / 5 | 93 / 15 | 8 / 0 |
| constant_chain | 13 | 67 / 6 | 76 / 13 | 6 / 0 |
| maximum | 13, 41 | 104 / 12 | 101 / 21 | 6 / 0 |
| wide_shift | $12345678 | 379 / 57 | 653 / 147 | 14 / 2 |
| loop_rotation | 13 | 180 / 37 | 1264 / 308 | 26 / 0 |
| sum_loop | 13 | 140 / 22 | 1395 / 344 | 16 / 0 |
| recursive_sum | 13 | 205 / 28 | 3091 / 699 | 190 / 67 |
| direct_calls | 13, 41 | 323 / 24 | 475 / 67 | 20 / 3 |
| byte_sum | $12FFFC, 16 | 238 / 35 | 4231 / 630 | 22 / 0 |
| record_field | $12FFFE | 82 / 10 | 114 / 24 | 8 / 0 |
| unlink | $130040 | 129 / INVALID (77) | 189 / INVALID (163) | 0 / INVALID (0) |
| forward_copy | $130000, $12FFF8, 8 | 304 / 42 | 3100 / 406 | 18 / 0 |

## Raw

| Kernel | Runtime arguments | Code bytes | Cycles | Stack bytes |
| --- | --- | ---: | ---: | ---: |
| identity | 13 | 61 / 5 | 66 / 20 | 4 / 2 |
| add | 13, 41 | 72 / 8 | 93 / 27 | 8 / 2 |
| subtract | 13, 41 | 72 / 8 | 93 / 27 | 8 / 2 |
| constant_chain | 13 | 157 / 9 | 226 / 25 | 6 / 2 |
| maximum | 13, 41 | 104 / 17 | 101 / 33 | 6 / 2 |
| wide_shift | $12345678 | 403 / 73 | 709 / 189 | 18 / 4 |
| loop_rotation | 13 | 192 / 65 | 1344 / 531 | 18 / 10 |
| sum_loop | 13 | 164 / 32 | 1767 / 533 | 14 / 4 |
| recursive_sum | 13 | 222 / 29 | 3571 / 759 | 190 / 67 |
| direct_calls | 13, 41 | 323 / 35 | 475 / 113 | 20 / 7 |
| byte_sum | $12FFFC, 16 | 262 / 68 | 4678 / 1528 | 16 / 6 |
| record_field | $12FFFE | 82 / 25 | 114 / 68 | 8 / 4 |
| unlink | $130040 | 191 / 121 | 324 / 251 | 6 / 10 |
| forward_copy | $130000, $12FFF8, 8 | 317 / 79 | 3325 / 1090 | 18 / 4 |
