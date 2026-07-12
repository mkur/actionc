# Codegen Comparison Tool

`tools/compare-codegen.sh` compiles one or more focused Action! programs with
the classic backend and the MIR6502 backend, then writes comparable artifacts into
one directory per source.

This is meant for small materialization investigations where a fixture is easier
to reason about than `TN.ACT`:

```sh
tools/compare-codegen.sh --keep fixtures/mir6502/call_word_arg.act
tools/compare-codegen.sh --out-dir target/experiment-outputs/codegen fixtures/mir6502/*.act
```

For each source, the tool writes:

- `semir`, `nir`, `mir6502`, and `mir6502.materialized`;
- `classic.*` and `mir6502.*` listings, source listings, maps, load files, and
  load-file hex dumps;
- `*.diff` files for source listings, plain listings, maps, load bytes,
  normalized listings, and instruction-only listings;
- `*.listing.normalized` files, which strip listing addresses and normalize
  relocated absolute addresses to reduce layout-only noise;
- `*.listing.ops` files, which additionally remove data directives and padding
  so control-flow and instruction shape are easier to compare;
- `classic.symbols`, `mir6502.symbols`, and `symbols.diff`, which extract and
  sort listing `DATA`/`PROC` boundaries by name for relocated symbol audits;
- `mir6502.compact` and `mir6502.materialized.compact`, derived MIR views that
  remove zero-offset noise such as `global g0+0` and `(zp$AC),y+0`;
- `mir6502.spills`, a quick accounting report for allocated spill data symbols
  and referenced materialized spill IDs;
- `summary.txt`, containing classic/MIR6502 load-file byte counts and the size
  delta;
- `*.err` files for every compiler invocation, kept even when one phase fails.

Useful options:

- `--origin <addr>`: pass a non-default code origin to both backends;
- `--profile legacy|modern`: choose the profile to compare against;
- `--max-diffs <n>`: limit printed diff snippets;
- `--no-diffs`: keep diff files without printing snippets.

The script exits non-zero if either backend fails for any source. Successful
artifacts are still kept when `--keep` or `--out-dir` is used, so a MIR6502
failure can be inspected beside the last successful SemIR/NIR/MIR output.
