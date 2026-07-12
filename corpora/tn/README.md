# TOMS Navigator Corpus

```text
atr/
  tn-1.23-stryker.atr

original/extracted/
  byte-exact extraction of the TN ATR
  decoded text files use the repository ATASCII escape spelling
  raw sidecars are kept as `*.atascii`
  TN.COM is the prebuilt original program from the ATR
```

Refresh the original extraction from the repo root with:

```sh
cargo run --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs -- \
  corpora/tn/atr/tn-1.23-stryker.atr \
  extract --all -o corpora/tn/original/extracted
```

The original source files live under `original/extracted/SRC/`.
