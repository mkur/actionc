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
The historical checked-arena scorecard remains frozen. New implementation
measurements will use separate `current` bindings here.
