# Bounded native incoming-parameter word forwarding

Implementation `4d7fcb3` removes a repeated direct load of an immutable incoming
word when A16 and full N/Z still describe its actual read. It implements the
[bounded plan](MIR65816_PARAMETER_FORWARDING_PLAN.md), preserving public ABI v1,
image v3, o65 profile v1, stack guards, interrupt reserves, stores and homes.

## Selection and state

Two same-block sequences qualify: a parameter Load/capture immediately followed
by another Load of that parameter, or the same sequence with one Store of the
exact captured temporary into a disjoint non-addressable frame word. Every
capture remains. The optional Store must already omit its source load through
ordinary temporary forwarding; its entire emission must be one STA16.

A separate incoming-read witness coexists with ordinary Temp/Frame forwarding.
It checks ParamId, final allocated incoming offset, read and capture generations,
A16, full N/Z, zero transient S displacement and exact instruction/label cursors.
A real typed LDA establishes the read relation without admitting incoming memory
as a writable private home. An omitted read cannot rearm the witness. Its retained
capture still publishes ordinary temporary forwarding.

The classifier excludes mutable or address-taken parameter homes and scans typed
operations for writes, address escape, Copy use and noncanonical access. Byte,
wider, indexed, indirect and volatile accesses retain their original paths.
Other operations, calls/helpers, labels, stack movement and mode changes revoke
or stale the witness. No value is kept across a call or join. The
[emission contract](MIR65816_EMISSION_CONTRACT.md) records these invariants.

## Measured results

The [comparison snapshot](benchmarks/65816-parameter-forwarding/after/tables.md)
and [exact delta](benchmarks/65816-parameter-forwarding/delta.json) confirm every
frozen forecast. Only raw rotation and raw recursion change; all 26 other Action
builds, including every optimized build, and all vbcc artifacts retain their code.
For input 13:

| Measurement | Raw rotation before → after | Raw recursion before → after |
| --- | ---: | ---: |
| Code bytes | 172 → 170 | 208 → 206 |
| Cycles | 1,226 → 1,221 | 3,467 → 3,402 |
| Instructions | 304 → 303 | 1,059 → 1,046 |
| Stack-byte reads | 135 → 133 | 256 → 230 |
| Stack-byte writes | 216 → 216 | 290 → 290 |
| Observed stack peak | 18 → 18 | 190 → 190 |

Across vectors per incoming I state, the two sites save 28 instructions,
140 cycles and 56 stack-byte reads. Recursion at zero still saves two static
bytes but no dynamic work. Fixed frames remain 18 and 8 bytes respectively.
Optimized rotation stays 138 bytes / 916 cycles / 16 stack bytes; optimized
`sum_loop(13)` stays 120 / 1,212 / 12. Existing temp/frame forwarding counters,
DP traffic, guard costs and results remain unchanged. The known optimized vbcc
`unlink` vector-0 failure remains reported.

## Independent qualification

The [exact checker](../tools/compare65816/check_parameter_forwarding.py) starts
from frozen old images and removes only the two declared LDA instructions.
It checks complete executable images, including later uncounted routines,
relocated branches/calls/PER, stack-budget metadata and all 264 records. A target
into a removed instruction is rejected. The reviewed emission snapshot changes
only raw recursion's bytes and shifted labels/fixups/spans; the expected change
was derived before comparing current emission. No NIR fixture changed.

The native evidence index derives each proof from verified MIR identities,
allocated homes, operation spans and independently decoded instructions.
Every proof-byte mutation is rejected. Reached CPU boundaries check A against
actual incoming bytes and full N/Z; new Action-only counters report these
executions separately from existing temp/frame forwarding.

Source probes execute in both frontend modes. For positive optimized target
coverage, verified MIR probes retain the raw typed worker in the independently
optimized surrounding program, since source optimization removes repeated
loads. Both forms, distinct/reused capture homes and argument ordinal one are
covered. Boundary values execute with both I masks, every incoming C/V
combination, serialized flat images and two o65 placements. Trace-on/off bytes,
labels, fixups and spans agree; VM traces validate simultaneous A/home/NZ facts.

Two tasks use different live argument values for the same lexical parameter.
IRQ and NMI injection at the initial read, each retained store and the consumer
checks complete registers and live argument/frame/return memory against an
independent uninterrupted instruction. Seeded schedules also complete correctly.
Refusal tests cover identity/generation, writes and aliases, flags, modes,
labels, calls, barriers, authoritative mutable homes, malformed metadata,
transient S and last-byte stack-relative bounds. Third loads must read again;
two or unrelated intervening stores cannot extend the permission.

[Qualification](abi/action65816-parameter-forwarding-qualification.json) records
87 compiler emitter/proof tests, 60 affected integration tests, 57 comparison-tool
tests and the 14-kernel generator check. All 104 native tests pass in both host
profiles, with 429 identical inputs and 610 identical artifacts. Corpus debug
and release records agree exactly. Actual LF/CRLF compilation covers all 28
Action builds. An isolated CRLF checkout passes the reviewed emission snapshot
and 17 native forwarding/preemption/state tests; all 366 artifacts match LF.
The full root suite and NIR sweep were unnecessary for this target-only change.

Reproduce after building the release `actionc-65816` binary:

```sh
python3 -B tools/compare65816/build.py --output target/parameter-forwarding-after --verify-crlf
A816_COMPARISON_MANIFEST="$PWD/target/parameter-forwarding-after/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/parameter-forwarding-after/debug.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -- --ignored
```

Repeat the observer with `--release` and `release.json`. Both observers save all
records before reporting the known vbcc failure. Then run:

```sh
python3 -B tools/compare65816/check_parameter_forwarding.py \
  target/frame-forwarding-after target/parameter-forwarding-after \
  --baseline docs/benchmarks/65816-parameter-forwarding/baseline.json \
  --output docs/benchmarks/65816-parameter-forwarding/delta.json
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```

Keep historical baselines immutable. Edge-home coalescing and scalar DP
allocation remain separate measured slices; Exec816 adoption remains separate.
