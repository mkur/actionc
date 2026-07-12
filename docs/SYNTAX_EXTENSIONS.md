# actionc Syntax Extensions

This note tracks source syntax that is intentionally accepted by `actionc` but
is not part of strict Action! compatibility. These forms are accepted in both
`legacy` and `modern` profiles where the owning backend supports them. They make
source intent explicit without relying on the original compiler's loose typing
behavior. Legacy code may still use many old implicit idioms; modernized code
should prefer these explicit forms, and the modern profile requires them for
some ambiguous routine-address cases.

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

## Compatibility Policy

These extensions are accepted by `actionc`, but they are not proof that the
original Action! compiler accepted the same source. The legacy profile remains
the reference-oriented path for compatibility work and accepts more old
Action!-style implicit idioms. The modern profile uses these explicit forms to
avoid ambiguous routine-address and pointer behavior, and may also use them to
support future IR-based optimizations.
