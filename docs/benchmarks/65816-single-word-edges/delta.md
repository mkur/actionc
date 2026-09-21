# Native direct single-word edges: before / after

All cells are **before / after actionc**, including guards and RTL.
Stack depth, ABI arguments, complete routine
storage maps, and stack-check costs are unchanged for every vector.

DP traffic counts byte reads plus writes; cycles are independent VM
cycles. Both host build modes produce identical measurements.

Each verified direct word copy removes two instructions, ten cycles,
two stack byte reads and two writes. Other traffic is unchanged.

| Case | Mode | Vector | Direct copies | Stack reads before / after | Stack writes before / after |
| --- | --- | ---: | ---: | ---: | ---: |
| byte_sum | optimized | 0 | 1 | 16 / 14 | 11 / 9 |
| byte_sum | optimized | 1 | 2 | 54 / 50 | 43 / 39 |
| byte_sum | optimized | 2 | 5 | 168 / 158 | 139 / 129 |
| byte_sum | optimized | 3 | 9 | 320 / 302 | 267 / 249 |
| byte_sum | optimized | 4 | 17 | 624 / 590 | 523 / 489 |
| sum_loop | optimized | 0 | 1 | 13 / 11 | 8 / 6 |
| sum_loop | optimized | 1 | 2 | 33 / 29 | 24 / 20 |
| sum_loop | optimized | 2 | 9 | 173 / 155 | 136 / 118 |
| sum_loop | optimized | 3 | 14 | 273 / 245 | 216 / 188 |
| sum_loop | optimized | 4 | 32 | 633 / 569 | 504 / 440 |

## Optimized

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 71 / 71 | 86 / 86 | 6 / 6 | 0 / 0 |
| maximum | 110 / 110 | 111 / 111 | 6 / 6 | 0 / 0 |
| wide_shift | 379 / 379 | 653 / 653 | 14 / 14 | 100 / 100 |
| loop_rotation | 186 / 186 | 1314 / 1314 | 26 / 26 | 0 / 0 |
| sum_loop | 154 / 146 | 1735 / 1595 | 16 / 16 | 0 / 0 |
| recursive_sum | 211 / 211 | 3291 / 3291 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 252 / 244 | 4646 / 4476 | 22 / 22 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 129 / 129 | 189 / 189 | 0 / 0 | 40 / 40 |
| forward_copy | 310 / 310 | 3225 / 3225 | 18 / 18 | 208 / 208 |

## Raw

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 191 / 191 | 311 / 311 | 6 / 6 | 0 / 0 |
| maximum | 110 / 110 | 111 / 111 | 6 / 6 | 0 / 0 |
| wide_shift | 403 / 403 | 709 / 709 | 18 / 18 | 100 / 100 |
| loop_rotation | 212 / 212 | 1604 / 1604 | 18 / 18 | 0 / 0 |
| sum_loop | 174 / 174 | 2032 / 2032 | 14 / 14 | 0 / 0 |
| recursive_sum | 226 / 226 | 3701 / 3701 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 272 / 272 | 5003 / 5003 | 16 / 16 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 191 / 191 | 324 / 324 | 6 / 6 | 68 / 68 |
| forward_copy | 321 / 321 | 3405 / 3405 | 18 / 18 | 208 / 208 |
