# Standalone runtime example

`standalone-runtime.act` is ordinary Action! source: it does not declare or import
a module, and it calls resident routines by their traditional unqualified
names. The standalone runtime covers the whole Action! system library. The compiler selects and links just the required
runtime routines when `--runtime standalone` is used.

Run it without the Action! cartridge:

```sh
actionc-run --runtime standalone samples/standalone/standalone-runtime.act
```

The MIR6502 backend accepts the same source:

```sh
actionc-run --mode mir6502 --runtime standalone \
  samples/standalone/standalone-runtime.act
```

The program prints a standalone greeting, the sample inputs, and their product,
quotient, remainder, and shifted result, then waits for a keypress before
returning.
