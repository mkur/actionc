# Toolkit Output Comparison

This note tracks original-vs-actionc output comparison for Toolkit sources.
The survey only proves that sources pass codegen; this note records load-file
and routine-shape gaps against original compiler VM captures.

Last refreshed: 2026-05-30.

Current VM captures:

- `outputs/vm/ABS.COM`
- `outputs/vm/ALLOCATE.COM`
- `outputs/vm/CHARTEST.COM`
- `outputs/vm/CIRCLE.COM`
- `outputs/vm/CONSOLE.COM`
- `outputs/vm/IO.COM`
- `outputs/vm/JOYSTIX.COM`
- `outputs/vm/PMG.COM`
- `outputs/vm/PRINTF.COM`
- `outputs/vm/REAL.COM`
- `outputs/vm/SORT.COM`
- `outputs/vm/TURTLE.COM`

Use `actionc-compare` without `--origin` for these captures. The tool then
defaults generated profiles to the original code segment origin, which avoids
relocation noise in absolute operands. Original Toolkit sources that now rely
on old-profile idioms are compared either through a harness or through the
modernized copies under `samples/toolkit/modern`.

## Commands

```sh
cargo run -q --bin actionc-compare -- \
  --original surveys/toolkit/outputs/vm/CIRCLE.COM \
  --original-symbols surveys/toolkit/outputs/vm/CIRCLE.symbols.json \
  --original-symbol-snapshots surveys/toolkit/outputs/vm/CIRCLE.symbol-snapshots.json \
  --max-diffs 30 \
  corpora/toolkit/original/extracted/CIRCLE.ACT

cargo run -q --bin actionc-compare -- \
  --original surveys/toolkit/outputs/vm/PMG.COM \
  --original-symbols surveys/toolkit/outputs/vm/PMG.symbols.json \
  --original-symbol-snapshots surveys/toolkit/outputs/vm/PMG.symbol-snapshots.json \
  --max-diffs 40 \
  --mode modern \
  samples/toolkit/modern/PMG.ACT
```

## CIRCLE.ACT

Summary:

- Original: 622 load bytes, 610 code bytes, origin `$0E08`.
- Compat: 613 load bytes, 601 code bytes, origin `$0E08`.
- Modern: 553 load bytes, 541 code bytes, origin `$0E08`.
- `Abs` is byte-identical after relocation normalization.
- `Circle` is 10 bytes smaller in actionc.

Main differences:

- Straight-line arithmetic temporaries use `$AC/$AD` where the original uses
  `$AE/$AF` for `Phiy=Phi+y1+y1+1` and `Phixy=Phiy-x1-x1+1`. This is a register
  allocation/style difference, not an obvious semantic gap.
- `IF Abs(Phixy)+0<Abs(Phiy) THEN` differs. The original preserves the first
  call result with stack `PHA/PLA` and keeps the explicit `+0`; actionc removes
  the identity add and keeps the first call result in zero-page slots. This is
  semantically compatible and explains most of the 9 byte load-size reduction.

Recommended follow-up:

- No urgent codegen fix. If strict compat parity becomes important, add an
  opt-in compat shape for materialized call comparisons that preserves the
  original stack-based save/restore and identity `+0`.

## PMG.ACT

Summary:

- Original: 1866 load bytes, 1854 code bytes, origin `$0E08`.
- Modern copy: 1604 load bytes, 1592 code bytes, origin `$0E08`.
- The original `PMG.ACT` source currently fails the shared semantic front door
  on old pointer/argument idioms. Use `PMG.ACT` for modern-profile
  comparison and keep the historical compat notes as background only.

Main differences:

- The modern copy makes the `Zero(@GraphP0,5)` pointer argument explicit and
  stages old ambiguous address expressions before backend comparison.
- The modern profile is still 262 code bytes smaller than the original VM
  capture. Most of that is expected profile behavior: skipped large local byte
  array backing, trampoline elision, branch inversion, and final-RTS removal.
- This report is no longer a compat parity report for original `PMG.ACT`; it is
  a modernized-source comparison against the VM capture.

Recommended follow-up:

- Keep compat parity analysis separate from modernized-source analysis.
- If strict original-source compat matters again, first decide whether the
  semantic front door should continue accepting the old pointer idioms in
  compat mode.

## Full Toolkit Capture Comparison

Reports are stored in `outputs/compare/*.txt`. Reports with `_modern` in the
filename use the modernized source copy rather than the original Toolkit file.

Sizes are split into load bytes and code bytes. `Compat delta` is measured
against original code bytes, because code bytes are the useful signal once the
Atari load-file wrapper is accounted for.

| Source | Orig load | Orig code | Compat load | Compat code | Compat delta | Modern source | Modern load | Modern code | Status | Next focus |
| --- | ---: | ---: | ---: | ---: | ---: | --- | ---: | ---: | --- | --- |
| `ABS.ACT` | 66 | 54 | 66 | 54 | 0 | original | 33 | 21 | exact compat | done |
| `ALLOCATE.ACT` | 1026 | 1014 | 1024 | 1012 | -2 | `ALLOCATE.ACT` | 947 | 935 | near exact compat | store-order byte shape |
| `CHARTEST.ACT` | 248 | 236 | 248 | 236 | 0 | original | 210 | 198 | exact compat | done |
| `CIRCLE.ACT` | 622 | 610 | 613 | 601 | -9 | original | 553 | 541 | smaller compat | call compare shape |
| `CONSOLE.ACT` | 284 | 272 | 284 | 272 | 0 | original | 248 | 236 | exact compat | done |
| `IO.ACT` | 480 | 468 | 480 | 468 | 0 | original | 427 | 415 | exact compat | done |
| `JOYSTIX.ACT` | 155 | 143 | 155 | 143 | 0 | original | 152 | 140 | same-size compat delta | temp allocation shape |
| `PMG.ACT` | 1866 | 1854 | n/a | n/a | n/a | `PMG.ACT` | 1604 | 1592 | original source semantic-blocked | modern-copy deltas |
| `PRINTF.ACT` | 1895 | 1883 | n/a | n/a | n/a | `PRINTF.ACT` | 1863 | 1851 | original source semantic-blocked | modern-copy deltas |
| `REAL.ACT` | 1404 | 1392 | 1404 | 1392 | 0 | `REAL.ACT` | 1330 | 1318 | exact compat | done |
| `SORT.ACT` | 2365 | 2353 | n/a | n/a | n/a | `SORT.ACT` | 2441 | 2429 | original source semantic-blocked | modern-copy size |
| `TURTLE.ACT` | 918 | 906 | 918 | 906 | 0 | original | 878 | 866 | exact compat | done |

`ALLOCATE.ACT` compat was compared through `allocate_harness.act`, matching the
VM capture harness that defines `CARD EndProg`; its modern column uses
`ALLOCATE.ACT`.

Main findings:

- `ABS.ACT` is a clean exact match in compat. Modern deliberately elides the
  entry trampoline, tail-calls the final return path, and removes the duplicate
  final `RTS`.
- `CHARTEST.ACT` is exact in compat. The remaining modern delta is expected
  branch-inversion and final-RTS removal.
- `CIRCLE.ACT` is 9 bytes smaller in compat. `Abs` is byte-identical after
  relocation normalization; the remaining `Circle` delta comes from actionc
  dropping an identity `+0` and using zero-page result slots instead of the
  original stack save/restore around a materialized call comparison.
- `CONSOLE.ACT` is now exact in compat. The final fixes were the original
  4-byte global `CARD` vector cells after an absolute alias, routine-name
  assignment through trampoline operand bytes, and the original bit-test
  zero-branch shape for `(console&mask)=0`. Modern is smaller through expected
  trampoline elision, branch inversion, redundant reload removal, and final
  `RTS` removal.
- `IO.ACT` is exact in compat after the negative-constant and IOCB address
  arithmetic fixes. Modern remains smaller through expected optimization wins.
- `JOYSTIX.ACT` has the same compat size as the original, but no longer compares
  byte-exact in the current tree: the refreshed report shows zero-page temp
  allocation differences in the `RETURN (value(ports(port)&3))` path.
- `PMG.ACT`, `PRINTF.ACT`, and `SORT.ACT` original sources currently do not
  refresh through `actionc-compare`; the shared semantic front door rejects old
  pointer/call-return idioms before codegen mode is reached. Their modern
  columns use the modernized source copies.
- `REAL.ACT` is exact in compat. The modern column uses `REAL.ACT`,
  which replaces routine-name assignment with a direct ROM call.
- `PRINTF.ACT` and `PMG.ACT` remain smaller than the VM captures,
  while `SORT.ACT` is larger than the original by 76 code bytes. These
  are modernized-source comparisons, not strict compat parity failures.
- `TURTLE.ACT` is exact in compat. Modern is smaller through expected branch
  inversion, tail calls, register reload removal, and final `RTS` removal.
- Historical compat note: prior PMG analysis had narrowed the residual gap to
  `PMGraphics`-style code shape after descriptor and local-storage fixes, but
  that original-source compat report is not refreshable in the current tree.
- `REAL.ACT` historical compat fixes included unsized absolute byte
  array pointer initialization (`BYTE ARRAY LBuff=$580`), clearing compatible
  `Y` hints across `WHILE` exits, and high-byte-first routine trampoline
  retargeting for `Junk=ROM_IFP`.
- `ALLOCATE.ACT` is now within 2 bytes in compat after reusing prepared
  record-field pointers across word compare/equality chains, direct word
  compound add/sub stores, equality true-path `Y=1`, and original-shaped
  materialized word equality. The remaining difference is mostly store ordering:
  actionc keeps a couple of shorter but equivalent indirect word stores instead
  of padding back to the cartridge compiler's byte shape.

Recommended follow-up:

1. Treat `ALLOCATE.ACT` as close enough unless we decide compat should preserve
   exact store ordering even when actionc is slightly smaller.
2. Revisit `CIRCLE.ACT` only if strict byte-for-byte compatibility matters for
   call-result comparisons; the current difference is smaller and appears
   semantically clean.
3. Decide whether old-profile Toolkit idioms should remain accepted by the
   shared semantic layer for compat comparison, or whether modern copies are the
   standing source of truth for PMG/PRINTF/REAL/SORT modern work.
4. If modern code size matters, start with `SORT.ACT`, which is currently
   76 code bytes larger than the original VM capture.
