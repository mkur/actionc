# Stress Comparison Notes

Generated with:

```sh
surveys/stress/compare-original.sh all
```

The runner compiles each stress source with the original Action! cartridge via
`action-compiler-vm`, writes the original load file under `outputs/vm/`, then
compiles the same source with `actionc` under `outputs/actionc/`.

## Current results

| Stress source | Original Action! | actionc | Status |
| --- | ---: | ---: | --- |
| `arrays.act` | 1167 bytes | 1154 bytes | both compile, output differs; actionc is smaller |
| `pointers.act` | 900 bytes | 884 bytes | both compile, output differs; actionc is smaller |
| `strings.act` | 546 bytes | 546 bytes | both compile, same size, output differs |
| `records.act` | 539 bytes | 539 bytes | exact match |
| `zero_page.act` | 332 bytes | 256 bytes | both compile, output differs |
| `zero_page_scalars.act` | 98 bytes | 98 bytes | exact match |
| `arithmetic_control.act` | 674 bytes | 672 bytes | both compile, output differs; actionc is smaller |
| `calls.act` | 549 bytes | 546 bytes | both compile, output differs; actionc is smaller |

## Interpretation

The stress suite is now mostly original-compatible. `arithmetic_control.act`
and `calls.act` still test staged call results, but avoid nested call arguments
and direct call-return expressions because the cartridge compiler reports error
11 for those forms. `zero_page.act` remains a semantic-policy test rather than
a clean original code-shape comparison; `zero_page_scalars.act` covers the
original-compatible scalar zero-page subset.

## Segment shape

All comparable stress outputs use the same high-level Atari load-file shape:
one `$3000` code/data segment followed by a two-byte `RUNAD` segment at
`$02E2-$02E3`. The differences are therefore inside the generated storage/code
body, not in the load format itself.

Current first segment ranges:

| Stress source | Original Action! | actionc |
| --- | --- | --- |
| `arrays.act` | `$3000-$3482` | `$3000-$3475` |
| `pointers.act` | `$3000-$3377` | `$3000-$3367` |
| `strings.act` | `$3000-$3215` | `$3000-$3215` |
| `records.act` | `$3000-$320E` | `$3000-$320E` |
| `zero_page.act` | `$3000-$313F` | `$3000-$30F3` |
| `zero_page_scalars.act` | `$3000-$3055` | `$3000-$3055` |
| `arithmetic_control.act` | `$3000-$3295` | `$3000-$3293` |
| `calls.act` | `$3000-$3218` | `$3000-$3215` |

## One-by-one difference analysis

### `arrays.act`

Earlier routine-size breakdown, before the final narrowing work:

| Routine | Original Action! | actionc | Delta |
| --- | ---: | ---: | ---: |
| `FillBytes` | 64 bytes | 62 bytes | -2 |
| `MixArrays` | 268 bytes | 310 bytes | +42 |
| `LocalWork` | 354 bytes | 348 bytes | -6 |
| `Main` | 347 bytes | 351 bytes | +4 |

The current total file result is now 1154 bytes for `actionc` versus 1167 bytes
for original Action!, so the old size gap is closed. The remaining differences
are code-shape differences rather than gross over-generation.

The original compiler is generally more direct for indexed array stores when it
can keep the destination address live in `$AE/$AF` and use `$AC/$AD` only for
the source. `actionc` often pushes the computed destination address, evaluates
the source into temporaries, then restores the destination. This is safe but
larger.

Historical examples that drove the fixes:

- `FillBytes`: original reloads `Y=#0` before `STA ($AE),Y`; `actionc` proves
  `Y` is already zero after the loop compare path and omits it. This is a small
  case where `actionc` is tighter than original.
- `words(1) = words(0) + idx`: original computes the destination once and keeps
  it in `$AE/$AF`; `actionc` now uses `$AC/$AD` for the source and avoids the
  stack-preserved temporary path for simple scalar addends.
- `words(NextIndex(0)) = w + $20`: both stage the call result; `actionc` now
  scales the byte function result directly instead of widening it through a
  temporary word index.
- `IF nums(idx) < 0`: original loads the indexed `INT`, subtracts zero, and
  branches on `BMI`. `actionc` now branches from the indexed value directly
  instead of materializing the word first.

Follow-up fixes:

- `absolute,X` byte-array assignments now preserve `X` only when the RHS can
  genuinely clobber the target index. Same-index inline byte copies such as
  `lb(n) = gb(n)` keep the compact direct indexed shape.
- Materialized signed `INT ARRAY` values keep their signedness, so
  `nums(idx) < 0` becomes a direct high-byte sign branch instead of an
  unsigned-looking generic compare.
- Dynamic two-byte array copies and simple `+ constant` stores can compute the
  destination in `$AE/$AF` and the source in `$AC/$AD`, avoiding stack
  preservation in the common descriptor/parameter array path.
- Inline byte-array references with a byte function-call index can use the
  call result directly as `X`.
- Indexed two-byte `+ scalar` stores use separate source/target pointers.
- Descriptor/parameter array indexes supplied by byte function calls can scale
  the return byte directly.

Together these changes reduced `arrays.act` from 1307 bytes to 1154 bytes.

Most actionable remaining buckets:

- `li(n) = Neg(li(0))` still needs destination preservation across an actual
  function call; this is safe and expected unless we add a more specialized
  call-result store shape.
- The remaining differences in `arrays.act` are now mostly places where
  `actionc` is slightly tighter than the original (`Y` reuse, direct low-byte
  stores, and direct sign tests) mixed with ordinary call setup/order
  differences. There is no longer a large array-index machinery gap in this
  stress file.

### `pointers.act`

The current result is 884 bytes for `actionc` versus 900 bytes for original
Action!. The large historical gap came from pointer-index address
formation. Original Action! often computes byte-pointer offsets directly as
`base + index`, while `actionc` widened the index through `$C0/$C1` even when
the index was a byte.

Important semantic-looking gap:

- In `StoreThrough`, `q(1) = q^ + 1` is compiled by original as:
  compute destination `q + 2` in `$AE/$AF`, load source `q^` through `$AC/$AD`,
  then store the incremented value through the preserved destination.
- Fixed in `actionc`: the destination pointer is now pushed before RHS
  evaluation and restored before the final store.
- Fixed in `actionc`: byte self-updates through a pointer, such as
  `p^ = p^ + 1`, avoid the general stack-preserved temporary path. The emitted
  shape now mirrors the original's separate destination/source pointer slots:
  target in `$AE/$AF`, source in `$AC/$AD`, then `LDA ($AC),Y` / `ADC #imm` /
  `STA ($AE),Y`.

Other differences:

- Fixed in `actionc`: byte scalar pointer indexes now form the effective
  address directly. Byte pointers use `base + index`; word/card/int pointers
  scale the byte index with the compact `ASL/PHP/ROL/PLP` carry-preserving
  shape before adding the base pointer.
- Constant `CARD POINTER` index `1` can be emitted as `base + 2`; original often
  still uses the generic `ASL/PHP/ROL/PLP` scaling shape.
- Pointer equality `bp^ = bq^` is structurally similar: both compare byte
  dereferences with `EOR`, then store a byte boolean.

Correctness note:

- `Y` state is now cleared when binding ordinary codegen labels unless an
  explicit label hint says otherwise. This prevents an `ELSE` branch from
  inheriting `Y=#1` from the preceding true branch and using `DEY/STY` for a
  false boolean store. Internal peephole labels that genuinely preserve `Y`
  still attach an explicit hint.

The remaining size difference is now mostly `actionc` being tighter than the
original in constant pointer indexes and indexed word copies. That looks like
acceptable modern codegen rather than a compatibility blocker.

### `strings.act`

The current result is same-size but not byte-identical: 546 bytes for both
`actionc` and original Action!. The largest historical differences were
safe-but-larger destination preservation and generic comparison paths:

- Fixed in `actionc`: `CopyString` tests `src(0) >= n` directly instead of
  materializing both sides into temporary zero-page slots.
- Fixed in `actionc`: byte indirect copies such as `dst(n) = src(n)` now use
  separate destination/source pointers (`$AE/$AF` and `$AC/$AD`) instead of
  pushing the destination pointer around RHS evaluation.
- Fixed in `actionc`: byte bitwise compound assignments such as
  `pad(n) ==! $20` update in place instead of lowering through a temporary
  synthetic assignment.
- Fixed in `actionc`: same-index inline byte array copies such as
  `dst(i) = src(i)` load `X` once.
- Fixed in `actionc`: call RHS assignments into an indexed byte array preserve
  a scalar target index with `LDA index/PHA` instead of `LDX/TXA/PHA`.
- Fixed in `actionc`: byte constant `1` stores can now walk from a tracked
  `Y=#0` with `INY/STY`, not only from the straight-line constant-store tracker.
  This matches the original `limit=s(0); n=1` shape in `Mix`.

Important semantic-looking gap:

- In `LocalStrings`, `local(n) = At(title, seed)` should reload `n` after the
  call before storing into `local(n)`. Original does this. `actionc` sets up the
  call using `X` and then stores through `local,X` without clearly restoring
  `X=n`; the call setup appears to leave `X` holding the high byte of the string
  argument instead.
- The same pattern appears in `Main` for `mirror(i) = At("HI", i)`: original
  reloads the mirror index after `At`, while `actionc` appears to rely on `X`
  surviving the call setup.

Fixed in `actionc`: `absolute,X` assignment targets now save `X`, materialize
the RHS into zero page, restore `X`, and only then perform the indexed store
when the RHS can clobber the target index. Same-index inline byte-array copies
remain direct, matching the original compiler's tighter shape.

The remaining byte differences are offsetting local code-shape choices, not a
size gap. The main drift is still around whether indexed call-result
assignments reload `X` from the scalar index after the call, as original does,
or preserve the index with stack traffic. Other places in `actionc` are tighter
than original, so the total size now balances out.

### `records.act`

The current record stress source is now original-compatible. The first version
hit several cartridge limitations:

- Record pointer parameters mixed with additional arguments can trigger error
  11 in broader routines, even when smaller probes work.
- Passing a record pointer variable as a record-pointer argument is less
  reliable than passing the record value itself.
- Assigning `@record` to a record pointer variable is not accepted by the
  original compiler in this stress shape; assigning the record value is the
  compatible spelling.
- Field/parameter name collisions such as `n.tag = tag` are easy to confuse in
  the original compiler, so the stress source now avoids them.

Current result: exact byte match, 539 bytes for both original Action! and
`actionc`. Both emit the same two Atari load-file segments: a main
`$3000-$320E` segment and a `RUNAD=$31C8` segment at `$02E2-$02E3`.

Fixed in `actionc` while making this comparable:

- Assigning a record value to a record pointer now stores the record address,
  matching original Action!'s `nextp = tail` behavior.
- Byte field copies between two record pointers, such as
  `nextp.flags = left.tag`, use separate source and target pointers instead of
  stack-preserving the destination pointer.
- Byte arithmetic over two pointer-backed record fields, such as
  `n.tag + n.flags`, can use separate pointers directly instead of first
  materializing both fields into temporaries.
- Word arithmetic like `total = total + cur.size` now adds directly from the
  pointer-backed field into the destination word instead of staging through a
  temporary.
- Field expressions now report their real byte width to condition generation,
  so `cur.next = 0` tests both bytes with `LDA ($AE),Y` / `INY` /
  `ORA ($AE),Y` instead of accidentally checking only the low byte.
- Two-byte ordered compares against constants, such as `n.size > $0100`, reuse
  the pointer-backed field address across the low-byte `CMP` and high-byte
  `SBC`, matching the original code shape.

There is no remaining byte-level gap for the current original-compatible
record stress source.

### `arithmetic_control.act`

The current source is original-compatible after staging the nested
`ClampByte(ClampByte(...))` return path through a local byte. The original
compiler emits 674 bytes and `actionc` emits 672 bytes.

Fixed in `actionc`: in-place constant shifts on absolute byte/card variables
now use direct memory shifts. This matches the cartridge shape for
`r = r RSH 1` in `MixWord`, where original emits `LSR high` / `ROR low`
instead of calling the `RSH` helper.

The file still differs byte-for-byte, but the old size gap is closed. The
remaining differences are now mixed code-shape choices in control flow, helper
setup, and boolean stores rather than a single obvious over-generation bucket.

### `calls.act`

The current source is original-compatible after staging call arguments through
locals before `Take3` and `TakeMixed`. The original compiler emits 549 bytes
and `actionc` emits 546 bytes.

Fixed in `actionc`: byte-shaped shifts now stay byte-shaped even when the
destination is wider. The cartridge output for
`CARD FUNC Pair(BYTE lo,hi) RETURN(lo+(hi LSH 8))` shows `hi LSH 8` folded as
a byte-width shift, with the card return high byte zero-filled; `actionc`
previously routed that through the runtime `LSH` helper.

Fixed in `actionc`: arithmetic over folded-zero subexpressions, such as
`lo + (hi LSH 8)` after the byte-width shift fold, no longer materializes the
zero in a temporary before adding it.

Fixed in `actionc`: when a complex argument forces call-argument staging,
simple one-byte register arguments can be deferred and loaded directly into
`A`/`X`/`Y` after the complex argument has been staged. This matches the
original shape in `Pair(p(i), i)` more closely and removes unnecessary
`STA $A1` / `LDX $A1` traffic.

The gap is now a useful call-frame comparison rather than an original parser
limit. Likely buckets to inspect next:

- SArgs prologue and optional-argument setup.
- Mixed byte/card argument evaluation order.
- Pointer/array argument passing into `TakePointers`.
- Whether staged local call results can be passed without extra reloads.

### `zero_page.act`

This file is not an original-compatible code-shape comparison yet.

Original Action! treats declarations like `BYTE POINTER zp_p=$E4` as a pointer
variable in normal object storage initialized to pointer value `$00E4`. The VM
output stores and uses `zp_p` at `$3000/$3001` and `zp_q` at `$3002/$3003`.

`actionc` currently treats `BYTE POINTER zp_p=$E4` as a pointer variable
physically located at zero-page `$E4/$E5`, and emits direct `(zp),Y` operations
through `$E4` and `$E6`. That is useful for the zero-page ABI extension we want,
but it is not matching the original compiler's interpretation of this syntax.

So the shorter `actionc` output here is mostly explained by a semantic
difference in absolute placement of pointer variables, not better peephole
codegen. For original-compatible probing, use scalar zero-page declarations or
create a separate source spelling for pointer storage once we confirm the
official syntax.

### `zero_page_scalars.act`

This file isolates the original-compatible subset: scalar zero-page aliases
such as `BYTE zp_b=$E0` and `CARD zp_sum=$E4`, without pointer declarations
initialized to zero-page addresses.

Current result: exact byte match, 98 bytes for both original Action! and
`actionc`. Both emit a main `$3000-$3055` segment and a `RUNAD=$304B` segment
at `$02E2-$02E3`.

Fixed in `actionc`: two-byte constant stores to compatible zero-page slots can
reuse a known `Y` value. This matches the original `zp_sum=0` shape:
`LDY #0`, then `STY $E5` / `STY $E4`, instead of two separate
`LDA #0` / `STA zp` pairs.
