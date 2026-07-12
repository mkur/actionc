# MIR6502 Machine Block Local Symbol Implementation Note

Snapshot date: 2026-06-04.

This note narrows the machine-block byte-stream plan to the concrete `LIB.ACT`
case seen in routine `_StrNam`. It should be read together with
`docs/archive/implementation-plans/mir6502/MIR6502_MACHINE_BLOCK_BYTE_STREAM_PLAN.md`.

The key point is simple: this is still byte-stream lowering. MIR6502 should not
interpret the machine block as 6502 assembly, but it must resolve compile-time
symbols and address expressions that Action! machine blocks commonly use.

## Triggering Example

From `LIB.ACT`:

```action
PROC _StrNam()
 CHAR ARRAY fnam(40) CARD r=$86
[$85$86$84$87$A0$00$B1$86$85$88$C8$B1$86$99 fnam+$FFFF $C4$88$D0$F6$C8$A9$9B$99 fnam+$FFFF]
 r=fnam
[$9D$344$A5 r+1 $9D$345 $60]
```

Observed MIR6502 diagnostics included unsupported raw machine block items such
as:

```text
+
$FFFF
$344
$345
```

and an unresolved `fnam`-style symbol case.

## Intended Interpretation

Do not parse this as assembly. Parse it as a sequence of byte-stream items.

These items are known at compile time and should resolve:

```text
fnam        local array storage address
fnam+$FFFF local array storage address minus 1, using 16-bit wrapping
r           fixed-address local/card symbol at $86
r+1         fixed-address local/card symbol at $87
$344        absolute word value $0344
$345        absolute word value $0345
```

The likely source-level intent is equivalent to byte-stream operands for shapes
such as:

```text
STA fnam-1,Y
LDA r+1
STA $0345,X
```

but MIR6502 must not infer or validate those instructions. It should only emit
the bytes requested by the byte-stream items.

## Resolution Rules Needed For This Case

Machine-block expression resolution must be able to resolve symbols in the
current routine scope, not only globals.

Supported symbols for this slice:

```text
local array storage symbol
local scalar storage symbol
local fixed-address alias
parameter storage symbol, if it already has a compile-time address
routine/global/static symbol already supported elsewhere
```

For `_StrNam`, the minimum required support is:

```text
fnam -> address of local CHAR ARRAY fnam
r    -> fixed zero-page address $0086
```

If a local symbol does not have a compile-time address in the current storage
model, do not guess. Emit a targeted diagnostic explaining that the symbol cannot
be used as a machine-block byte-stream address.

## 16-Bit Wrapping Offsets

Address expression offsets in machine blocks should use 16-bit wrapping
arithmetic for compatibility.

```text
resolved = (base + offset) & $FFFF
```

This makes the common Action!/6502 idiom work:

```text
fnam+$FFFF == fnam-1
```

Do not reject `$FFFF` as an overflowing positive offset when it appears as an
offset in `atom + constant`. In this context it is a compile-time word-sized
offset.

Still reject values that cannot be resolved to a compile-time 16-bit value.

## Emission Width Rule For The Example

Apply the byte-stream width rule from the main plan:

```text
<expr or >expr -> exactly one selected byte
resolved zero-page value -> one byte
resolved non-zero-page word -> little-endian low/high bytes
```

Expected emissions for the `_StrNam` shapes:

```text
fnam+$FFFF -> low(fnam-1), high(fnam-1)   ; unless fnam-1 is zero-page
r+1        -> $87                         ; because r=$86 is zero-page
$344       -> $44 $03
$345       -> $45 $03
$FFFF      -> $FF $FF when used as a standalone item
```

If `fnam` is not zero-page, `fnam+$FFFF` emits two bytes. If a future storage
model places `fnam` in zero page, the normal zero-page one-byte rule applies,
unless the source uses `<` or `>` explicitly.

## Bare Symbol Warnings

Bare symbols remain accepted for compatibility but should warn in modern/MIR6502
paths:

```text
fnam+$FFFF -> warning: bare symbol `fnam` interpreted as address; use `@fnam`
r+1        -> warning: bare symbol `r` interpreted as address; use `@r`
```

Modern spelling would be:

```action
@fnam+$FFFF
@r+1
```

or explicit selected bytes where the source wants exactly one byte:

```action
<@fnam+$FFFF
>@fnam+$FFFF
<@r+1
```

Do not warn for `<fnam`, `>fnam`, `<@fnam`, or `>@fnam`; the byte selector is
explicit enough for compatibility code.

## Implementation Steps

### Step 1: Characterize Current Machine-Block Items

Add a focused fixture based on `_StrNam` or a smaller equivalent:

```action
PROC Main()
  CHAR ARRAY fnam(40)
  CARD r=$86
  [$99 fnam+$FFFF]
  r=fnam
  [$A5 r+1 $9D$345]
RETURN
```

If the current parser cannot represent that exact source as a fixture, use the
smallest source that reaches the same raw machine-block item stream.

Acceptance criteria:

- The fixture currently reproduces unsupported item diagnostics.
- The diagnostic includes the routine/block and raw item text.

### Step 2: Parse `atom +/- constant` As One Byte-Stream Expression

The current diagnostics show `+` and `$FFFF` as independent unsupported items.
That suggests the raw stream is being handled token-by-token.

Add a narrow machine-block item parser/collector that recognizes:

```text
atom + integer_constant
atom - integer_constant
```

Do not add general expression parsing.

Acceptance criteria:

- `fnam+$FFFF` is treated as one expression item, not as `fnam`, `+`, `$FFFF`
  independent items.
- Standalone `+` still gets a targeted diagnostic.

Suggested commit:

```text
mir6502: parse simple machine block address offsets
```

### Step 3: Resolve Local Storage Symbols

Extend machine-block expression resolution to query the current routine-local
symbol/storage context.

Support at least:

```text
local array storage address
local fixed-address scalar/card address
```

For `_StrNam`:

```text
fnam -> local array address
r    -> $0086
```

Acceptance criteria:

- `fnam` resolves when used inside `_StrNam` machine blocks.
- `r+1` resolves to `$0087`.
- Unresolvable locals produce a precise diagnostic and do not silently become 0.

Suggested commit:

```text
mir6502: resolve local symbols in machine blocks
```

### Step 4: Apply 16-Bit Offset Arithmetic

Evaluate `atom +/- constant` with 16-bit wrapping.

Acceptance criteria:

- `fnam+$FFFF` resolves to `fnam-1` modulo 16 bits.
- `$344`, `$345`, `$FFFF` standalone numeric values still resolve as compile-time
  word values.
- Values outside the supported literal range produce diagnostics.

Suggested commit:

```text
mir6502: evaluate machine block offsets with word wrapping
```

### Step 5: Emit Bytes By Width Rule

Use the main byte-stream width rule:

```text
zero-page resolved value -> one byte
non-zero-page word -> low/high
< or > selector -> exactly one byte
```

Acceptance criteria for the `_StrNam`-style fixture:

- `$344` emits `$44 $03`.
- `$345` emits `$45 $03`.
- `r+1` emits `$87`, not `$87 $00`.
- `fnam+$FFFF` emits the low/high bytes of `fnam-1` if non-zero-page.
- The machine block is not interpreted as assembly.

Suggested commit:

```text
mir6502: emit local machine block address expressions
```

## Non-Goals

Do not implement these in this slice:

- char constants in machine blocks;
- inline strings in machine blocks;
- full expression grammar;
- runtime-dependent expressions;
- dereference expressions;
- opcode/addressing-mode inference;
- branch target interpretation;
- relocation records;
- automatic conversion of byte-stream operands based on previous opcode byte.

Char constants and inline strings may be useful later, but they are deliberately
held out of this note.

## Stop Conditions

Stop and add a diagnostic instead of guessing if:

- a symbol has no compile-time address;
- a local object has no stable storage location at machine-block emission time;
- an expression requires runtime values;
- the implementation needs to inspect previous opcode bytes;
- the implementation would change non-machine-block semantics;
- a warning or diagnostic would be hidden to make the fixture pass.

## Summary

For `_StrNam`, the correct implementation target is:

```text
machine block = byte stream
fnam = local compile-time storage address
r = fixed zero-page local address $86
fnam+$FFFF = fnam-1 with 16-bit wrapping
r+1 = $87, emitted as one byte because it is zero-page
$344/$345 = little-endian word bytes
bare symbols = accepted with warning
@symbols = preferred, no warning
no assembly interpretation
```
