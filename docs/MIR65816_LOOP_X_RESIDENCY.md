# Bounded 65816 loop residency in X

The [plan](MIR65816_LOOP_X_RESIDENCY_PLAN.md) is implemented by frozen evidence
`0c3a4e5`, tracker prerequisite `084cf64`, selection `3f3d6e8` and qualification
probes/checker `7a73fca`. Selection uses typed MIR and verified allocation; it
has no kernel-name or fixed-offset special cases.

One private unsigned word header parameter may retain an X16 mirror in a
call-free scalar-DP routine with one simple loop. Admission requires its only
uses to be an immediate unsigned header test and one `p + 1` update, and its
assignment to finish both incoming word-copy schedules. The checked header
relation permits CPX; the update input can use TXA. A final TAX refresh follows
each incoming edge. The ordinary A-forwarding rule retains priority.

The DP home remains authoritative. All stores, distinct input/result homes,
closed-operation interference, frame sizes, maps, stack guards, ABI v1, image
v3 and o65 profile v1 remain. Overwriting the home invalidates the old mirror
until TAX, so the intermediate state may legitimately contain different X and
DP values. Checked CFG obligations establish a fresh relation at each join;
ordinary value/flag facts still stop there. Every instruction in the reserved
region is checked for clobbers. Calls, helpers, aliasing/unknown writes,
unsupported operations and extra loop entries reject admission. Reservation
ends on exit. See the [emission contract](MIR65816_EMISSION_CONTRACT.md).

## Measured result

The [new comparison baseline](benchmarks/65816-loop-x/after/tables.md) executes
raw and optimized output, six rotation inputs, both incoming I states and both
host VM profiles. Values below cover the optimized worker entry through RTL,
including stack checks:

| Metric | Scalar DP baseline | X mirror |
| --- | ---: | ---: |
| Worker bytes | 130 | 129 |
| Cycles, each rotation vector | 793 | 759 |
| Instructions | 218 | 218 |
| Fixed frame / observed stack bytes | 8 / 8 | 8 / 8 |
| DP bytes read / written | 102 / 104 | 68 / 104 |
| Stack bytes read / written | 21 / 32 | 21 / 32 |
| TXA input replacements | 0 | 8 |

The saving is 34 cycles (4.3%) and one byte. Nine CPX tests and eight TXA
inputs pay for nine TAX refreshes. Copy counts, existing forwarding counts,
metadata traffic and guard costs remain unchanged.

The [exact checker result](benchmarks/65816-loop-x/delta.json) verifies all
28 complete images, manifests, instruction sites, control-flow metadata and
264 records against the pre-implementation frozen transform. The other 27
Action builds and all vbcc results remain identical. All 132 Action records
are correct; the historical optimized vbcc unlink vector-0 failure remains
reported. Sum-loop still measures 120 bytes / 1,092 cycles at input 13.

## Qualification

[Saved qualification](abi/action65816-loop-x-qualification.json) authenticates
440 compiler/test inputs, 649 matching native artifacts and comparison results.
All 104 native library tests, 61 affected integration tests and 116 native VM
tests per host profile pass. The isolated CRLF checkout passes one emission
check and 69 native tests, with all 617 artifacts matching LF. Coverage includes:

- CPX index-width effects with byte/word A, independently assembled TAX/TXA/CPX
  encodings, exact cycles and CPU flags; short and long dispatch at two origins.
- Exhaustive unsigned branch truth for all 65,536 counters at representative
  `<`/`<=` bounds, with `$FFFF` rejected for `<=`.
- 288 executions varying initial words, bounds, zero/one/two trips and wrapping
  ADD, across raw/optimized frontend probes, flat images and two o65 placements.
  Raw positive probes retain independently verified optimized worker MIR; the
  actual raw corpus remains unchanged.
- Direct, self-copy-repair and selectively staged incoming tails; authoritative
  stores, frame guards, disjoint exit-home reuse and trace-on/off equality.
- Mutation rejection for instruction bytes, thresholds, homes, refresh tails,
  stale X values and branch polarity, plus eight corrupted-counter controls.
- IRQ/NMI at all 110 reached task/instruction sites per frontend mode in the
  scalar leaf, with two task
  domains and scalar IRQ dispatch. Independent uninterrupted steps verify full
  CPU, live frame and suspended DP restoration, including STA-before-TAX and
  CPX-before-branch boundaries. Both seeded schedules complete correctly.
- The affected native library/integration suites, full native debug/release
  qualification, comparison/disassembler tools and an isolated CRLF rebuild.
  This target-only slice changes no NIR or semantic contract.

Reproduce with a release `actionc-65816` build and the retained scalar baseline:

```sh
python3 -B tools/compare65816/build.py --output target/loop-x-after --verify-crlf
A816_COMPARISON_MANIFEST="$PWD/target/loop-x-after/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/loop-x-after/debug.json" \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -- --ignored
```

Repeat with `--release` and `release.json`. These corpus test commands return
failure for the retained vbcc case; the exact checker requires that single
known failure and rejects any additional one:

```sh
python3 -B tools/compare65816/check_loop_x.py \
  target/scalar-dp-after target/loop-x-after \
  --inventory docs/benchmarks/65816-loop-x/frozen.json \
  --output target/loop-x-delta.json
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```

Mutable-counter promotion, removing backing stores/homes, broader loops,
Y allocation and cross-call residency remain separate work. The new baseline
should guide the next inventory before extending this bounded strategy.
