# Character constants

`USE ASCII` provides named character codes without runtime code, data or imports.

| Constants | Values |
| --- | --- |
| `NUL` | 0 |
| `TAB`, `LF`, `VT`, `FF`, `CR` | 9, 10, 11, 12, 13 |
| `SPACE`, `DOUBLE_QUOTE`, `ASTERISK` | 32, 34, 42 |
| `DIGIT_ZERO`, `DIGIT_NINE` | 48, 57 |
| `ATASCII_EOL` | $9b (Atari end-of-line, an extension to ASCII) |

Names are qualified, for example `ASCII.SPACE` or `ASCII.ATASCII_EOL`.
The Atari constant lives in the same module for convenience; it does not change
the standard ASCII values. This module supplies constants only, with no string
operations or character classification calls.
