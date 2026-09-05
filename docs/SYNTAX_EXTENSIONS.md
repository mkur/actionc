# actionc Syntax Extensions

This note tracks source syntax that is intentionally accepted by `actionc` but
is not part of strict Action! compatibility. Unless marked modern-only, these
forms are accepted in both `legacy` and `modern` profiles where the owning
backend supports them. They make
source intent explicit without relying on the original compiler's loose typing
behavior. Legacy code may still use many old implicit idioms; modernized code
should prefer these explicit forms, and the modern profile requires them for
some ambiguous routine-address cases.

## Contents

- [Compile-Time Constants](#compile-time-constants)
- [Comparison Values](#comparison-values)
- [Fixed-Length Arrays Inside Records](#fixed-length-arrays-inside-records)
- [Volatile Storage](#volatile-storage)
- [ATASCII And Screen-Code Escapes](#atascii-and-screen-code-escapes)
- [Typed Cast Expressions](#typed-cast-expressions)
- [Explicit Address Values](#explicit-address-values)
- [Plain CARD Values Are Not Typed Pointers](#plain-card-values-are-not-typed-pointers)
- [Function Pointers](#function-pointers)
- [Machine Block Label Bytes](#machine-block-label-bytes)
- [Relocatable Static Initializers](#relocatable-static-initializers)
- [MADS-Style Inline Assembler](#mads-style-inline-assembler)
- [Explicit Lexical Blocks](#explicit-lexical-blocks)
- [Compatibility Policy](#compatibility-policy)

## Fixed-Length Arrays Inside Records

Modern classic and MIR6502 on Atari support arrays stored directly inside a
record, with either ActionCart or Standalone runtime:

```action
CONST Count=100
TYPE Buffers=[INT ARRAY x(Count),y(Count)]
Buffers data=[1 2 3],copy
INT POINTER first=data.x

PROC Main()
  data.y(99)=first(1)
  copy=data
RETURN
```

Bounds must be positive compile-time constants. Storage is inline: this example
has 200 bytes for `x`, then 200 for `y`, with no member-array descriptor.
BYTE, CHAR, INT, CARD, REAL and complete supported record elements are covered.
Classic's existing restriction on pointer-valued record fields remains, including
arrays of pointers. Incomplete and recursive by-value layouts are rejected.

Direct, local, absolute, nested and pointer-based records support element access,
as do arrays of records (`rows(r).x(i)`). `SIZEOF(data.x)` is 200,
`ELEMENTS(data.x)` is 100, and layout queries do not execute their operands.
An exact element-pointer or matching array-parameter context permits field
decay; explicit address-of and pointer casts are also available. Member arrays
cannot be rebound or assigned as whole arrays.

Flat initializer lists visit fields and elements in declaration order and
zero-fill missing values. Address leaves such as `[@data.x(1)]` use the normal
relocation machinery. Static pointer initializers require known storage and
constant indexes; runtime pointer/index expressions belong in routine statements.
This extension does not add scalar non-pointer subobject-alias declarations.

Whole-record assignment copies every embedded byte, including records larger
than 255 bytes. Destination and source places are evaluated once, in that order;
self-copy and overlapping aliases preserve whole-value semantics. No runtime
bounds checks are added. Compatibility/legacy continues to reject array fields.

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
The modern profile also permits scalar comparisons and their composition in
CONST expressions; the result of each comparison is BYTE 0 or 1.

A constant has no address and allocates no storage. It works anywhere its
typed value is accepted, including array bounds, initializers, `SET`, fixed
routine addresses, loop bounds, and inline assembler operands. `CONST` is an
`actionc` extension supported by both compiler profiles and both backends; the
original Action! cartridge compiler does not recognize it.

`DEFINE` remains available for textual type aliases, directive macros, and
machine-byte macros. `CONST` does not change those expansion rules.

## Comparison Values

Modern classic and MIR6502 support `<`, `<=`, `>`, `>=`, `=`, and `#`/`<>`
as expressions producing BYTE 0 (false) or 1 (true):

```action
result=(x<y)
wide=(x>=lo AND x<hi)
PrintBE(x=y)
RETURN(x#y)
```

These values can be assigned, passed, returned, indexed with, or composed with
ordinary arithmetic/bitwise operators. Comparison operands use the existing
promotion rules; assignment to INT/CARD zero-extends the BYTE result.
In value context, AND/OR/XOR evaluate both operands and remain bitwise: for
example, `(x<y) OR 2` produces 2 or 3. This does not change the existing
conditional AND/OR behavior.

This feature is modern-only. The original cartridge does not assign numeric
values to relational expressions. Compatibility rejects value uses during
semantic analysis with `comparison values require the modern profile`;
comparisons in IF, WHILE and UNTIL conditions remain supported.

## Volatile Storage

`VOLATILE` qualifies storage whose contents can change outside the current
Action! routine, most commonly hardware and operating-system registers:

```action
VOLATILE BYTE WSYNC=$D40A,
              VCOUNT=$D40B,
              COLBAK=$D01A

VOLATILE CARD RTCLOK=$0012
VOLATILE BYTE ARRAY POKEY(16)=$D200
```

The qualifier precedes the type and applies to every entry in the declaration:

```text
VOLATILE (BYTE|CHAR|CARD|INT) [ARRAY] declaration-entry
```

Each source read performs one real memory read and each source write performs
one real memory write. actionc does not cache, combine, remove, duplicate, or
reorder those accesses. A compound assignment retains its read and write and
avoids a 6502 read/modify/write instruction when that instruction would add an
observable dummy write.

`VOLATILE` is a compiler-ordering rule; it emits no fence instruction. A
volatile `CARD` or `INT` access still consists of two byte accesses and is not
atomic.

Global and routine-local scalar and array declarations are supported. A scalar
storage alias initialized from volatile storage inherits the qualifier. The
first implementation rejects volatile constants, parameters, record fields,
and pointer declarations; volatile pointer cells and pointers to volatile data
need distinct future syntax.

`VOLATILE` is supported by compatibility, optimized classic, and MIR6502 modes.
It is an actionc extension and is not accepted by the original Action!
cartridge compiler.

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

MADS-style self-modification labels can name the first encoded operand byte:

```action
ASM
    lda patch:#0
    clc
    adc #1
    sta patch

    lda source:$ff00,y
    sta source+1
ENDASM
```

For a word operand, the label names its low byte and `label+1` names its high
byte. The instruction must have an encoded operand, so implied and accumulator
forms cannot carry such a label. Reads or writes through an inline-code label
are treated as conservative memory effects by the optimizer.

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

## Explicit Lexical Blocks

The modern profile supports nestable, line-delimited `BEGIN`/`END` blocks:

```action
PROC Main()
  BYTE value

  value=1
  BEGIN
    CARD value

    value=1000
  END

  ; BYTE value is visible again.
RETURN
```

Each explicit block creates one lexical scope. Its declarations form a prefix:
all declarations must appear before the first executable statement. A block may
shadow names from an outer block, the routine, a module/global scope, or the
resident library. Lookup after `END` resumes in the parent scope, and sibling
blocks cannot see one another's declarations.

Supported block declarations include scalar and array storage, pointers,
`VOLATILE` and absolute storage, storage aliases, native `REAL`, `CONST`,
`TYPE`, and `RECORD`. Block-local `DEFINE` is not supported because source-text
expansion requires its own scoped parser environment.

An `IF`, loop, or other control-flow body does not create a scope by itself; put
an explicit block inside it when local declarations or shadowing are wanted.
Lexical visibility also does not imply stack allocation. Block locals retain
Action!'s static routine-storage lifetime, so an address may escape the block
even though the declaration's name is no longer visible.

`BEGIN` and `END` are contextual words rather than lexer keywords. They remain
legal ordinary identifier spellings in compatibility source. A lexical block
is a modern-profile feature and is rejected by the compatibility profile with a
focused diagnostic.

See [samples/lexical-blocks.act](../samples/lexical-blocks.act) for nested
shadowing, a block-local type, a branch-local block, and address escape.

## Compatibility Policy

These extensions are accepted by `actionc`, but they are not proof that the
original Action! compiler accepted the same source. The legacy profile remains
the reference-oriented path for compatibility work and accepts more old
Action!-style implicit idioms. The modern profile uses these explicit forms to
avoid ambiguous routine-address and pointer behavior, and may also use them to
support future IR-based optimizations.
