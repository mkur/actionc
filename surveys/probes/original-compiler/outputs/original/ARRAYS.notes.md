# ARRAYS.COM observations

Source: `surveys/probes/original-compiler/arrays.act`

Original load-file layout:

- Code/data segment: `$3000..$30D6`
- RUNAD segment: `$02E2..$02E3 = $3012`

Storage layout inferred from generated code:

- `ba`: byte array storage at `$3000..$3007`
- `ca`: card array descriptor at `$3008..$300B`
  - `$3008..$3009`: data pointer, initialized to `$30D7`
  - `$300A..$300B`: byte size, initialized to `$0010`
- `i`: `$300C`
- `bx`: `$300D`
- `ci`: `$300E..$300F`
- `cw`: `$3010..$3011`
- `Main` trampoline/RUNAD: `$3012`
- `Main` body: `$3015`

Byte array lowering:

- Constant byte indexes are direct absolute accesses:
  - `ba(0) = $11` -> `STA $3000`
  - `ba(3) = $33` -> `STA $3003`
  - `bx = ba(3)` -> `LDA $3003`
- Dynamic byte indexes use absolute indexed addressing with `X`:
  - `LDX $300C`
  - `STA $3000,X`
  - `LDA $3000,X`

Card array lowering:

- `CARD ARRAY ca(8)` is not emitted as inline card storage in the saved segment.
- The compiler emits a descriptor and initializes the pointer to the first byte after the code segment (`$30D7` here).
- Constant and dynamic card indexes compute `ca + index * 2` into zero-page `$AE/$AF`, then use `($AE),Y`.
- Constant card stores use high-byte-first order through `Y=1`, then low byte after `DEY`.
- Card loads store high byte first into `cw+1`, then low byte into `cw`.

Current actionc comparison:

- actionc now emits `CARD ARRAY ca(8)` as an Action!-style descriptor plus post-code backing storage.
- actionc now uses `$AE/$AF` for compatible array address calculations.
- actionc now uses absolute indexed `,X` forms for compatible dynamic byte array
  access, including original-style value-before-index ordering for dynamic
  byte-array stores.
- actionc now emits the original card-array scaled-index address shape for
  constant and direct scalar indexes, and preserves `Y` across the `cw=ca(2)`
  load into the following `ci=1` word constant store.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.

Likely compatibility work:

- Probe broader array-index expression forms to decide how far to extend this
  original scaled-index shape beyond constants and direct scalar indexes.
