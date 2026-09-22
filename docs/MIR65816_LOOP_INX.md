# Bounded native 65816 loop increments

The [post-X inventory and forecast](MIR65816_POST_X_INVENTORY.md), committed as
`81b5ac4`, are implemented by `1ce9624`. The existing reserved unsigned word
loop parameter can now use `INX; TXA; STA q` for its typed `q = p + 1` update,
replacing `TXA; CLC; ADC #1; STA q`. Admission uses verified MIR identities and
allocation, with no program-name, source-string or fixed-offset special cases.

## Contract

The existing call-free scalar whitelist, simple-loop shape, sole-use parameter,
unsigned immediate test and final incoming-copy placement remain required.
The bounded extension also requires the body suffix after the update to contain
no materialized comparison: those selectors introduce internal labels. Such a
suffix keeps the prior X-mirror update sequence. Calls, helpers, aliasing writes,
unsupported operations and extra loop entries retain their existing fallback.

INX changes X from p to q while p's authoritative DP home still contains p.
The tracker invalidates that relation before emitting INX, retains the register
reservation, and grants only the immediate TXA permission to obtain q. CPX,
another increment, an old-parameter load or any label/edge rejects the pending
relation. The retained backedge stores do not themselves restore permission;
only the existing final TAX refresh establishes the new X/parameter relation.

Both interfering DP homes, every store, backedge reload, copy schedule and TAX
refresh remain. Frame maps, stack accounting and guards, interrupt reserves,
public ABI v1, image v3 and o65 profile v1 are unchanged. The instruction model
uses index width for INX, wraps accordingly, writes X and N/Z, and preserves
A/Y/C/V/S/D. TXA establishes the result's word N/Z in A16.

C/V are not outputs of the admitted MIR ADD. The closed routine whitelist has
no flag-valued operands or machine blocks; arithmetic and comparison consumers
establish their own flag inputs, and public flags are call-clobbered. The new
sequence preserves the CPU's actual C/V rather than producing the old ADC's
flags. Interruption checks therefore compare with the new instruction stream.
See the [emission contract](MIR65816_EMISSION_CONTRACT.md).

## Measured result

The [comparison baseline](benchmarks/65816-loop-inx/after/tables.md) executes
raw and optimized output with both incoming I states and debug/release hosts.
Optimized rotation has the following result for each of its six vectors,
measured from worker entry through RTL, including stack checks:

| Metric | X mirror | Native INX |
| --- | ---: | ---: |
| Worker bytes | 129 | 126 |
| Cycles | 759 | 735 |
| Instructions | 218 | 210 |
| Fixed frame / observed stack bytes | 8 / 8 | 8 / 8 |
| DP bytes read / written | 68 / 104 | 68 / 104 |
| Stack bytes read / written | 21 / 32 | 21 / 32 |
| X-forwarded input loads | 8 | 0 |
| Dedicated X increment updates | 0 | 8 |

This saves three bytes and 24 cycles (3.2%). The separate increment counter
preserves the old load counter's meaning: TXA now transfers q, not p. All other
forwarding/copy counts and memory traffic remain equal.

The [exact checker](benchmarks/65816-loop-inx/delta.json) matches every complete
image, address map, instruction site, control-flow record and measurement to
the frozen transform. Only optimized rotation changes among 28 Action builds.
All 132 Action records are correct, all vbcc output remains unchanged, and the
known optimized vbcc unlink vector-0 failure remains reported. Sum-loop input
13 stays at 120 bytes / 1,092 cycles; mutable counter promotion is separate work.

## Qualification

[Saved qualification](abi/action65816-loop-inx-qualification.json) authenticates
440 compiler/test inputs and 649 matching native debug/release artifacts.
All 107 emitter/proof library tests, 61 affected root integration tests and
117 native VM tests per host profile pass. The isolated CRLF checkout passes
one emission check and 70 native tests, with all 617 artifacts matching LF.
All 28 corpus builds also match their CRLF rebuilds. Coverage includes:

- Independent ca65 INX encoding and VM execution over seven boundary values,
  A8/A16, X8/X16, both I states and all C/V states; exact cycles, wrapping,
  preserved registers and N/Z effects.
- 288 loop probe executions across raw/optimized frontends, zero/one/two trips,
  wrapping updates, direct/self/selective incoming tails, flat images and two
  o65 placements. Raw positive probes retain independently verified optimized
  worker MIR; the actual raw corpus remains unchanged.
- Rejection of stale relations, unplanned increments, wrong homes/identities,
  altered bytes, thresholds, refresh tails and branch polarity; a valid body
  with a later materialized comparison retains the old update sequence.
- IRQ/NMI at all 108 reached task/instruction sites per frontend mode, including
  INX, its following TXA and the pending-refresh interval. Two tasks and scalar
  IRQ execution check full CPU, live frame and complete DP-domain restoration
  against uninterrupted steps; both seeded schedules complete correctly.
- Exact comparison of all 264 corpus records in both host profiles, with nine
  corrupted-counter/control tests rejected; 77 comparison-tool tests, seven
  disassembler tests and checks for all 14 generated corpus pairs.

This native strategy/emission slice changes no NIR or semantic contract. The
full root compiler suite and NIR sweep were outside the affected-check scope.
Historical snapshots and Exec816's compiler pin remain unchanged.

Reproduce with a release `actionc-65816` build and the retained X baseline:

```sh
python3 -B tools/compare65816/build.py --output target/loop-inx-after --verify-crlf
A816_COMPARISON_MANIFEST="$PWD/target/loop-inx-after/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/loop-inx-after/debug.json" \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -- --ignored
```

Repeat with `--release` and `release.json`. These corpus commands fail for the
retained vbcc case; the checker requires exactly that known failure:

```sh
python3 -B tools/compare65816/check_loop_inx.py \
  target/loop-x-after target/loop-inx-after \
  --inventory docs/benchmarks/65816-loop-inx/frozen.json \
  --output target/loop-inx-delta.json
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```

Removing authoritative stores/homes, promoting mutable counters, broader loops,
Y allocation and cross-call residency each require separate measured plans.
