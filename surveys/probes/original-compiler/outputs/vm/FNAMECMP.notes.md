# FNAMECMP Probe Notes

Probe: `fnamecmp.act`

Purpose:

- Isolate the TN `Fnamecmp` pointer-dereference control-flow shape.
- Exercise `CHAR POINTER` parameters, `s^` / `t^` comparisons, nested
  `IF`/`ELSEIF`, and `WHILE s^=t^` pointer scans.

Current comparison:

| Compiler | File size | Code segment | RUNAD |
| --- | ---: | --- | --- |
| Original Action! | 290 | `$3000-$3115` | `$30ED` |
| `actionc` | 287 | `$3000-$3112` | `$30EA` |

Important finding:

- Pointer-dereference equality and ordering need real indirect-indexed RHS
  opcodes. Original Action! uses `EOR (zp),Y` for `s^=t^` and `CMP (zp),Y`
  for ordered dereference comparisons.
- `actionc` now emits those addressing modes instead of silently dropping the
  RHS dereference.
- When both operands are pointer dereferences, prepare the RHS pointer in a
  separate zero-page pair before loading the LHS byte. Preparing the RHS after
  `LDA (lhs),Y` clobbers `A` and compares the RHS byte with itself.

Remaining divergence:

- `actionc` remains a few bytes smaller due existing known-register reuse and
  branch/layout choices. Keep this probe as an accepted semantic guard rather
  than forcing byte-identical output.
