# MIR6502 Machine Block Byte Stream Plan

Snapshot date: 2026-06-04.

This note defines the intended MIR6502 treatment of Action! `machine` blocks.
It is deliberately small: machine blocks are byte streams, not inline assembly
that the compiler interprets.

The goal is compatibility with existing Action! sources while avoiding a hidden
6502 assembler inside MIR6502 lowering or emission.

## Core Rule

A machine block emits bytes.

MIR6502 may evaluate compile-time constants and compile-time address expressions
inside a machine block, but it must not infer 6502 instruction meaning or
addressing modes.

```text
Allowed responsibility:
  expression -> compile-time value/address -> emitted byte(s)

Not allowed responsibility:
  tokens -> inferred 6502 instruction operand/addressing mode
```

Machine blocks can represent code or data. The compiler must not assume which.

## Accepted Item Model

Each machine block item is lowered as a byte-stream item.

Supported item shape, initially:

```text
optional byte selector: < or >
optional explicit address marker: @
atom: numeric literal | label | storage symbol | routine symbol
optional offset: + integer_constant | - integer_constant
```

Examples:

```text
$34
$0348
label
label+1
label-1
@label
@label+1
<label
>label
<@label
>@label
$0348+1
<$0348
>$0348
```

Do not support arbitrary expressions in the first implementation. In particular,
do not support nested expressions, non-constant offsets, runtime values,
dereference syntax, or expression trees that require semantic interpretation
beyond compile-time address/value evaluation.

## Emission Width Rule

After resolving the item to a compile-time value/address:

```text
if there is an explicit < selector:
    emit low byte only
else if there is an explicit > selector:
    emit high byte only
else if resolved value is in zero page ($0000..$00FF):
    emit one byte
else if resolved value is in word range ($0000..$FFFF):
    emit little-endian word: low byte, then high byte
else:
    diagnostic
```

The zero-page one-byte rule is intentional compatibility behavior. Existing
Action! machine blocks often use zero-page labels as one-byte operands.

Examples:

```text
label at $3456:
  @label     -> $56 $34
  @label+1   -> $57 $34
  <@label    -> $56
  >@label    -> $34
  label      -> warning, emits $56 $34
  <label     -> $56
  >label     -> $34

zp_label at $00E4:
  @zp_label  -> $E4
  @zp_label+1 -> $E5
  <@zp_label -> $E4
  >@zp_label -> $00
  zp_label   -> warning, emits $E4
  <zp_label  -> $E4
  >zp_label  -> $00

numeric values:
  $34        -> $34
  $0348      -> $48 $03
  $F2F8      -> $F8 $F2
  $FFFF      -> $FF $FF
  <$0348     -> $48
  >$0348     -> $03
```

The `<` and `>` selectors always force exactly one emitted byte. They override
the zero-page heuristic.

## `@label` And Explicit Address Taking

`@name` is the preferred modern spelling for compile-time address taking inside
machine blocks.

Supported meanings:

```text
@label        address of code/data label
@var          storage address of variable or array
@routine      routine entry address
@name+const   address plus constant offset
@name-const   address minus constant offset
```

If `@name` cannot be resolved to a compile-time address, emit a targeted
diagnostic. Do not fall back to guessing.

## Bare Symbol Compatibility And Warnings

For compatibility, bare symbolic names are accepted inside machine blocks as
address expressions.

However, bare symbolic names should produce a warning because the modern explicit
spelling is `@name`.

```text
label       accepted, warning, interpreted as address of label
label+1     accepted, warning, interpreted as address of label plus 1
var         accepted, warning, interpreted as storage address of var
routine     accepted, warning, interpreted as routine entry address
```

Suggested warning text:

```text
warning: bare symbol `foo` in machine block is interpreted as an address; use `@foo` to make address-taking explicit
```

Do not warn for:

```text
@label
@label+1
<label
>label
<@label
>@label
numeric literals
```

Rationale: `<` and `>` already make the byte-selection intent explicit enough
for compatibility code, while plain `label` remains ambiguous to readers.

## Operators

The `+` and `-` tokens are not valid standalone machine block items. They are
only valid as part of the narrow address expression form:

```text
atom + integer_constant
atom - integer_constant
```

So these are valid if parsed as one item/expression:

```text
@label+1
label+1
$0348+1
```

These remain invalid as independent byte-stream items:

```text
+
-
```

Suggested diagnostic:

```text
machine block operator `+` must be part of a compile-time address expression
```

## Diagnostics

Diagnostics should be targeted and should not hide the raw item.

Examples:

```text
machine block item `$12345` does not fit in 16 bits
machine block item `foo+x` is not compile-time known
machine block item `+` is not a byte-stream item; use it only inside an address expression
machine block item `@foo` cannot be resolved to a compile-time address
machine block item `foo` is ambiguous in modern profile; use `@foo` for address-taking
```

Warnings should not be fatal initially. They can later become configurable if a
strict modern profile wants to deny bare symbolic names.

Possible future options, not required for the first implementation:

```text
--allow-bare-machine-labels
--warn-bare-machine-labels
--deny-bare-machine-labels
```

## Non-Goals

Do not implement these as part of this plan:

- a 6502 assembler;
- opcode parsing;
- addressing-mode inference;
- instruction-length inference;
- automatic immediate/absolute/zero-page operand selection based on preceding
  opcode bytes;
- runtime expression evaluation;
- full Action! expression lowering inside machine blocks;
- broad relocation support;
- changing non-machine-block address semantics.

If code before an address byte is an opcode, the programmer is responsible for
emitting the right opcode byte and the right number of operand bytes.

## Implementation Outline

Implement in small steps.

### Step 1: Parse/classify byte-stream items

Add a small machine-block item classifier that recognizes:

```text
numeric literal
symbol
@symbol
<item
>item
item +/- integer constant
unsupported raw token
```

No emission behavior should change beyond better diagnostics if this can be done
separately.

Suggested commit:

```text
mir6502: classify machine block byte stream items
```

### Step 2: Resolve compile-time values and addresses

Resolve supported atoms to compile-time values:

```text
numeric literal -> numeric value
label -> address with bare-symbol warning
@label -> address without warning
var/storage symbol -> storage address
routine symbol -> routine entry address
```

Only resolve values that are known at compile time.

Suggested commit:

```text
mir6502: resolve machine block address expressions
```

### Step 3: Emit bytes by width rule

Apply the emission width rule:

```text
< -> one low byte
> -> one high byte
zero-page value -> one byte
word value -> low/high bytes
```

Add focused fixtures for:

```text
$34
$0348
$F2F8
$FFFF
@label
@label+1
label with warning
<label
>label
@zero_page_label
<@zero_page_label
>@zero_page_label
```

Suggested commit:

```text
mir6502: emit machine block address bytes
```

### Step 4: Preserve diagnostics for unsupported forms

Ensure unsupported items still produce clear diagnostics and are not silently
ignored.

Suggested commit:

```text
mir6502: diagnose unsupported machine block byte items
```

## Acceptance Criteria

The implementation is acceptable when:

- existing unsupported raw items like `$F2F8`, `$FFFF`, `$0348`, `$0349`, and
  `$034A` can be emitted as compile-time byte-stream values;
- `+` is no longer reported as a standalone unsupported item when it belongs to a
  supported `atom +/- integer` expression;
- zero-page symbols emit one byte by default;
- non-zero-page symbols emit little-endian words by default;
- `<` and `>` force exactly one byte;
- `@symbol` is accepted without warning;
- bare symbolic names are accepted with a warning;
- machine blocks are still not interpreted as 6502 assembly;
- unsupported forms fail with targeted diagnostics.

## Design Summary

```text
Machine block = byte stream.
Compile-time value/address expression = allowed.
@name = preferred explicit address taking.
Bare name = compatibility address taking with warning.
<expr / >expr = explicit byte selector.
Zero-page resolved value = one byte by default.
Non-zero-page resolved value = little-endian word by default.
No opcode interpretation.
No addressing-mode inference.
```
