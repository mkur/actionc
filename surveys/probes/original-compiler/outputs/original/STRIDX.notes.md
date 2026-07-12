# STRIDX.COM observations

Source: `surveys/probes/original-compiler/stridx.act`

Original compiler output:

- `STRIDX.COM`
- Atari binary load segment: `$3000..$3027`
- `RUNAD` segment: `$02E2..$02E3 = $3009`

Probe intent:

- Confirm that named `STRING`/`CHAR ARRAY` storage can be indexed.
- Confirm constant index `0` reads the string length byte.
- Confirm constant index `1` reads the first character byte.
- Confirm dynamic indexing into named string storage uses absolute indexed
  addressing.
- Avoid direct string-literal indexing; the earlier `"Z"(0)` probe form was
  rejected by the original compiler.

Observed layout and code shape:

- `s(0)="Z"` stores `$01 $5A` at `$3000..$3001`.
- `text(0)="AB"` stores `$02 $41 $42` at `$3002..$3004`.
- Scalar outputs begin at `$3005`.
- `len = s(0)` emits `LDA $3000`.
- `first = s(1)` emits `LDA $3001`.
- `ch = text(i)` emits `LDX i` followed by `LDA text,X`.

Current actionc comparison:

- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.

Conclusion:

- Original Action! supports indexing named string storage.
- Direct string literal indexing is not a supported expression form in the
  original compiler; use named storage or pass the literal to a routine instead.
