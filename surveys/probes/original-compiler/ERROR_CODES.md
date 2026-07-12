# ACTION! Error Codes

Source: `../ACTION_REF_MANUAL.pdf`, Appendix C, "Error Code Explanation".

The manual says this appendix covers errors discovered by the ACTION! system
itself, not Atari OS errors in the range 128-255. It still lists error 128 for
the BREAK key.

| Code | Explanation |
| --- | --- |
| 0 | Out of system memory. See Part II section 4.3 and Part V section 4.4 in the manual. |
| 1 | Missing double quote in a string. |
| 2 ||
| 3 | Global variable symbol table full. |
| 4 | Local variable symbol table full. |
| 5 | `SET` directive syntax error. |
| 6 | Declaration error. Wrong declaration format. |
| 7 | Invalid argument list. A statement or routine was given too many arguments. |
| 8 | Variable not declared. Variables must be declared before use. |
| 9 | Not a constant. A variable was used where a constant was required. |
| 10 | Illegal assignment. An assignment form is not allowed, such as `var=5>7`. |
| 11 | Unknown error. The ACTION! system error routines were impaired, so the exact error cannot be reported. |
| 12 | Missing `THEN`. |
| 13 | Missing `FI`. |
| 14 | Out of code space. See Part V section 4.4 in the manual. |
| 15 | Missing `DO`. |
| 16 | Missing `TO`. |
| 17 | Bad expression. Illegal expression format. |
| 18 | Unmatched parentheses. |
| 19 | Missing `OD`. |
| 20 | Cannot allocate memory. The ACTION! system was impaired and cannot allocate more memory. |
| 21 | Illegal array reference. |
| 22 | Input file is too large. Break it into smaller pieces. |
| 23 | Illegal conditional expression. |
| 24 | Illegal `FOR` statement syntax. |
| 25 | Illegal `EXIT`. There is no `DO`/`OD` loop for the `EXIT` to exit from. |
| 26 | Nesting too deep. Maximum nesting is 16 levels. |
| 27 | Illegal `TYPE` syntax. |
| 28 | Illegal `RETURN`. |
| 61 | Out of symbol table space. See Part IV in the manual. |
| 128 | BREAK key was used to stop program execution. |

Notes:

- The PDF is OCR-derived, so the source text contains a few typos. This note
  normalizes obvious typos such as "errorp" to "error" and "uneble" to "unable".
- Error 17 is the one seen from the original compiler when it reports
  `Error: 17`; the manual defines it as a bad expression or illegal expression
  format.

