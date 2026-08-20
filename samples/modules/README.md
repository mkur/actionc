# Module examples

These samples are retained for the latent implementation and require the
default-off `experimental-named-modules` Cargo feature. Named modules are not
part of the current public language surface.

These examples exercise the Action 2027 module system without external include
or library directories. `SYS` and `ATARI.*` are supplied by the compiler's
embedded virtual filesystem.

- `hardware-rainbow.act` uses qualified Atari hardware and OS names.
- `sys-memory-qualified.act` calls compiler-provided `SYS` routines through the
  module qualifier.
- `sys-memory-open.act` uses all public `SYS` names with `USE ALL FROM SYS`.
- `local-runtime-override.act` supplies a local implementation of a runtime
  helper.
- `project/` is a multi-file program whose `DEMO.COLOR` module exports a public
  procedure to `main.act`.

Compile a single-file example without the Action! cartridge:

```sh
cargo run --features experimental-named-modules --bin actionc -- \
  --mode optimized --runtime standalone \
  samples/modules/hardware-rainbow.act
```

The directory containing the root source is automatically the project module
root, so the multi-file example needs no module-path option:

```sh
cargo run --features experimental-named-modules --bin actionc -- \
  --mode mir6502 --runtime standalone \
  samples/modules/project/main.act
```

It can also be compiled and launched directly:

```sh
cargo run --features experimental-named-modules --bin actionc-run -- \
  --no-cart samples/modules/hardware-rainbow.act
```
