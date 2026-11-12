# MIR6502 home value invalidation controls

The baseline compiler sources are pinned to `95e334be`. The checked library
is unchanged. [baseline.json](baseline.json) binds four freshly compiled
TN/TNDBG images, Alloc listings, routine maps and storage placement records.
[neutral-baseline.json](neutral-baseline.json) binds the raw/optimized NIR
controls and observed VM cycles.

The compiler and before-change measurement executables are saved under
`build/home-value-invalidation/baseline/`. The source fixture contains an
ordinary captured word comparison plus whole-word and high-byte mutation
controls. It is parsed from real LF/CRLF files in both runtimes and origins.
The VM asserts numerical results and source mutations, not assembly templates.

```sh
python3 tools/compare6502/tn/home_values.py --compiler build/home-value-invalidation/baseline/actionc --phase baseline --check
cargo test --test mir6502_home_value_invalidation
ACTIONC_HOME_VALUE_TN_IMAGES="$PWD/build/home-value-invalidation/baseline" cargo test --release --manifest-path tools/vm-runtime-tests/Cargo.toml --test home_value_invalidation
```

Baseline: Alloc is 99 bytes; successful Alloc(1) takes 223 cycles including
the 17-cycle wrapper. Four real images pass 44 normal/fault controls. The
neutral matrix passes 3,384 calls, including source and partial-word mutations.
The historical checked-arena scorecard remains frozen. Implementation measurements use separate `current` bindings here.


The implementation and qualification are recorded in [RESULTS.md](RESULTS.md).
[current.json](current.json) binds fresh final images and listings.
[baseline-details.json](baseline-details.json) and
[current-details.json](current-details.json) bind frame labels, zero-page
allocations, MIR hashes, accepted rewrite counts, retained-fact peaks, neutral
images and measured cycles. [current-mads.json](current-mads.json) binds the
sixteen real-image reassemblies and relocations.

Build `actionc` and `actionc-emit` together with the same source and profile.
The frozen baseline pair must come from `95e334be`. The detailed check rejects
a stale current emitter without the new fact-count diagnostics.

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo build --bin actionc --bin actionc-emit
python3 tools/compare6502/tn/home_values.py --compiler target/debug/actionc --phase current --check
ACTIONC_HOME_VALUE_OUTPUT="$PWD/build/home-value-invalidation/current-neutral" cargo test --test mir6502_home_value_invalidation
ACTIONC_HOME_VALUE_TN_IMAGES="$PWD/build/home-value-invalidation/current" ACTIONC_HOME_VALUE_ALLOC_CYCLES="$PWD/build/home-value-invalidation/current/alloc-cycles.tsv" ACTIONC_HOME_VALUE_VM_RESULTS="$PWD/build/home-value-invalidation/current/neutral-cycles.tsv" cargo test --release --manifest-path tools/vm-runtime-tests/Cargo.toml --test home_value_invalidation
python3 tools/compare6502/tn/home_values.py --compiler target/debug/actionc --phase current --details --check
python3 tools/compare6502/tn/home_values.py --compiler target/debug/actionc --phase current --mads --check
cargo test --test mir6502_home_value_invalidation mads -- --ignored
```

Use the commands without `--check` only when intentionally refreshing this
task’s bindings. Keep the historical scorecards frozen. Detailed checks use the saved neutral
JSON/XEX and VM cycle files from the preceding commands. Main image checks
recompile all four configurations; detailed checks rematerialize their MIR.
MADS checks also compile the relocated real images. All text hashing normalizes
CRLF; binary and ATASCII data remain byte-exact.
