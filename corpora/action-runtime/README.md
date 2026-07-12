# ACTION! RunTime Disk Corpus

```text
The ACTION! RunTime Disk.atr
extracted/
  byte-exact extraction of the runtime disk
  decoded text files use the repository ATASCII escape spelling
  raw sidecars are kept as `*.atascii`
```

Refresh the extraction from the repo root with:

```sh
cargo run --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs -- \
  "corpora/action-runtime/The ACTION! RunTime Disk.atr" \
  extract --all -o corpora/action-runtime/extracted
```
