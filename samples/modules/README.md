# Module examples

These examples exercise the Action 2027 module system without external include
or library directories. `SYS` and `ATARI.*` are supplied by the compiler's
embedded virtual filesystem.

For a guided introduction that builds up to these examples, see the
[modules tutorial](../../docs/tutorials/MODULES.md).

- `hardware-rainbow.act` uses qualified Atari hardware and OS names.
- `hello.act` is a minimal named module using qualified `SYS` and Atari OS
  interfaces.
- `sys-memory-qualified.act` calls compiler-provided `SYS` routines through the
  module qualifier.
- `sys-memory-open.act` uses all public `SYS` names with `USE ALL FROM SYS`.
- `local-runtime-override.act` supplies a local implementation of a runtime
  helper.
- `project/` is a multi-file program whose `DEMO.COLOR` module exports a public
  procedure to `main.act`.

Compile a single-file example without the Action! cartridge:

```sh
cargo run --bin actionc -- \
  --mode optimized --runtime standalone \
  samples/modules/hardware-rainbow.act
```

The directory containing the root source is automatically the project module
root, so the multi-file example needs no module-path option:

```sh
cargo run --bin actionc -- \
  --mode mir6502 --runtime standalone \
  samples/modules/project/main.act
```

It can also be compiled and launched directly:

```sh
cargo run --bin actionc-run -- \
  --no-cart samples/modules/hardware-rainbow.act
```
