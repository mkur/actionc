# `actionc-compare`

`actionc-compare` compares `actionc` output across codegen profiles, optionally
against an original Action! compiler `.COM` capture.

Example:

```sh
cargo run --bin actionc-compare -- \
  --original surveys/probes/original-compiler/outputs/vm/ARRASN.COM \
  surveys/probes/original-compiler/array_assign.act
```

With VM-generated original symbol dumps, it can also print a labeled
original-side disassembly:

```sh
cargo run --bin actionc-compare -- \
  --origin '$0E08' \
  --original surveys/probes/original-compiler/outputs/vm/MODSYM.COM \
  --original-symbols surveys/probes/original-compiler/outputs/vm/MODSYM.symbols.json \
  --original-symbol-snapshots surveys/probes/original-compiler/outputs/vm/MODSYM.symbol-snapshots.json \
  --disassemble-original \
  surveys/probes/original-compiler/module_symbols.act
```

The report includes:

- Atari load-file segment layout for original, `compat`, and `modern`
- load-file byte size and exact-match status
- first byte differences
- `--origin <addr>` for generated profiles; when omitted with `--original`,
  generated profiles default to the original code segment origin
- opcode counts from disassembly
- optional original disassembly labeled from original global/local symbol dumps,
  including simple branch labels and operand comments such as `arg+1`
- routine-relative original-vs-generated diffs when original symbol dumps are
  provided, with offsets such as `Circle+$0042`, original symbol annotations,
  and generated-side source spans
- structured codegen map summaries for generated profiles, including skipped
  ranges and address-sorted storage symbols
- per-routine range comparison for `compat` vs `modern`, including mapped
  source locations, source-range delta grouping, and snippets for localized
  instruction diffs
- normalized instruction diffs for original-vs-compat, original-vs-modern, and
  compat-vs-modern

The disassembler is intentionally simple and linear. Mixed code/data segments
can make opcode counts approximate because inline data may decode as 6502
instructions. Routine-range diffs use `actionc`'s codegen map and are therefore
available only for generated profiles, not original compiler captures.
Byte-level exactness remains authoritative for compatibility.
