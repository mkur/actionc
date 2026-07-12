# ACTION! Toolkit Corpus

```text
atr/
  The_ACTION_Toolkit.atr

original/extracted/
  byte-exact extraction of the Toolkit ATR
  decoded text files use the repository ATASCII escape spelling
  raw sidecars are kept as `*.atascii`

extracted-raw/
  scratch raw extraction material retained from local investigations
```

Refresh the original extraction from the repo root with:

```sh
cargo run --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs -- \
  corpora/toolkit/atr/The_ACTION_Toolkit.atr \
  extract --all -o corpora/toolkit/original/extracted
```

Text files are decoded with the existing repository escape syntax, for example
`\{INV:...}`, `\{CLEAR}`, and `\{$HH}`. `MUSIC.SCR` is a raw screen dump, not
source text; keep it as raw bytes.
