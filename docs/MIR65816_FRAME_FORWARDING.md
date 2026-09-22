# Direct native frame store/load forwarding

The emitter forwards a stored word in A16 into the next load's retained temporary
capture. Selection is independent of frontend optimization mode. This implements
the first recommendation from the [movement inventory](MIR65816_MOVEMENT_INVENTORY.md).

## Eligibility and state

Both MIR operations must be nonvolatile, two-byte, direct accesses to the same
non-addressable `AutomaticFrame` object and byte displacement. The store source
must be a word temporary; the load destination must have a complete stack home.
The selector checks object extent, both stack-relative bytes and the retained
destination store before requesting omission. Incoming parameter reloads and
address-taken, external, indexed, indirect, byte and wider accesses are excluded.

The existing state tracker uses a distinct `Frame(object, displacement)` identity
alongside `Temp(id)`. The witness requires A16, full-word N/Z, the exact physical
home and generation, zero transient S displacement and an unchanged instruction
and label cursor. A matching numeric address does not substitute for identity.
Overlapping writes, unknown writes, calls/helpers, other operations, labels,
width changes and stack movement invalidate the witness or make its cursor stale.
The witness is consumed once. The retained load capture can then publish the
existing temporary witness for arithmetic, comparison, store or return.

For the measured rotation, the final sequence changes from:

```asm
LDA $0A,S
STA $02,S
LDA $02,S       ; omitted
STA $0C,S
```

Both stores and all homes remain. The rule does not allocate registers across
calls or joins, change DP use, move memory effects or remove stack guards.
The ABI remains `action65816.native.v1`, image version 3 and o65 profile version 1.

## Qualification

The [frozen forecast](benchmarks/65816-frame-forwarding/baseline.json) identifies
one instruction in the qualified selective-staging image. The
[exact checker](../tools/compare65816/check_frame_forwarding.py) constructs the
expected complete image by deleting that LDA and relocating later positions.
It checks all executable segments, including the uncounted driver, all metadata,
all 28 Action images, vbcc artifacts and every measurement field. Existing
adjacent-temp forwarding counters keep their meaning; `frame_forwarded_loads`
and `frame_forwarded_load_sites` report the new proof family separately.

The test-only frame index combines verified MIR object facts and operation spans
with independently decoded final instructions. It walks only retained word
stores back to a word LDA/ADC/SBC and rejects labels inside the proof window.
Byte mutations invalidate each proof. The VM checks A against the stored object
and checks complete N/Z at every reached forwarding boundary.

Generated raw/optimized probes exercise zero, sign and wrap boundaries with both
incoming IRQ masks, flat images and two o65 placements. Tracker trace-on/off
checks cover bytes, labels, spans, fixups and actual home generations through
rotation loops, including rebased o65 execution. Two-task probes inject IRQ and
NMI at each retained store, independently execute the interrupted instruction,
and require exact restoration of every register and the live invocation frame.
Seeded IRQ/NMI schedules also retain correct task results. Negative selector and
state tests cover identity, generation, partial writes, aliases, volatility,
flags, mode, labels, calls, transient S and source/destination extents.

The historical movement inventory remains frozen; its incoming-parameter
candidates and edge coalescing are separate future slices.

## Measured result

The [saved comparison](benchmarks/65816-frame-forwarding/after/tables.md) and
[exact delta](benchmarks/65816-frame-forwarding/delta.json) confirm the forecast.
Only optimized `loop_rotation` changes; all 27 other Action builds and all vbcc
artifacts retain their code. For input 13:

| Measurement | Before | After |
| --- | ---: | ---: |
| Code bytes | 140 | 138 |
| Cycles | 956 | 916 |
| Instructions | 230 | 222 |
| Stack-byte reads | 143 | 127 |
| Stack-byte writes | 140 | 140 |
| Frame / stack peak | 16 | 16 |

Each of the six vectors executes the site eight times: 48 fewer instructions,
240 fewer cycles and 96 fewer stack-byte reads per incoming I state. Existing
temp forwarding counts, guard costs, DP traffic, frames and results remain.
Optimized `sum_loop(13)` stays 120 bytes / 1,212 cycles / 12 stack bytes. The
known optimized vbcc `unlink` vector-0 failure remains visible.

[Qualification](abi/action65816-frame-forwarding-qualification.json) records
80 root emitter/proof tests, 60 affected integration tests, 50 comparison-tool
tests and the 14-kernel generator check. All 101 native tests pass in each host
profile, with 425 identical compiler/fixture inputs and 474 identical artifacts.
Debug/release corpus records agree exactly. Actual LF/CRLF corpus compilation
covers 28 Action builds. A separate CRLF checkout passes the emission snapshot
and 14 forwarding/state/preemption tests; all 230 emitted artifacts match LF.
The existing reviewed emission snapshot remains unchanged. No NIR contract or
NIR fixture changed, so the full root suite and NIR sweep were not required.

Reproduce after building `actionc-65816` in release mode:

```sh
python3 -B tools/compare65816/build.py --output target/frame-forwarding-after --verify-crlf
A816_COMPARISON_MANIFEST="$PWD/target/frame-forwarding-after/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/frame-forwarding-after/debug.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -- --ignored
```

Repeat the observer with `--release` and `release.json`. Both runs save all 264
records before reporting the known vbcc failure. Then run:

```sh
python3 -B tools/compare65816/check_frame_forwarding.py \
  target/selective-staging-after target/frame-forwarding-after \
  --baseline docs/benchmarks/65816-frame-forwarding/baseline.json \
  --output docs/benchmarks/65816-frame-forwarding/delta.json
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```

The exact checker requires the preserved selective-staging baseline artifacts.
Keep the historical inventory and snapshots frozen when measuring later slices.
