# ACTION! Runtime Helper Notes

Source analyzed from the runtime block containing:

- `r_Lsh`, `r_Rsh`, `r_Mul`, `r_Div`, `r_Mod`, `r_Par`
- `SET $4E4=r_Lsh`
- `SET $4E6=r_Rsh`
- `SET $4E8=r_Mul`
- `SET $4EA=r_Div`
- `SET $4EC=r_Mod`
- `SET $4EE=r_Par`

The same helper family appears in the extracted runtime/library sources under
names such as `LShift`, `RShift`, `MultI`, `DivI`, `RemI`, and `SArgs`.

## Helper Vector Table

The `SET` directives patch a small compiler/runtime helper vector table. Code
generation should treat these addresses as helper slots whose contents are
initialized by the active runtime, not as the helper routines themselves.

| Vector slot | Runtime helper | Purpose |
| --- | --- | --- |
| `$04E4` | `r_Lsh` | 16-bit logical left shift |
| `$04E6` | `r_Rsh` | 16-bit logical right shift |
| `$04E8` | `r_Mul` | signed 16-bit multiply |
| `$04EA` | `r_Div` | signed 16-bit divide |
| `$04EC` | `r_Mod` | signed 16-bit remainder |
| `$04EE` | `r_Par` | save call arguments into callee parameter frame |

The runtime source writes these slots with:

```action
SET $4E4=r_Lsh
SET $4E6=r_Rsh
SET $4E8=r_Mul
SET $4EA=r_Div
SET $4EC=r_Mod
SET $4EE=r_Par
```

This means the compiler can emit calls based on preinitialized helper entries.
When running from the cartridge, the corresponding helper implementation may be
resident elsewhere in the cartridge ROM/RAM environment. The original cartridge
probes often show those resident addresses directly, for example `JSR $A0F5`
for `SArgs`/`r_Par`.

`ARITH.COM` shows more cartridge-resident helper addresses:

| Operation | Cartridge helper | Standalone slot / known helper |
| --- | --- | --- |
| `CARD LSH` | `$B5C0` | `$04E4` / `r_Lsh` |
| `CARD RSH` | `$A0E6` | `$04E6` / `r_Rsh` |
| signed multiply | `$A000` | `$04E8` / `r_Mul` |
| signed divide | `$A090` | `$04EA` / `r_Div` |
| signed modulo | `$A0DE` | `$04EC` / `r_Mod` |

These cartridge addresses are from the original compiler environment used for
the probes. Keep them distinct from the standalone runtime vector table slots
when comparing byte-for-byte output.

## Current actionc Target Model

For now, `actionc` treats cartridge-compatible output as the primary model. In
that mode, the default helper targets are the contents of the cartridge-
initialized vector slots, so generated code calls `$A000`, `$A090`, `$A0F5`,
and the other observed cartridge helper entries directly.

`SET $04E4..$04EE=value` still mutates the compile-time helper target table,
matching Action!'s runtime-library mechanism. The standalone slot addresses
remain available for the older plain codegen path, but standalone runtime
linking is not the active target until the runtime package is compiled or
bundled by `actionc`.

## Calling Convention

Arithmetic helpers use the same value convention currently assumed by
`actionc`:

| Value | Location |
| --- | --- |
| left operand low byte | `A` |
| left operand high byte | `X` |
| right operand low byte | `$84` |
| right operand high byte | `$85` |
| result low byte | `A` |
| result high byte | `X` |

Shift helpers use:

| Value | Location |
| --- | --- |
| value low byte | `A` |
| value high byte | `X` |
| shift count | `$84` |
| result low byte | `A` |
| result high byte | `X` |

`r_Mod` calls `r_Div` and returns the remainder from `$86/$87` in `A/X`.

## Zero-Page Temporaries

Observed helper scratch usage:

| Location | Use |
| --- | --- |
| `$82/$83` | normalized left operand / quotient workspace |
| `$84/$85` | right operand, metadata pointer in `r_Par`, temporary quotient for sign fix |
| `$86/$87` | result/remainder accumulator |
| `$A0+` | call argument byte area |
| `$C0..$C2` | multiply/sign temporaries |

These helpers are not leaf-safe with respect to those zero-page locations.

## r_1

`r_1` computes two's-complement negation of the 16-bit value passed in `A/X`.

Input:

- `A`: low byte
- `X`: high byte

Output:

- `A/X`: negated value

It stores the input in `$86/$87`, subtracts each byte from zero with carry, and
returns the result in `A/X`.

## r_3

`r_3` normalizes operands for signed multiply/divide.

Input:

- left operand in `A/X`
- right operand in `$84/$85`

Output/workspace:

- absolute left operand in `$82/$83`
- absolute right operand in `$84/$85`
- result sign tracker in `$C2`
- `$87` cleared to zero for later accumulation

It negates either operand when its high byte is negative. `$C2` tracks whether
the final quotient/product should be negative.

## r_2

`r_2` is an 8-bit multiply helper used by `r_Mul` for the cross terms of a
16-bit product.

It takes one byte in `A` and one byte in `X`, computes the product contribution,
adds the relevant low byte into `$87`, and returns the current 16-bit product
accumulator in `A=$86`, `X=$87`.

In `r_Mul`, it is used for:

- left low byte * right high byte
- left high byte * right low byte

These terms are shifted by 8 in the final 16-bit product, so only their low
byte contributes to `$87`.

## r_Mul

`r_Mul` performs signed 16-bit multiplication.

Flow:

1. Calls `r_3` to normalize signed operands and set the result sign.
2. Multiplies low byte * low byte into `$86/$87`.
3. Adds cross terms through `r_2`.
4. If `$C2` says the signs differed, jumps to `r_1` to negate the result.
5. Returns product in `A/X`.

The routine returns the low 16 bits of the product.

## r_Div

`r_Div` performs signed 16-bit division.

Flow:

1. Calls `r_3` to normalize signed operands and set quotient sign.
2. Uses a shorter path when the divisor high byte is zero.
3. Uses a wider path for full 16-bit divisors.
4. Leaves quotient in `A/X`.
5. Leaves remainder in `$86/$87`.
6. Negates the quotient when the operand signs differed.

There is no obvious explicit divide-by-zero trap in this helper block.

## r_Mod

`r_Mod` is a wrapper around `r_Div`.

It calls `r_Div`, then returns:

- `A = $86`
- `X = $87`

So the modulo operation returns the remainder left by division.

## r_Lsh

`r_Lsh` performs a 16-bit logical left shift.

Input:

- value in `A/X`
- count in `$84`

If count is zero, it returns immediately. Otherwise it copies `X` to `$85` and
loops:

- `ASL A`
- `ROL $85`
- decrement count

The result returns in `A/X`.

## r_Rsh

`r_Rsh` performs a 16-bit logical right shift.

Input:

- value in `A/X`
- count in `$84`

If count is zero, it returns immediately. Otherwise it copies `X` to `$85` and
loops:

- `LSR $85`
- `ROR A`
- decrement count

The result returns in `A/X`.

This is logical right shift behavior, not arithmetic sign-extension.

## r_Par

`r_Par` is the standalone-runtime argument-frame helper corresponding to
`SArgs`.

Input:

- `A`, `X`, `Y`: first three argument bytes
- `$A3+`: later argument bytes
- inline metadata immediately after the `JSR r_Par`

Metadata format:

```text
.BYTE <frame-base-low>, <frame-base-high>, <arg-byte-count-minus-one>
```

This matches the `SARGS.COM` probe metadata shape.

Operation:

1. Saves `A`, `X`, and `Y` into `$A0`, `$A1`, and `$A2`.
2. Pops the `JSR` return address into `$84/$85`.
3. Adds 3 to the return address and pushes it back, so `RTS` resumes after the
   three metadata bytes.
4. Reads metadata through the original return address:
   - frame low byte
   - frame high byte
   - last argument byte offset
5. Copies argument bytes backwards from `$A0+offset` into `(frame),offset`.

The backwards copy is:

```text
for y = arg_byte_count_minus_one downto 0:
    frame[y] = $A0[y]
```

## Implications For actionc

- Current arithmetic helper ABI in `src/codegen.rs` matches this runtime block.
- The helper slot constants `$04E4..$04EE` are runtime vector slots populated by
  `SET`, not necessarily final routine bodies.
- The original compiler emits calls to the address currently stored in each
  helper slot. It does not emit `JSR $04E4`/`JSR $04EE` directly unless that is
  the value stored in the slot.
- Cartridge-original output may show resident helper addresses such as `$A0F5`
  and `$A090` because the cartridge environment has its own initialized helper
  entry points.
- `actionc` now models those slots as compile-time helper targets. Compatible
  output defaults to the cartridge-resident helper addresses, while plain output
  keeps the historical slot-address defaults. Numeric `SET $04E4..$04EE=value`
  directives override the target table; named `SET` values are also resolved
  when the name refers to a fixed-address routine or to a generated routine
  label in the same program.
- Compatible parameter prologues now emit `SArgs`/`r_Par` style metadata for
  argument frames of three or more bytes, while one- and two-byte frames keep the
  direct register stores.
