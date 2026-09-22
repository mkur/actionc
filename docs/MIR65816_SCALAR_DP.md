# Native scalar DP allocation

The [implementation plan](MIR65816_SCALAR_DP_PLAN.md) is complete. Inventory
`3af8799` froze typed admission and exact transforms; prerequisite `0298e49`
introduced tagged word selection/tracking without changing output; allocator
`c805e48` enabled bounded scalar residency; `78d0a13` added qualification probes.

## Allocation contract

The pointer-leaf strategy remains first. General routines first receive the
verified stack allocation and existing edge coalescing. A whole-routine whitelist
then admits ordinary word temps, compare-produced stack Booleans, direct safe
word frame/parameter memory, native ADD/SUB and comparisons, word edges and
word/void returns. Calls, helpers, pointer operations, casts, escaped/volatile
storage and unknown effects retain the existing stack strategy.

Up to 16 complete word-home classes move to aligned D+$20..D+$3F slots, ordered
by their old stack offset. Capacity failure retains all original homes. Existing
coalesced identities and closed-operation conflicts remain. Fixed objects,
parameters and Boolean offsets stay on the stack; cyclic staging is repacked
above them. A separate mixed-location verifier checks liveness, types, ownership,
edge costs, staging, bounds and exact frame accounting.

Private tracker facts distinguish S-relative and D-relative bytes. Native word
arithmetic, comparisons, returns, frame/parameter captures and parallel copies
use both spaces without losing adjacent forwarding. Partial writes invalidate
whole overlapping words. Calls, unmodelled writes and joins retain conservative
barriers; liveness establishes loop residency without propagating tracker
permissions across labels.

The scratch partition is internal to selection. All 64 scratch bytes remain
call-clobbered and owned by the current aligned domain. Public ABI v1, image v3,
o65 profile v1, stack guards and interrupt reserves remain. New scalar maps need
a matching validator. o65 keeps DP operands literal and allocates no application
scratch in its zero segment. Exec816's pin is unchanged.

## Measured results

The [exact delta](benchmarks/65816-scalar-dp/delta.json) checks all 28 complete
Action images/maps and all 264 debug/release records against the immutable
[inventory](benchmarks/65816-scalar-dp-inventory/inventory.json). Fourteen builds
change exactly as forecast; the other fourteen remain identical. The checks
include uncounted wrappers, relocated references, every per-PC counter, full
argument/frame maps and unchanged vbcc controls.

| Representative call | Bytes before → after | Cycles before → after | Observed stack peak before → after |
| --- | ---: | ---: | ---: |
| Identity(13), either mode | 59 → 51 | 63 → 49 | 4 → 0 |
| Add/subtract(13,41), either mode | 70 → 62 | 90 → 72 | 8 → 0 |
| Constant chain(13), raw | 155 → 147 | 223 → 193 | 6 → 0 |
| Constant chain(13), optimized | 65 → 57 | 73 → 58 | 6 → 0 |
| Maximum(13,41), either mode | 90 → 90 | 94 → 90 | 6 → 2 |
| Rotation(13), optimized | 130 → 130 | 896 → 793 | 16 → 8 |
| Sum loop(13), optimized | 120 → 120 | 1,212 → 1,092 | 12 → 6 |
| Direct calls(13,41), either mode | 319 → 311 | 466 → 436 | 20 → 14 |

At input 13, sum-loop replaces 40 word reads and 80 writes with DP accesses:
stack bytes become 85 reads / 28 writes, resident DP bytes 80 / 160. Rotation
moves 51 reads and 52 writes: stack bytes become 21 / 32, resident DP 102 / 104.
Both retain the same instructions except operand/addressing changes; zero-frame
routines additionally lose the existing eight-byte, 13-cycle teardown.

Forwarding, coalescing and logical copy counts retain their values. The delta
reports resident DP traffic separately from selector scratch and guard metadata.
The [full comparison](benchmarks/65816-scalar-dp/after/tables.md) retains the known
optimized vbcc `unlink` vector-0 error; both corpus commands save all measurements
then fail that case. All Action records are correct.

## Qualification

[Saved qualification](abi/action65816-scalar-dp-qualification.json) binds 436
compiler/fixture inputs and 648 identical artifacts across both native host
profiles. All 111 native tests pass in each profile. Root validation covers 100
emitter/proof unit tests and 61 affected image/ABI/CLI/o65/boundary integration
tests. The comparison tools pass 65 tests, the disassembler five, and all 14
corpus generator cases pass. Five deliberate counter/forecast corruptions are
rejected by the final checker.

Independent ca65 encodings and native execution check word boundaries, signed
bit patterns, flags, partial writes and separate stack/DP generations. Capacity
probes execute one and 16 resident homes and the 17-class all-stack fallback;
trace-on/off bytes, labels, fixups and spans agree. Existing raw/optimized copy,
forwarding, helper-clobber, recursion, alias, guard-fault and two-placement o65
coverage remains green. The unchanged pointer corpus checks its original maps
and machine code exactly.

The new task/IRQ probe injects IRQ and NMI at all 108 reached task/instruction
sites per frontend mode in a scalar rotation leaf, including staged cycles.
Uninterrupted steps establish the expected full CPU state, live frame and entire
256-byte suspended DP domain. Two tasks hold different resident values while
IRQ dispatch executes scalar code using its own DP. Seeded schedules also
complete correctly. The raw-mode target probe retains independently verified
optimized worker MIR so both frontend modes exercise positive scalar residency.

An isolated CRLF checkout rebuilds the reviewed emission snapshot and 64 native
tests; all 616 saved artifacts match LF. All 28 corpus source builds also agree
under LF/CRLF. The reviewed machine snapshot intentionally changes scalar homes,
frame operands and zero-frame teardown; no NIR or semantic contract changes.
Historical reports remain immutable.

Reproduce after building `actionc-65816` in release mode:

```sh
python3 -B tools/compare65816/build.py --output target/scalar-dp-after --verify-crlf
A816_COMPARISON_MANIFEST="$PWD/target/scalar-dp-after/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/scalar-dp-after/debug.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -- --ignored
```

Repeat with `--release` and `release.json`, retaining the known external failure.
With the preserved edge-coalescing baseline available:

```sh
python3 -B tools/compare65816/check_scalar_dp.py \
  target/edge-coalescing-after target/scalar-dp-after \
  --inventory docs/benchmarks/65816-scalar-dp-inventory/inventory.json \
  --output target/scalar-dp-delta.json
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```
