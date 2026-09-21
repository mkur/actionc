# Native word returns: before / after

All cells are **before / after actionc**, including guards and RTL.
Stack depth, byte traffic on the stack, ABI arguments, complete routine
storage maps, and stack-check costs are unchanged for every vector.

DP traffic counts byte reads plus writes; cycles are independent VM
cycles. Both host build modes produce identical measurements.

## Optimized

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 87 / 63 | 108 / 71 | 4 / 4 | 10 / 0 |
| add | 98 / 74 | 135 / 98 | 8 / 8 | 10 / 0 |
| subtract | 98 / 74 | 135 / 98 | 8 / 8 | 10 / 0 |
| constant_chain | 95 / 71 | 123 / 86 | 6 / 6 | 10 / 0 |
| maximum | 234 / 186 | 210 / 173 | 6 / 6 | 14 / 4 |
| wide_shift | 379 / 379 | 653 / 653 | 14 / 14 | 100 / 100 |
| loop_rotation | 338 / 314 | 2223 / 2186 | 26 / 26 | 46 / 36 |
| sum_loop | 276 / 252 | 2866 / 2829 | 16 / 16 | 66 / 56 |
| recursive_sum | 333 / 286 | 4687 / 4171 | 190 / 190 | 196 / 56 |
| direct_calls | 377 / 329 | 611 / 500 | 20 / 20 | 30 / 0 |
| byte_sum | 374 / 350 | 6011 / 5974 | 22 / 22 | 286 / 276 |
| record_field | 106 / 82 | 151 / 114 | 8 / 8 | 17 / 7 |
| unlink | 129 / 129 | 189 / 189 | 0 / 0 | 40 / 40 |
| forward_copy | 393 / 393 | 3823 / 3823 | 18 / 18 | 244 / 244 |

## Raw

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 87 / 63 | 108 / 71 | 4 / 4 | 10 / 0 |
| add | 98 / 74 | 135 / 98 | 8 / 8 | 10 / 0 |
| subtract | 98 / 74 | 135 / 98 | 8 / 8 | 10 / 0 |
| constant_chain | 215 / 191 | 348 / 311 | 6 / 6 | 10 / 0 |
| maximum | 234 / 186 | 210 / 173 | 6 / 6 | 14 / 4 |
| wide_shift | 403 / 403 | 709 / 709 | 18 / 18 | 100 / 100 |
| loop_rotation | 316 / 292 | 2182 / 2145 | 18 / 18 | 46 / 36 |
| sum_loop | 278 / 254 | 2941 / 2904 | 14 / 14 | 66 / 56 |
| recursive_sum | 345 / 298 | 5041 / 4525 | 190 / 190 | 196 / 56 |
| direct_calls | 377 / 329 | 611 / 500 | 20 / 20 | 30 / 0 |
| byte_sum | 376 / 352 | 6098 / 6061 | 16 / 16 | 286 / 276 |
| record_field | 106 / 82 | 151 / 114 | 8 / 8 | 17 / 7 |
| unlink | 191 / 191 | 324 / 324 | 6 / 6 | 68 / 68 |
| forward_copy | 401 / 401 | 3967 / 3967 | 18 / 18 | 244 / 244 |
