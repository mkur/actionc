# Toolkit Size Analysis

Generated from:

```sh
surveys/toolkit/compile-toolkit-batch.sh --preset all
```

Sizes are object-file byte counts. `Original` is the object captured from the
original Action! compiler VM under `outputs/vm`. The actionc columns come from
`outputs/batch/{legacy-classic,modern-classic,modern-mir6502}`.

The original legacy sources for KALSCOPE, PMGDM1, PMGDM2, and PRINTF1 depend on
loose pointer conversions that actionc intentionally rejects. The legacy preset
verifies those rejections, then compiles their maintained overlays with the
legacy profile and classic backend. The size matrix reports those emitted
overlay objects.

## Summary

| Column | Successes | Comparable original bytes | Output bytes | Delta vs original |
| --- | ---: | ---: | ---: | ---: |
| Original compiler | 20 / 20 | 46,292 | 46,292 | +0 (+0.0%) |
| Legacy classic | 20 / 20 | 46,292 | 49,546 | +3,254 (+7.0%) |
| Modern classic | 20 / 20 | 46,292 | 45,442 | -850 (-1.8%) |
| Modern MIR6502 | 20 / 20 | 46,292 | 57,596 | +11,304 (+24.4%) |

The comparable original byte total is restricted to entries that produced an
object in that actionc column. Modern MIR6502 produced 57,596 bytes versus
45,442 bytes for modern classic: +12,154 bytes (+26.7%).

## Per-Entry Size Matrix

| Stem | Source | Original | Legacy classic | Legacy vs orig | Modern classic | Modern classic vs orig | Modern MIR6502 | MIR6502 vs orig | MIR6502 vs modern classic |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `ABS` | `ABS.ACT` | 66 | 66 | +0 (+0.0%) | 33 | -33 (-50.0%) | 84 | +18 (+27.3%) | +51 (+154.5%) |
| `ALLOCATE` | `ALLOCATE.ACT` | 1,026 | 1,024 | -2 (-0.2%) | 947 | -79 (-7.7%) | 1,258 | +232 (+22.6%) | +311 (+32.8%) |
| `CHARTEST` | `CHARTEST.ACT` | 248 | 248 | +0 (+0.0%) | 216 | -32 (-12.9%) | 283 | +35 (+14.1%) | +67 (+31.0%) |
| `CONSOLE` | `CONSOLE.ACT` | 284 | 284 | +0 (+0.0%) | 248 | -36 (-12.7%) | 258 | -26 (-9.2%) | +10 (+4.0%) |
| `IO` | `IO.ACT` | 480 | 480 | +0 (+0.0%) | 427 | -53 (-11.0%) | 431 | -49 (-10.2%) | +4 (+0.9%) |
| `JOYSTIX` | `JOYSTIX.ACT` | 155 | 155 | +0 (+0.0%) | 152 | -3 (-1.9%) | 163 | +8 (+5.2%) | +11 (+7.2%) |
| `CIRCLE1` | `CIRCLE.DM1` | 704 | 698 | -6 (-0.9%) | 631 | -73 (-10.4%) | 845 | +141 (+20.0%) | +214 (+33.9%) |
| `CIRCLE2` | `CIRCLE.DM2` | 991 | 983 | -8 (-0.8%) | 895 | -96 (-9.7%) | 1,122 | +131 (+13.2%) | +227 (+25.4%) |
| `GEMDEM` | `GEM.DEM` | 10,311 | 11,429 | +1,118 (+10.8%) | 10,375 | +64 (+0.6%) | 14,008 | +3,697 (+35.9%) | +3,633 (+35.0%) |
| `KALSCOPE` | `KALSCOPE.DEM` | 2,938 | 3,560 | +622 (+21.2%) | 3,382 | +444 (+15.1%) | 3,964 | +1,026 (+34.9%) | +582 (+17.2%) |
| `MUSICDEM` | `MUSIC.DEM` | 3,696 | 3,785 | +89 (+2.4%) | 3,306 | -390 (-10.6%) | 3,901 | +205 (+5.5%) | +595 (+18.0%) |
| `PMGDM1` | `PMG.DM1` | 2,167 | 2,307 | +140 (+6.5%) | 1,926 | -241 (-11.1%) | 2,461 | +294 (+13.6%) | +535 (+27.8%) |
| `PMGDM2` | `PMG.DM2` | 2,232 | 2,339 | +107 (+4.8%) | 1,934 | -298 (-13.4%) | 2,553 | +321 (+14.4%) | +619 (+32.0%) |
| `PRINTF1` | `PRINTF.DM1` | 2,361 | 2,496 | +135 (+5.7%) | 2,279 | -82 (-3.5%) | 2,854 | +493 (+20.9%) | +575 (+25.2%) |
| `REALDM1` | `REAL.DM1` | 2,228 | 2,228 | +0 (+0.0%) | 1,974 | -254 (-11.4%) | 2,079 | -149 (-6.7%) | +105 (+5.3%) |
| `SNAILS` | `SNAILS.DEM` | 1,500 | 1,501 | +1 (+0.1%) | 1,311 | -189 (-12.6%) | 1,614 | +114 (+7.6%) | +303 (+23.1%) |
| `SORTDM1` | `SORT.DM1` | 3,962 | 4,202 | +240 (+6.1%) | 4,197 | +235 (+5.9%) | 6,181 | +2,219 (+56.0%) | +1,984 (+47.3%) |
| `SORTDM2` | `SORT.DM2` | 2,620 | 2,599 | -21 (-0.8%) | 2,682 | +62 (+2.4%) | 3,957 | +1,337 (+51.0%) | +1,275 (+47.5%) |
| `TURTLE1` | `TURTLE.DM1` | 1,190 | 1,190 | +0 (+0.0%) | 1,132 | -58 (-4.9%) | 1,400 | +210 (+17.6%) | +268 (+23.7%) |
| `WARPDEM` | `WARP.DEM` | 7,133 | 7,972 | +839 (+11.8%) | 7,395 | +262 (+3.7%) | 8,180 | +1,047 (+14.7%) | +785 (+10.6%) |

## Largest Size Gaps

Relative to the original compiler, the largest MIR6502 increases are:

| Stem | Original | Modern MIR6502 | Delta |
| --- | ---: | ---: | ---: |
| `GEMDEM` | 10,311 | 14,008 | +3,697 (+35.9%) |
| `SORTDM1` | 3,962 | 6,181 | +2,219 (+56.0%) |
| `SORTDM2` | 2,620 | 3,957 | +1,337 (+51.0%) |
| `KALSCOPE` | 2,938 | 3,964 | +1,026 (+34.9%) |
| `WARPDEM` | 7,133 | 8,180 | +1,047 (+14.7%) |

The best MIR6502 result versus original is `REALDM1`: 2,079 bytes versus
2,228, or -149 bytes (-6.7%).
