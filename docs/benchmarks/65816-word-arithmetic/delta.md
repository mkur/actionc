# Native word arithmetic: before / after

All cells are **before / after actionc**, including guards and RTL.
Stack depth, byte traffic on the stack, ABI arguments, complete routine
storage maps, and stack-check costs are unchanged for every vector.

DP traffic counts byte reads plus writes; cycles are independent VM
cycles. Both host build modes produce identical measurements.

## Optimized

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 87 / 87 | 108 / 108 | 4 / 4 | 10 / 10 |
| add | 116 / 98 | 162 / 135 | 8 / 8 | 14 / 10 |
| subtract | 116 / 98 | 162 / 135 | 8 / 8 | 14 / 10 |
| constant_chain | 112 / 95 | 148 / 123 | 6 / 6 | 14 / 10 |
| maximum | 234 / 234 | 210 / 210 | 6 / 6 | 14 / 14 |
| wide_shift | 379 / 379 | 653 / 653 | 14 / 14 | 100 / 100 |
| loop_rotation | 401 / 338 | 2624 / 2223 | 26 / 26 | 118 / 46 |
| sum_loop | 311 / 276 | 3542 / 2866 | 16 / 16 | 170 / 66 |
| recursive_sum | 368 / 333 | 5363 / 4687 | 190 / 190 | 300 / 196 |
| direct_calls | 412 / 377 | 688 / 611 | 20 / 20 | 42 / 30 |
| byte_sum | 405 / 374 | 6747 / 6011 | 22 / 22 | 414 / 286 |
| record_field | 106 / 106 | 151 / 151 | 8 / 8 | 17 / 17 |
| unlink | 129 / 129 | 189 / 189 | 0 / 0 | 40 / 40 |
| forward_copy | 410 / 393 | 4023 / 3823 | 18 / 18 | 276 / 244 |

## Raw

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 87 / 87 | 108 / 108 | 4 / 4 | 10 / 10 |
| add | 116 / 98 | 162 / 135 | 8 / 8 | 14 / 10 |
| subtract | 116 / 98 | 162 / 135 | 8 / 8 | 14 / 10 |
| constant_chain | 427 / 215 | 658 / 348 | 6 / 6 | 74 / 10 |
| maximum | 234 / 234 | 210 / 210 | 6 / 6 | 14 / 14 |
| wide_shift | 403 / 403 | 709 / 709 | 18 / 18 | 100 / 100 |
| loop_rotation | 385 / 316 | 2634 / 2182 | 18 / 18 | 118 / 46 |
| sum_loop | 313 / 278 | 3617 / 2941 | 14 / 14 | 170 / 66 |
| recursive_sum | 380 / 345 | 5717 / 5041 | 190 / 190 | 300 / 196 |
| direct_calls | 412 / 377 | 688 / 611 | 20 / 20 | 42 / 30 |
| byte_sum | 411 / 376 | 6930 / 6098 | 16 / 16 | 414 / 286 |
| record_field | 106 / 106 | 151 / 151 | 8 / 8 | 17 / 17 |
| unlink | 191 / 191 | 324 / 324 | 6 / 6 | 68 / 68 |
| forward_copy | 418 / 401 | 4167 / 3967 | 18 / 18 | 276 / 244 |
