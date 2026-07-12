# atrcopy-rs

A tiny Rust ATR utility for the `actionc` workflow.

Current scope:

- Read ATR headers.
- List Atari DOS/MyDOS style directory entries, including MyDOS subdirectories.
- Extract individual files, subdirectories, or all visible files.
- Create a new ATR copy with host files added to the root directory.
- Decode text-like ATASCII files to the repository's ASCII escape encoding,
  keeping the raw bytes beside them as `.atascii`.
- Handle 128-byte and 256-byte sector ATR images, including the first three
  128-byte boot sectors in 256-byte images.

Usage:

```sh
cargo run -- <disk.atr> list
cargo run -- <disk.atr> extract FILE.EXT -o target/out
cargo run -- <disk.atr> extract SRC/FILE.ACT -o target/out
cargo run -- <disk.atr> extract SRC -o target/out
cargo run -- <disk.atr> extract --all -o target/out
cargo run -- <disk.atr> extract --all -o target/out --text=always
cargo run -- <disk.atr> extract --all -o target/out --raw-only
cargo run --bin atrcopy-rs -- <disk.atr> add -o <out.atr> FILE.COM=FILE.COM
cargo run --bin atascii-to-ascii -- FILE.ACT.atascii FILE.ACT
cargo run --bin ascii-to-atascii -- FILE.ACT FILE.ACT.atascii
```

Text extraction defaults to `--text=auto`. Files with source/document-style
extensions such as `.ACT`, `.ASM`, `.DOC`, `.TXT`, `.EXC`, `.DEM`, `.DM1`, and
`.DM2` are written as ASCII text and also preserved as raw `*.atascii`.
Binary-looking files, such as `.COM`, are written raw. Use `--text=always` or
`--text=never` when an image uses unusual extensions.

`add` is declarative: it reads the input ATR and writes a separate output ATR,
leaving the source image untouched. Each file spec is `host-path` or
`host-path=ATARI.EXT`; omitted Atari names are inferred from the host filename.
Writing currently targets the root directory and uses Atari DOS/MyDOS sector
chains.

The ASCII encoding uses the same escape spellings accepted by `actionc`:

```text
\{$HH}       exact ATASCII byte
\{CHAR:$HH}  exact ATASCII byte
\{RETURN}    $9B
\{ESC}       $1B
\{CLEAR}     $7D
\{INV:text}  inverse-video ASCII bytes
```

Examples:

```sh
cargo run -- '../../corpora/toolkit/atr/The_ACTION_Toolkit.atr' list
cargo run -- ../../surveys/probes/original-compiler/outputs/ACTION-37-MYDOS.atr \
  extract FUNC.ACT FUNC.COM -o target/action37
cargo run --bin atrcopy-rs -- ../../corpora/tn/atr/tn-1.23-stryker.atr \
  add -o target/tn-with-actionc.atr ../../TN-C.COM=TN.COM
```

This is intentionally small. It is not a full `atrcopy` replacement yet; add DOS
formats and sector-chain variants as we find real images that need them.
