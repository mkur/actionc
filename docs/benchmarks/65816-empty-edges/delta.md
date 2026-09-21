# Native empty edges: before / after

All cells are **before / after actionc**, including guards and RTL.
Stack depth, ABI arguments, complete routine
storage maps, and stack-check costs are unchanged for every vector.

DP traffic counts byte reads plus writes; cycles are independent VM
cycles. Both host build modes produce identical measurements.

Stack reads and writes are unchanged for every vector.

## Optimized

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 71 / 71 | 86 / 86 | 6 / 6 | 0 / 0 |
| maximum | 116 / 110 | 117 / 111 | 6 / 6 | 0 / 0 |
| wide_shift | 379 / 379 | 653 / 653 | 14 / 14 | 100 / 100 |
| loop_rotation | 192 / 186 | 1344 / 1314 | 26 / 26 | 0 / 0 |
| sum_loop | 160 / 154 | 1780 / 1735 | 16 / 16 | 0 / 0 |
| recursive_sum | 217 / 211 | 3372 / 3291 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 258 / 252 | 4700 / 4646 | 22 / 22 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 129 / 129 | 189 / 189 | 0 / 0 | 40 / 40 |
| forward_copy | 324 / 310 | 3309 / 3225 | 18 / 18 | 208 / 208 |

## Raw

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 191 / 191 | 311 / 311 | 6 / 6 | 0 / 0 |
| maximum | 116 / 110 | 117 / 111 | 6 / 6 | 0 / 0 |
| wide_shift | 403 / 403 | 709 / 709 | 18 / 18 | 100 / 100 |
| loop_rotation | 226 / 212 | 1688 / 1604 | 18 / 18 | 0 / 0 |
| sum_loop | 188 / 174 | 2161 / 2032 | 14 / 14 | 0 / 0 |
| recursive_sum | 232 / 226 | 3782 / 3701 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 286 / 272 | 5159 / 5003 | 16 / 16 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 191 / 191 | 324 / 324 | 6 / 6 | 68 / 68 |
| forward_copy | 335 / 321 | 3489 / 3405 | 18 / 18 | 208 / 208 |
