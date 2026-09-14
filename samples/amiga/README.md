# Amiga examples

Compile these with `--runtime amiga` and run them from the Amiga Shell.
They select the MC68000 target through a source annotation.

```sh
actionc --runtime amiga -o build/hello.amiga samples/amiga/hello.act
```

| Source | Example |
| --- | --- |
| [hello.act](hello.act) | Greeting and BYTE, CARD and INT output |
| [integer-array.act](integer-array.act) | CASE, a loop, static/local arrays and a function pointer |
| [insertsort.act](insertsort.act) | Ten sorted values, checksum 65 and full element checks |
| [division-zero.act](division-zero.act) | Deliberate fault, status 20 and clean Shell return |

[Expected output](expected/) is checked byte for byte in the native test suite.
The insertion-sort driver includes the unchanged
[TACLeBench port](../../fixtures/runtime/tacle/insertsort/README.md), whose
`CheckResult()` returns zero on success. It also checks all ten values and the
sentinel independently.

See [Amiga usage and smoke testing](../../docs/AMIGA.md) for the supported SYS
calls, stack setup and current validation status. Real AmigaOS acceptance is
still pending; the automated tests run through r68k and an OS-call shim.
