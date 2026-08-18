# actionc Syntax Extensions

This note tracks source syntax that is intentionally accepted by `actionc` but
is not part of strict Action! compatibility. These forms are accepted in both
`legacy` and `modern` profiles where the owning backend supports them. They make
source intent explicit without relying on the original compiler's loose typing
behavior. Legacy code may still use many old implicit idioms; modernized code
should prefer these explicit forms, and the modern profile requires them for
some ambiguous routine-address cases.

## Compile-Time Constants

`CONST` declares a typed scalar value evaluated by the compiler:

```action
CONST BYTE TOP_BLANK_ROWS=4
CONST BYTE FIRST_VISIBLE_VCOUNT=2+TOP_BLANK_ROWS
CONST CARD DISPLAY_LIST_A_BASE=$5000,
      DISPLAY_LIST_B_BASE=DISPLAY_LIST_A_BASE+$400

PROC Draw()
  CONST BYTE LAST_ROW=159
  BYTE row

  FOR row=0 TO LAST_ROW DO
    ; ...
  OD
RETURN
```

The scalar type is optional:

```text
CONST [BYTE|CHAR|CARD|INT] name=expression [, name=expression ...]
```

Without it, each entry's type is inferred using normal Action! expression
typing. With it, the declared type applies to every entry in that declaration
and has exactly the same wrapping and truncation behavior as an explicit cast.
For example, `CONST BYTE MASK=$1FF` is equivalent to
`CONST MASK=BYTE($1FF)` and produces `$FF`. Use separate declarations when
constants need different declared types.

Constants may be global or local to a routine. Names are case-insensitive and
use the ordinary local-before-global lookup order. Entries are evaluated from
left to right and may refer only to constants already visible at that point;
forward references are rejected.

Constant expressions support numeric and character literals, parentheses,
unary `+` and `-`, explicit `BYTE`, `CHAR`, `CARD`, and `INT` casts, and the
arithmetic and bitwise operators `+`, `-`, `*`, `/`, `MOD`, `LSH`, `RSH`,
`AND`, `OR`, and `XOR`. Calls, storage references, strings, addresses, and the
current-location `*` value are not constant expressions.

A constant has no address and allocates no storage. It works anywhere its
typed value is accepted, including array bounds, initializers, `SET`, fixed
routine addresses, loop bounds, and inline assembler operands. `CONST` is an
`actionc` extension supported by both compiler profiles and both backends; the
original Action! cartridge compiler does not recognize it.

`DEFINE` remains available for textual type aliases, directive macros, and
machine-byte macros. `CONST` does not change those expansion rules.

## ATASCII And Screen-Code Escapes

String literals and character constants accept textual byte escapes. In
addition to exact and named ATASCII bytes and inverse text, `\{SCREEN:text}`
converts ATASCII text to the internal screen codes consumed directly by ANTIC:

```action
BYTE eol = '\{RETURN}
BYTE inverseA = '\{INV:A}
BYTE screenA = '\{SCREEN:A}
CHAR ARRAY title(0)="\{SCREEN:ACTION!}"
```

Use screen-code escapes only for data read as a character display buffer, not
for `Print`, CIO, or files. See [ATASCII and screen-code escapes](ATASCII_ESCAPES.md)
for the exact forms and conversion table.

## Typed Cast Expressions

Use Action!-style type syntax followed by a parenthesized expression:

```action
BYTE(expr)
CARD(expr)
INT(expr)
CHAR(expr)

BYTE POINTER(expr)
CARD POINTER(expr)
CHAR POINTER(expr)
```

The cast is an explicit promise to the semantic layer and code generator. The
first implementation treats it as a type reinterpretation, not as a generated
numeric conversion.

Typical uses:

```action
Print(CHAR POINTER(menu))
PopUp(BYTE POINTER(@delcancel), 1, 4)
Strcpy(CHAR POINTER(linebuf), CHAR POINTER(@filename))
```

## Explicit Address Values

Use Action!'s existing address-of spelling for places and labels:

```action
@buffer
@delcancel
@DrawMenu
```

For routine/data-block labels, the address value should normally be paired with
a typed pointer cast at the call site:

```action
PopUp(BYTE POINTER(@delcancel), 1, 4)
```

This gives source a readable escape hatch for old Action! idioms such as using
`PROC name=*() [...]` as inline data while keeping the intended pointer type
explicit. Legacy code may still rely on more implicit forms; modernized code
should use the explicit address and pointer spelling.

## Plain CARD Values Are Not Typed Pointers

The original compiler and old Toolkit sources sometimes use `CARD` values as
raw addresses. `actionc` still accepts some of those idioms, especially in the
legacy profile, but a plain `CARD` is not a typed pointer everywhere.

For example, these forms are rejected in both profiles because `p` is only a
`CARD`:

```action
CARD p
BYTE b

p^ = 1
b = p(0)
```

Modernize these sites by declaring the intended pointer type, or by casting an
explicit address at a call boundary:

```action
BYTE POINTER p
BYTE b

p^ = 1
b = p(0)

PopUp(BYTE POINTER(@menuData), 1, 4)
```

The maintained Toolkit and TN samples use this style for old menu/data-block
patterns.

## Function Pointers

Use Action-like routine syntax with `POINTER`:

```action
PROC POINTER handler
BYTE FUNC POINTER keyReader
CARD FUNC POINTER nextItem
```

Assign routine addresses explicitly:

```action
handler = @DrawMenu
keyReader = @Key
```

Call through the pointer with normal call syntax:

```action
handler()
b = keyReader()
```

The first implementation models only the routine kind and return type;
parameterized function-pointer signatures can be added later if needed. Direct
assignment to routine names is rejected in the modern profile:

```action
DrawMenu = OtherProc      ; rejected
handler = @OtherProc     ; accepted
```

## Machine Block Label Bytes

Inside machine blocks, `<name` and `>name` emit the low and high byte of a
symbol address:

```action
PROC Target()
RETURN

PROC JumpVector=*()
[ <Target >Target ]
```

This keeps full label operands unchanged (`[$20 Target]` still means a two-byte
absolute operand) while making byte selection explicit for tables and
self-contained machine code fragments.

## Relocatable Static Initializers

Initializer lists can contain addresses that are fixed after the compiler lays
out storage and routines. Use `<` or `>` in a byte array and `@` in a word
array:

```action
BYTE ARRAY dlist(3)=[$41 <dlist >dlist]

PROC Draw()
RETURN

CARD ARRAY handlers(1)=[@Draw]
```

The compiler emits the low byte, high byte, or complete little-endian word at
the initializer position. Constant addends are supported, for example
`<dlist+4`, and forward references are allowed. An array reference denotes its
element backing address, including for arrays represented internally by a
descriptor.

## MADS-Style Inline Assembler

Use `ASM` and `ENDASM` on their own lines to embed official NMOS 6502
instructions:

```action
BYTE ARRAY pixels(256)
CARD ptr=$A0

PROC Draw()
ASM
    lda #<pixels
    sta ptr
    lda #>pixels
    sta ptr+1

    ldy #0
loop:
    lda (ptr),y
    sta pixels,y
    iny
    bne loop
ENDASM
RETURN
```

The assembler is built into `actionc`; MADS is not needed at compile time.
The supported MADS-compatible subset includes:

- all official NMOS 6502 instructions and addressing modes;
- hexadecimal `$`, binary `%`, decimal, and ATASCII character constants;
- named labels (with an optional colon) and anonymous `@`, `@+`, `@-` labels;
- block-local `name = expression` and `name EQU expression` constants;
- `.z`/`.b` and `.a`/`.w` address-size suffixes;
- checked arithmetic, shift, and bitwise expressions;
- `;`, `//`, and `/* ... */` comments;
- direct references to visible Action! globals, locals, parameters, arrays,
  constants, and routines.

Address selection is deterministic. Numeric addresses below `$100` use a
zero-page encoding where one exists. Ordinary allocated Action! objects use an
absolute encoding. `.z`, `(pointer),Y`, and `(pointer,X)` require the referenced
pointer cell to be provably in zero page, for example:

```action
CARD ptr=$A0
```

Assembler-local names shadow Action! names. Prefix a name with `:` to request
the Action! object explicitly:

```action
BYTE value

PROC Example()
ASM
value:
    inc :value
    bne value
ENDASM
RETURN
```

Low and high address bytes are written `#<name` and `#>name`. A numeric
Action! `DEFINE` or a visible `CONST` can be used directly as an operand; the
compiler diagnoses a byte operand that does not fit instead of truncating it.
A direct `JSR` to an Action! routine is relocated through the normal routine
identity; storage references likewise retain a stable compiler storage
identity rather than a source-name string.

Analyzed blocks participate in MIR6502 memory-effect and machine-register
liveness analysis. Fall-through and return paths must preserve stack depth.
Operations whose effects are deliberately outside that contract can use
`ASM OPAQUE`, which still receives syntax, opcode, relocation, and zero-page
validation but acts as a full compiler barrier:

```action
ASM OPAQUE
    ; deliberately non-standard machine-state manipulation
ENDASM
```

Macros, conditional assembly, repetition, include/output directives, data
directives, illegal opcodes, and 65C02/65816 instructions are not part of this
initial subset. Keep static data in Action! arrays.

## Compatibility Policy

These extensions are accepted by `actionc`, but they are not proof that the
original Action! compiler accepted the same source. The legacy profile remains
the reference-oriented path for compatibility work and accepts more old
Action!-style implicit idioms. The modern profile uses these explicit forms to
avoid ambiguous routine-address and pointer behavior, and may also use them to
support future IR-based optimizations.
