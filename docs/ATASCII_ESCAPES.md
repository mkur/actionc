# ATASCII Escapes

`actionc` accepts raw ATASCII bytes from extracted source files, but modern text tools are easier to use when unusual characters are written as ASCII escapes.

Escapes are available in string literals and character constants:

```action
BYTE cr = '\{RETURN}
BYTE invA = '\{INV:A}
BYTE raw = '\{$9B}
CHAR ARRAY text(0)="HELLO\{RETURN}\{INV:!}"
```

Supported forms:

```text
\{$HH}        exact ATASCII byte
\{CHAR:$HH}   verbose exact ATASCII byte
\{NAME}       named ATASCII byte
\{INV:text}   inverse-video bytes for ASCII text
```

The exact byte escape is the compatibility anchor. Named escapes are convenience aliases and can be expanded over time without changing the source format.

Current named escapes:

```text
RETURN, EOL, CR  $9B
ESC, ESCAPE      $1B
CLEAR, CLS       $7D
```

`\{INV:text}` sets the high bit of each ASCII character in `text`. For example, `\{INV:A}` emits `$C1`.

To convert an edited text file back to raw ATASCII bytes for the original
Action! compiler or ATR sidecars:

```sh
tools/actionc-to-atascii.sh FILE.ACT FILE.ACT.atascii
```
