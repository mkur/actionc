# TNVAL Probe Notes

Probe: `tn_value_index.act`

Purpose:

- Isolate TN drift patterns around `Value(v(i))`, `Value(v(i)+1)`, and the
  `Instr()` string-scan loop.
- Keep the source small enough that original-vs-`actionc` byte differences can
  be inspected directly.

Current comparison:

| Compiler | File size | Code segment | RUNAD |
| --- | ---: | --- | --- |
| Original Action! | 319 | `$3000-$3132` | `$30F8` |
| `actionc` | 318 | `$3000-$3131` | `$30F7` |

`actionc` is 1 byte smaller on this isolated pattern.

Important byte-level finding:

- The original compiler does not append an extra `RTS` after the inline
  machine-code `Value()` routine.
- `actionc` now matches this and does not emit an implicit routine `RTS` when a
  routine body ends in a raw machine block.

Further byte-level finding:

- `IsProtected` uses a shorter original shape for `Value(v(i)+1)`: it computes
  the `+1` offset directly while loading the array-derived pointer.
- `actionc` now matches that shape for dynamic CARD ARRAY element plus constant
  staging, avoiding the prior `$AC/$AD` temporary copy.

Further byte-level finding:

- `Instr` now caches the byte `FOR ... TO` bound for indexed expressions such as
  `s(0)`, matching the original "evaluate TO once" behavior.
- Indexed byte equality conditions now use the shorter original-style
  `EOR`/branch shape instead of materializing through a zero-page temporary.

Further byte-level finding:

- Byte function-return equality against a constant now branches directly with
  `LDA return / EOR #const / BEQ|BNE`, avoiding the prior `$AC` temporary and
  generic compare scaffold.

Remaining observed difference:

- `actionc` is now slightly smaller than the original in this probe because it
  reuses known `Y=0` in `Instr` where the original reloads `LDY #0`. This is a
  deliberate retained flexibility point rather than a bug to reproduce.
