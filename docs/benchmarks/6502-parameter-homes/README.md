# Parameter home selection measurements

Slice 0 baseline, compiler revision `8039b30b`, 2026-11-14. No placement
optimization is enabled in this receipt. See the
[implementation plan](../../MIR6502_PARAMETER_HOME_SELECTION_IMPLEMENTATION_PLAN.md).

The [baseline receipt](baseline.json) records 12 complete executable cases,
source/compiler hashes, image hashes, routine code and parameter storage, and
tracing work counts. Both origins (`$3000`, `$41C7`) give the same sizes:

| Program | ActionCart image | Standalone image |
| --- | ---: | ---: |
| TN | 9,962 | 10,069 |
| Independent controls | 323 | 467 |

Fnamecmp is 185 code bytes plus four inline parameter bytes. Independent
`StepWord` is 32 + 2; `ComparePointers` is 111 + 4. `KeepByte` already has no
inline home (three code bytes); it is a nonregression control. Exposed storage
and multiplication supply fallback controls. Controls with inlining enabled and
disabled currently produce identical images.

Reproduce using the rlib from the intended compiler build, never a stale binary:

```sh
source build/mir6502-layout-census/build-env.sh
cargo build --lib
python3 tools/compare6502/parameter_homes.py \
  --api-rlib build/checked-arena/shared-target/debug/deps/libactionc-bf811c1ef8b2df31.rlib \
  --output build/mir6502-parameter-homes/baseline
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test parameter_homes --test word_result_placement
```

The rlib hash suffix is build-specific; substitute the library just built.
Validation passed both VM targets. The new control executes real LF and CRLF
files in both runtimes at both origins, checking return ABI, exposed-cell
contents, wraparound scalar arithmetic, and pointer page crossings through
offset 512. Ordinary mapped buffers terminate at an explicit mismatch. This
baseline does not yet claim pointer address-wraparound or volatile-read coverage.

## Binding foundation (slice 1)

The shared binding representation is implemented, with production selection
still disabled. All 12 final executable hashes and routine/storage measurements
match the baseline. Validation passed 1,357 MIR6502 unit tests (15 existing
ignored cases), including five new binding tests, and 76 integration tests in
`mir6502_call_abi`, `mir6502_materialization_gap`, and `mir6502_pointer_setup`.
The latter exercises actual LF/CRLF fixture paths. New binding tests compare
planning and emitted ZP addresses, unchanged inline bytes, missing register
addresses, and rejection of malformed, exposed, and source-overlapping bindings.

## Qualified parameter placement

The [feature-off receipt](qualification-off.json) and
[feature-on receipt](qualification-on.json) use base compiler `e1f19ff5` plus
the final qualification changes, identified by the recorded MIR6502 diff and
library hashes. Configuration is explicit, independent of the default. Each covers seven programs,
both runtimes, and both origins, comparing traced and untraced final outputs.
All 12 original feature-off image hashes remain identical to the baseline.
The new independent trace and cleanup controls also remain byte-identical with
the feature enabled.

| Program | Runtime | Before | After | Saving |
| --- | --- | ---: | ---: | ---: |
| TN | ActionCart | 9,962 | 9,878 | 84 |
| TN | Standalone | 10,069 | 9,985 | 84 |
| Pointer/general controls | ActionCart | 323 | 267 | 56 |
| Pointer/general controls | Standalone | 467 | 411 | 56 |
| Scalar controls | Either | 308 | 256 | 52 |
| Register controls | Either | 154 | 106 | 48 |
| Trace controls | Either | 164 | 164 | 0 |
| Cleanup controls | Either | 99 | 99 | 0 |

Both origins produce these sizes. Fnamecmp is **105 code bytes, no inline
parameter bytes, and four bound ZP bytes**, versus 185 + 4 inline previously.
ComparePointers is 59 code + 0 inline + 4 ZP versus 111 + 4 inline. Other routine
measurements, hashes, source hashes (including TN's LIB.ACT), and exact counters
are in the receipts. Pointer controls have the same results with inlining on
and off. The scalar control gains six additional bytes beyond slice 4 by keeping
the fourth argument in its incoming `$A3` home.

All 28 feature-on cases retain the corresponding upstream attempt counts and
selection decisions. An admitted batch adds exactly one core materialization;
no-candidate controls add none. For TN, core materializations are 3 → 4, emission
invocations 43 → 83, inner emission probes 258 → 498, and preservation replays
6 → 12. Resumption is bounded but includes substantial downstream work; these
counts do not describe the entire compiler as emitting only twice.

One sequential local measurement run took 69.19 seconds disabled and 101.16
seconds enabled, with maximum RSS 260,669,440 and 295,387,136 bytes respectively.
Each total includes all 28 cases, traced and untraced; it excludes compiling the
probe/library. Per-case elapsed times and raw macOS `/usr/bin/time -l` reports
are retained. The library used the shared opt-level-1, no-debug build settings.
These single-run observations document overhead, not a statistical performance
claim or a runtime speed measurement.

Reproduce both receipts with the desired current library:

```sh
python3 tools/compare6502/parameter_homes.py \
  --api-rlib PATH/TO/libactionc-HASH.rlib \
  --output build/mir6502-parameter-homes/qualification-off
python3 tools/compare6502/parameter_homes.py \
  --api-rlib PATH/TO/libactionc-HASH.rlib --enabled \
  --output build/mir6502-parameter-homes/qualification-on
```

The measurement driver uses macOS `/usr/bin/time -l`. Generated images and probe
binaries remain under ignored build storage. The feature is enabled for optimized
builds and disabled in the conservative configuration. The
[qualification receipt](qualification.json) records passing unit, integration
and VM targets, reproduction commands and validation scope; the
[implementation plan](../../MIR6502_PARAMETER_HOME_SELECTION_IMPLEMENTATION_PLAN.md)
records the slice results.
