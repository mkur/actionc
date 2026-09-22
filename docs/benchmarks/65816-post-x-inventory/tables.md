# Current memory and register inventory

VM cycles and byte traffic are per invocation and per incoming I state; both
I states agree, as do both host builds. Loads are executed word LDA counts.
DP excludes task metadata. Stack includes argument and return-address traffic.
X/Y columns count explicit body accesses, excluding entry and return sequences;
call clobbers and conservative operation barriers are recorded separately in JSON.

| Kernel | Mode | Arguments | Bytes | Cycles | Peak | Stack R/W | DP R/W | Word LDA stack/DP | Body X R/W | Body Y R/W |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| identity | raw | 13 | 51 | 49 | 0 | 5/0 | 0/2 | 1/0 | 0/0 | 0/0 |
| identity | optimized | 13 | 51 | 49 | 0 | 5/0 | 0/2 | 1/0 | 0/0 | 0/0 |
| add | raw | 13, 41 | 62 | 72 | 0 | 7/0 | 4/6 | 2/1 | 0/0 | 0/0 |
| add | optimized | 13, 41 | 62 | 72 | 0 | 7/0 | 4/6 | 2/1 | 0/0 | 0/0 |
| subtract | raw | 13, 41 | 62 | 72 | 0 | 7/0 | 4/6 | 2/1 | 0/0 | 0/0 |
| subtract | optimized | 13, 41 | 62 | 72 | 0 | 7/0 | 4/6 | 2/1 | 0/0 | 0/0 |
| constant_chain | raw | 13 | 147 | 193 | 0 | 5/0 | 0/34 | 1/0 | 0/0 | 0/0 |
| constant_chain | optimized | 13 | 57 | 58 | 0 | 5/0 | 0/4 | 1/0 | 0/0 | 0/0 |
| maximum | raw | 13, 41 | 90 | 90 | 2 | 9/0 | 2/6 | 3/0 | 0/0 | 0/0 |
| maximum | optimized | 13, 41 | 90 | 90 | 2 | 9/0 | 2/6 | 3/0 | 0/0 | 0/0 |
| wide_shift | raw | 305419896 | 401 | 706 | 18 | 39/28 | 48/52 | 4/1 | 8/10 | 0/0 |
| wide_shift | optimized | 305419896 | 377 | 650 | 14 | 31/20 | 48/52 | 4/1 | 8/10 | 0/0 |
| loop_rotation | raw | 13 | 170 | 1221 | 18 | 133/216 | 0/0 | 55/0 | 0/0 | 0/0 |
| loop_rotation | optimized | 13 | 129 | 759 | 8 | 21/32 | 68/104 | 9/33 | 17/9 | 0/0 |
| sum_loop | raw | 13 | 146 | 1587 | 14 | 197/246 | 0/0 | 69/0 | 0/0 | 0/0 |
| sum_loop | optimized | 13 | 120 | 1092 | 6 | 85/28 | 80/160 | 40/27 | 0/0 | 0/0 |
| recursive_sum | raw | 13 | 206 | 3402 | 190 | 230/290 | 0/0 | 54/0 | 0/13 | 13/13 |
| recursive_sum | optimized | 13 | 191 | 2987 | 190 | 174/236 | 0/0 | 40/0 | 0/13 | 13/13 |
| direct_calls | raw | 13, 41 | 311 | 436 | 14 | 25/26 | 0/8 | 5/0 | 0/2 | 2/2 |
| direct_calls | optimized | 13, 41 | 311 | 436 | 14 | 25/26 | 0/8 | 5/0 | 0/2 | 2/2 |
| byte_sum | raw | 1245180, 16 | 244 | 4459 | 16 | 530/559 | 96/112 | 196/0 | 0/0 | 16/16 |
| byte_sum | optimized | 1245180, 16 | 218 | 4009 | 18 | 492/489 | 96/112 | 194/0 | 0/0 | 16/16 |
| record_field | raw | 1245182 | 80 | 111 | 8 | 13/6 | 3/4 | 5/0 | 0/0 | 1/1 |
| record_field | optimized | 1245182 | 80 | 111 | 8 | 13/6 | 3/4 | 5/0 | 0/0 | 1/1 |
| unlink | raw | 1245248 | 189 | 321 | 6 | 27/8 | 38/30 | 12/6 | 0/0 | 8/8 |
| unlink | optimized | 1245248 | 127 | 186 | 0 | 7/0 | 30/10 | 2/2 | 0/0 | 8/8 |
| forward_copy | raw | 1245184, 1245176, 8 | 297 | 3207 | 18 | 393/340 | 96/112 | 154/0 | 0/0 | 16/16 |
| forward_copy | optimized | 1245184, 1245176, 8 | 284 | 2982 | 18 | 357/322 | 96/112 | 145/0 | 0/0 | 16/16 |

Known optimized vbcc unlink vector 0 remains incorrect in the source reports.
The complete Action instruction streams and all 132 Action records are inventoried;
uncounted Main wrappers have static facts only. This is not a register-allocation forecast.
