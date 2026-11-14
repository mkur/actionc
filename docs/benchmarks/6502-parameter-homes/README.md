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
