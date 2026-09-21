# Native compare-to-branch fusion: before / after

All cells are **before / after actionc**, including guards and RTL.
Stack depth, ABI arguments, complete routine
storage maps, and stack-check costs are unchanged for every vector.

DP traffic counts byte reads plus writes; cycles are independent VM
cycles. Both host build modes produce identical measurements.

Each reached, decoded fusion removes exactly one stack byte read and write.
The predeclared counts match both host modes and both incoming I states.
All other stack traffic and all DP traffic are unchanged.

| Case | Mode | Vector | Fusions | Stack reads before / after | Stack writes before / after |
| --- | --- | ---: | ---: | ---: | ---: |
| byte_sum | optimized | 0 | 1 | 17 / 16 | 12 / 11 |
| byte_sum | optimized | 1 | 2 | 56 / 54 | 45 / 43 |
| byte_sum | optimized | 2 | 5 | 173 / 168 | 144 / 139 |
| byte_sum | optimized | 3 | 9 | 329 / 320 | 276 / 267 |
| byte_sum | optimized | 4 | 17 | 641 / 624 | 540 / 523 |
| byte_sum | raw | 0 | 1 | 21 / 20 | 16 / 15 |
| byte_sum | raw | 1 | 2 | 62 / 60 | 51 / 49 |
| byte_sum | raw | 2 | 5 | 185 / 180 | 156 / 151 |
| byte_sum | raw | 3 | 9 | 349 / 340 | 296 / 287 |
| byte_sum | raw | 4 | 17 | 677 / 660 | 576 / 559 |
| forward_copy | optimized | 0 | 9 | 416 / 407 | 331 / 322 |
| forward_copy | optimized | 1 | 9 | 416 / 407 | 331 / 322 |
| forward_copy | optimized | 2 | 9 | 416 / 407 | 331 / 322 |
| forward_copy | optimized | 3 | 1 | 16 / 15 | 11 / 10 |
| forward_copy | raw | 0 | 9 | 434 / 425 | 349 / 340 |
| forward_copy | raw | 1 | 9 | 434 / 425 | 349 / 340 |
| forward_copy | raw | 2 | 9 | 434 / 425 | 349 / 340 |
| forward_copy | raw | 3 | 1 | 18 / 17 | 13 / 12 |
| loop_rotation | optimized | 0 | 9 | 210 / 201 | 187 / 178 |
| loop_rotation | optimized | 1 | 9 | 210 / 201 | 187 / 178 |
| loop_rotation | optimized | 2 | 9 | 210 / 201 | 187 / 178 |
| loop_rotation | optimized | 3 | 9 | 210 / 201 | 187 / 178 |
| loop_rotation | optimized | 4 | 9 | 210 / 201 | 187 / 178 |
| loop_rotation | optimized | 5 | 9 | 210 / 201 | 187 / 178 |
| loop_rotation | raw | 0 | 9 | 248 / 239 | 225 / 216 |
| loop_rotation | raw | 1 | 9 | 248 / 239 | 225 / 216 |
| loop_rotation | raw | 2 | 9 | 248 / 239 | 225 / 216 |
| loop_rotation | raw | 3 | 9 | 248 / 239 | 225 / 216 |
| loop_rotation | raw | 4 | 9 | 248 / 239 | 225 / 216 |
| loop_rotation | raw | 5 | 9 | 248 / 239 | 225 / 216 |
| maximum | optimized | 0 | 1 | 16 / 15 | 7 / 6 |
| maximum | optimized | 1 | 1 | 16 / 15 | 7 / 6 |
| maximum | optimized | 2 | 1 | 16 / 15 | 7 / 6 |
| maximum | optimized | 3 | 1 | 16 / 15 | 7 / 6 |
| maximum | optimized | 4 | 1 | 16 / 15 | 7 / 6 |
| maximum | raw | 0 | 1 | 16 / 15 | 7 / 6 |
| maximum | raw | 1 | 1 | 16 / 15 | 7 / 6 |
| maximum | raw | 2 | 1 | 16 / 15 | 7 / 6 |
| maximum | raw | 3 | 1 | 16 / 15 | 7 / 6 |
| maximum | raw | 4 | 1 | 16 / 15 | 7 / 6 |
| recursive_sum | optimized | 0 | 1 | 8 / 7 | 3 / 2 |
| recursive_sum | optimized | 1 | 2 | 28 / 26 | 22 / 20 |
| recursive_sum | optimized | 2 | 9 | 168 / 159 | 155 / 146 |
| recursive_sum | optimized | 3 | 14 | 268 / 254 | 250 / 236 |
| recursive_sum | raw | 0 | 1 | 10 / 9 | 5 / 4 |
| recursive_sum | raw | 1 | 2 | 34 / 32 | 28 / 26 |
| recursive_sum | raw | 2 | 9 | 202 / 193 | 189 / 180 |
| recursive_sum | raw | 3 | 14 | 322 / 308 | 304 / 290 |
| sum_loop | optimized | 0 | 1 | 14 / 13 | 9 / 8 |
| sum_loop | optimized | 1 | 2 | 35 / 33 | 26 / 24 |
| sum_loop | optimized | 2 | 9 | 182 / 173 | 145 / 136 |
| sum_loop | optimized | 3 | 14 | 287 / 273 | 230 / 216 |
| sum_loop | optimized | 4 | 32 | 665 / 633 | 536 / 504 |
| sum_loop | raw | 0 | 1 | 18 / 17 | 13 / 12 |
| sum_loop | raw | 1 | 2 | 41 / 39 | 32 / 30 |
| sum_loop | raw | 2 | 9 | 202 / 193 | 165 / 156 |
| sum_loop | raw | 3 | 14 | 317 / 303 | 260 / 246 |
| sum_loop | raw | 4 | 32 | 731 / 699 | 602 / 570 |

## Optimized

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 71 / 71 | 86 / 86 | 6 / 6 | 0 / 0 |
| maximum | 146 / 116 | 149 / 117 | 6 / 6 | 0 / 0 |
| wide_shift | 379 / 379 | 653 / 653 | 14 / 14 | 100 / 100 |
| loop_rotation | 277 / 247 | 2000 / 1720 | 26 / 26 | 0 / 0 |
| sum_loop | 213 / 183 | 2465 / 2030 | 16 / 16 | 0 / 0 |
| recursive_sum | 247 / 217 | 3819 / 3372 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 311 / 281 | 5532 / 5004 | 22 / 22 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 129 / 129 | 189 / 189 | 0 / 0 | 40 / 40 |
| forward_copy | 354 / 324 | 3589 / 3309 | 18 / 18 | 208 / 208 |

## Raw

| Kernel | Code bytes | Cycles | Stack bytes | DP traffic |
| --- | ---: | ---: | ---: | ---: |
| identity | 63 / 63 | 71 / 71 | 4 / 4 | 0 / 0 |
| add | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| subtract | 74 / 74 | 98 / 98 | 8 / 8 | 0 / 0 |
| constant_chain | 191 / 191 | 311 / 311 | 6 / 6 | 0 / 0 |
| maximum | 146 / 116 | 149 / 117 | 6 / 6 | 0 / 0 |
| wide_shift | 403 / 403 | 709 / 709 | 18 / 18 | 100 / 100 |
| loop_rotation | 256 / 226 | 1968 / 1688 | 18 / 18 | 0 / 0 |
| sum_loop | 218 / 188 | 2596 / 2161 | 14 / 14 | 0 / 0 |
| recursive_sum | 262 / 232 | 4229 / 3782 | 190 / 190 | 0 / 0 |
| direct_calls | 329 / 329 | 500 / 500 | 20 / 20 | 0 / 0 |
| byte_sum | 316 / 286 | 5687 / 5159 | 16 / 16 | 208 / 208 |
| record_field | 82 / 82 | 114 / 114 | 8 / 8 | 7 / 7 |
| unlink | 191 / 191 | 324 / 324 | 6 / 6 | 68 / 68 |
| forward_copy | 365 / 335 | 3769 / 3489 | 18 / 18 | 208 / 208 |
