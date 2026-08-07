# ATASCII And Screen-Code Escapes

`actionc` accepts raw ATASCII bytes from extracted source files, but modern text tools are easier to use when unusual characters are written as ASCII escapes.

Escapes are available in string literals and character constants:

```action
BYTE cr = '\{RETURN}
BYTE invA = '\{INV:A}
BYTE screenA = '\{SCREEN:A}
BYTE raw = '\{$9B}
CHAR ARRAY text(0)="HELLO\{RETURN}\{INV:!}"
CHAR ARRAY display(0)="\{SCREEN:HELLO!}"
```

Supported forms:

```text
\{$HH}        exact ATASCII byte
\{CHAR:$HH}   verbose exact ATASCII byte
\{NAME}       named ATASCII byte
\{INV:text}   inverse-video bytes for ASCII text
\{SCREEN:text} ANTIC screen-code bytes converted from ATASCII text
```

The exact byte escape is the compatibility anchor. Named escapes are convenience aliases and can be expanded over time without changing the source format.

Current named escapes:

```text
RETURN, EOL, CR  $9B
ESC, ESCAPE      $1B
CLEAR, CLS       $7D
```

`\{INV:text}` sets the high bit of each ASCII character in `text`. For example, `\{INV:A}` emits `$C1`.

`\{SCREEN:text}` converts each ATASCII byte to the internal character code
consumed directly by ANTIC text modes. It preserves the inverse-video bit and
uses the standard Atari mapping:

```text
ATASCII $00-$1F -> screen $40-$5F
ATASCII $20-$5F -> screen $00-$3F
ATASCII $60-$7F -> unchanged
```

The same mapping applies with bit 7 set. For example, `\{SCREEN:A}` emits
`$21`, while `\{SCREEN:a}` emits `$61`. Use this form for display buffers read
directly by ANTIC. Continue using ordinary ATASCII text for `Print`, CIO, files,
and other character I/O.

See the [inline-assembler fine scroller](../samples/inline-asm-fine-scroll.act)
for a static `SCREEN`-encoded buffer used directly by an ANTIC LMS instruction.

To convert an edited text file back to raw ATASCII bytes for the original
Action! compiler or ATR sidecars:

```sh
tools/actionc-to-atascii.sh FILE.ACT FILE.ACT.atascii
```
