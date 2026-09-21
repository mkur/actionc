# Native local accumulator forwarding: before / after

All cells are **before / after actionc**, including guards and RTL.
Stack depth, ABI arguments, complete routine
storage maps, and stack-check costs are unchanged for every vector.

DP traffic counts byte reads plus writes; cycles are independent VM
cycles. Both host build modes produce identical measurements.

Each verified forwarded word removes one instruction, five cycles and
two private stack-byte reads. Every store, DP access and frame remains.

1422 reload executions match independent predictions per incoming I state.

## Optimized

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 61 | 71 / 66 | 4 / 4 | 0 / 0 |
| add | 74 / 72 | 98 / 93 | 8 / 8 | 0 / 0 |
| subtract | 74 / 72 | 98 / 93 | 8 / 8 | 0 / 0 |
| constant_chain | 71 / 67 | 86 / 76 | 6 / 6 | 0 / 0 |
| maximum | 110 / 104 | 111 / 101 | 6 / 6 | 0 / 0 |
| wide_shift | 379 / 379 | 653 / 653 | 14 / 14 | 100 / 100 |
| loop_rotation | 186 / 180 | 1314 / 1264 | 26 / 26 | 0 / 0 |
| sum_loop | 146 / 140 | 1595 / 1395 | 16 / 16 | 0 / 0 |
| recursive_sum | 211 / 205 | 3291 / 3091 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 323 | 500 / 475 | 20 / 20 | 0 / 0 |
| byte_sum | 244 / 238 | 4476 / 4231 | 22 / 22 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 129 / 129 | 189 / 189 | 0 / 0 | 40 / 40 |
| forward_copy | 310 / 304 | 3225 / 3100 | 18 / 18 | 208 / 208 |

## Raw

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 61 | 71 / 66 | 4 / 4 | 0 / 0 |
| add | 74 / 72 | 98 / 93 | 8 / 8 | 0 / 0 |
| subtract | 74 / 72 | 98 / 93 | 8 / 8 | 0 / 0 |
| constant_chain | 191 / 157 | 311 / 226 | 6 / 6 | 0 / 0 |
| maximum | 110 / 104 | 111 / 101 | 6 / 6 | 0 / 0 |
| wide_shift | 403 / 403 | 709 / 709 | 18 / 18 | 100 / 100 |
| loop_rotation | 212 / 192 | 1604 / 1344 | 18 / 18 | 0 / 0 |
| sum_loop | 174 / 164 | 2032 / 1767 | 14 / 14 | 0 / 0 |
| recursive_sum | 226 / 222 | 3701 / 3571 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 323 | 500 / 475 | 20 / 20 | 0 / 0 |
| byte_sum | 272 / 262 | 5003 / 4678 | 16 / 16 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 191 / 191 | 324 / 324 | 6 / 6 | 68 / 68 |
| forward_copy | 321 / 317 | 3405 / 3325 | 18 / 18 | 208 / 208 |
