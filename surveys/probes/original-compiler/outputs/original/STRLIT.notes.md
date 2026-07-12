# STRLIT.COM observations

Source: `surveys/probes/original-compiler/strlit.act`

Original compiler output:

- `STRLIT.COM`
- Atari binary load segment: `$3000..$3044`
- `RUNAD` segment: `$02E2..$02E3 = $3033`

Probe intent:

- Test whether original Action! accepts string literals as array/pointer
  arguments.
- If accepted, identify where literal backing storage is emitted.

Source revision note:

- The first version also included `y0 = "Z"(0)`.
- Original Action! rejected that file with error 17, "Bad expression / illegal
  expression format."
- The current `STRLIT` probe now isolates `Take("HI")`.
- Direct string-literal indexing was moved to `stridx.act`.

Current actionc comparison:

- actionc now compiles `Take("HI")`.
- The emitted shape matches the important original behavior: a jump-over block
  protects inline length-prefixed literal storage, and the caller passes the
  address of the literal length byte.
- Exact surrounding addresses differ from the original while broader call
  prologue compatibility work remains in progress.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `x0` | `$3000` | result byte |
| `x1` | `$3001` | result byte |
| `Take.s` | `$3002..$3003` | string pointer param |
| `Take` | trampoline `$3004`, body `$3007` | direct 2-byte prologue |
| `Main` | trampoline `$3033`, body `$3036` | `RUNAD=$3033` |
| literal skip | `$3036` | `JMP $303C` |
| literal bytes | `$3039..$303B` | `$02 "HI"` |
| call site | `$303C..$3043` | passes `$3039`, then `JSR Take` |

Conclusions:

- `Take("HI")` is legal in original Action!.
- The string literal is emitted in the code segment as length-prefixed storage:
  `$02 $48 $49`.
- The compiler emits the literal inline in `Main` and jumps over it before
  executing the call site.
- The caller passes the address of the literal length byte:

```text
JMP after_literal
literal:
  .BYTE $02,$48,$49
after_literal:
LDX #$30
LDA #$39
JSR Take
```

- In the callee, `s(0)` reads the length byte and `s(1)` reads the first
  character:

```text
s(0) -> x0 = $02
s(1) -> x1 = $48 ; H
```

Questions to answer from original output:

- Confirm with another probe whether identical literals are pooled or emitted
  separately.
- Confirm whether literal storage is mutable if passed to a routine that writes
  through the `STRING` parameter.
