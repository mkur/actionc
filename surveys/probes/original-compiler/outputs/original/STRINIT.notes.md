# STRINIT.COM observations

Source: `surveys/probes/original-compiler/strinit.act`

Original compiler output:

- `STRINIT.COM`
- Atari binary load segment: `$3000..$305E`
- `RUNAD` segment: `$02E2..$02E3 = $301E`

Probe intent:

- Confirm global `STRING` / `CHAR ARRAY` string initializer storage.
- Confirm whether `(0)` with a string initializer auto-sizes storage.
- Confirm empty-string storage.
- Confirm doubled quote handling in string literals.
- Confirm whether indexed reads see raw character bytes, length-prefix bytes,
  terminators, or another layout.

Current actionc comparison:

- Original-first probe for now.
- actionc parses/analyzes the source, but codegen does not yet support string
  / named `CHAR ARRAY` initializer layout and indexed string reads.
- No actionc hex/listing was generated.

Observed layout:

| Symbol | Address / range | Stored bytes |
| --- | --- | --- |
| `empty` | `$3000` | `$00` |
| `one` | `$3001..$3002` | `$01 $41` |
| `hello` | `$3003..$3008` | `$05 "HELLO"` |
| `quoted` | `$3009..$300C` | `$03 "A\"B"` |
| `fixed` | `$300D..$3013` | `$06 "ATARI!"` |
| `e0..f5` | `$3014..$301D` | result bytes |
| `Main` | trampoline `$301E`, body `$3021` | `RUNAD=$301E` |

Conclusions:

- String initializers store a length byte at element 0, followed by character
  data starting at element 1.
- `STRING` is a `CHAR ARRAY` alias in storage and indexing behavior.
- `(0)` with a string initializer auto-sizes to length byte plus string bytes.
- `CHAR ARRAY fixed(6)="ATARI!"` also stores a length byte plus six characters,
  so it occupies seven bytes in this probe.
- Empty string storage is a single `$00` length byte.
- Doubled quotes inside a string become one quote character: `A""B` stores
  length `$03`, then `$41 $22 $42`.

Observed reads:

| Assignment | Loaded address | Loaded value |
| --- | --- | --- |
| `e0 = empty(0)` | `$3000` | `$00` |
| `o0 = one(0)` | `$3001` | `$01` |
| `h0 = hello(0)` | `$3003` | `$05` |
| `h1 = hello(1)` | `$3004` | `$48` / `H` |
| `h4 = hello(4)` | `$3007` | `$4C` / `L` |
| `q0 = quoted(0)` | `$3009` | `$03` |
| `q1 = quoted(1)` | `$300A` | `$41` / `A` |
| `q2 = quoted(2)` | `$300B` | `$22` / `"` |
| `f0 = fixed(0)` | `$300D` | `$06` |
| `f5 = fixed(5)` | `$3012` | `$49` / `I` |

Questions to answer from original output:

- Confirm with `STRMUT.COM` whether the length byte at index 0 is mutable and
  whether normal writes to character indexes mutate in-place.
- Confirm with `STRPASS.COM` that string parameters pass the base address of
  this length-prefixed storage.
