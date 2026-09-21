# Native word comparisons: before / after

All cells are **before / after actionc**, including guards and RTL.
Stack depth, stack writes, ABI arguments, complete routine
storage maps, and stack-check costs are unchanged for every vector.

DP traffic counts byte reads plus writes; cycles are independent VM
cycles. Both host build modes produce identical measurements.

Stack reads change only by the following predeclared amounts; all
other records retain exactly their previous stack-read counts.

| Case | Mode | Vector | Stack reads before / after |
| --- | --- | ---: | ---: |
| maximum | optimized | 3 | 14 / 16 |
| maximum | optimized | 4 | 14 / 16 |
| maximum | raw | 3 | 14 / 16 |
| maximum | raw | 4 | 14 / 16 |

## Optimized

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 71 / 71 | 86 / 86 | 6 / 6 | 0 / 0 |
| maximum | 186 / 146 | 173 / 149 | 6 / 6 | 4 / 0 |
| wide_shift | 379 / 379 | 653 / 653 | 14 / 14 | 100 / 100 |
| loop_rotation | 314 / 277 | 2186 / 2000 | 26 / 26 | 36 / 0 |
| sum_loop | 252 / 213 | 2829 / 2465 | 16 / 16 | 56 / 0 |
| recursive_sum | 286 / 247 | 4171 / 3819 | 190 / 190 | 56 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 350 / 311 | 5974 / 5532 | 22 / 22 | 276 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 129 / 129 | 189 / 189 | 0 / 0 | 40 / 40 |
| forward_copy | 393 / 354 | 3823 / 3589 | 18 / 18 | 244 / 208 |

## Raw

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 191 / 191 | 311 / 311 | 6 / 6 | 0 / 0 |
| maximum | 186 / 146 | 173 / 149 | 6 / 6 | 4 / 0 |
| wide_shift | 403 / 403 | 709 / 709 | 18 / 18 | 100 / 100 |
| loop_rotation | 292 / 256 | 2145 / 1968 | 18 / 18 | 36 / 0 |
| sum_loop | 254 / 218 | 2904 / 2596 | 14 / 14 | 56 / 0 |
| recursive_sum | 298 / 262 | 4525 / 4229 | 190 / 190 | 56 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 352 / 316 | 6061 / 5687 | 16 / 16 | 276 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 191 / 191 | 324 / 324 | 6 / 6 | 68 / 68 |
| forward_copy | 401 / 365 | 3967 / 3769 | 18 / 18 | 244 / 208 |
