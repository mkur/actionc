# action-compiler-vm Usage Notes

`action-compiler-vm` lives next to this repo at `../action-compiler-vm`.
Run it from `actionc/` with `cargo run --quiet --manifest-path
../action-compiler-vm/Cargo.toml -- run ...`.

## Build an object to trace

```sh
cargo run --quiet --bin actionc-emit -- --backend mir6502 --emit-load samples/tn/modern/TN.ACT > target/tn-mir.xex
cargo run --quiet --bin actionc-emit -- --backend mir6502 --emit-listing samples/tn/modern/TN.ACT > target/tn-mir.lst
cargo run --quiet --bin actionc-emit -- --backend mir6502 --emit-map samples/tn/modern/TN.ACT > target/tn-mir.map
cargo run --quiet --bin atari-load-info -- target/tn-mir.xex
```

`atari-load-info` prints the loaded code ranges and `RUNAD`. For TN, a
healthy deferred-storage layout should show only the code load segment plus
`RUNAD`; large arrays are intentionally absent from the load file.

To trace a binary produced by the classic code generator, build the object and
map with `--backend classic`:

```sh
cargo run --quiet --bin actionc-emit -- --backend classic --emit-load samples/tn/modern/TN.ACT > target/tn-classic.xex
cargo run --quiet --bin actionc-emit -- --backend classic --emit-listing samples/tn/modern/TN.ACT > target/tn-classic.lst
cargo run --quiet --bin actionc-emit -- --backend classic --emit-map samples/tn/modern/TN.ACT > target/tn-classic.map
```

## Basic object run

```sh
cargo run --quiet --manifest-path ../action-compiler-vm/Cargo.toml -- run \
  --cart roms/action.rom \
  --os roms/rev02.rom \
  --load-object target/tn-mir.xex \
  --dump-screen-on-stop \
  --max-steps 1000000
```

Useful TN/MyDOS startup pokes:

```sh
--poke '$0700=$4D' --poke '$076F=$A9' --poke '$070B=$01' --poke '$070A=$08'
```

## Host file mappings

The VM can satisfy simple CIO host file reads and writes:

```sh
--host-file '*.*:target/tn-dir.txt'
--host-file 'TN:/path/to/TN.ACT'
--host-output 'TNCOPY:target/tn-copy-out.bin'
```

For directory traces, create a plain text host file whose records look like
DOS directory rows, for example:

```text
:SRC     SYS 008
AUTHORS      001
CHANGES      002
TN           040
```

## Trace ranges and CIO

Use listing addresses to trace specific routines:

```sh
--trace-range '$7934:$79DF' \
--trace-range '$57D4:$5F3E' \
--trace-action-calls-from-map target/tn-mir.map \
--trace-cio \
--history 160
```

`--trace-cio` is very useful for data/code overlap bugs because each read
shows `buf=$....`. In the TN deferred-storage bug, bad output read directory
records into `$35E7`, inside `r_Par`; fixed output read into `$7AD2`, just
after the `$2C00-$7AD1` code segment.

`--trace-action-calls-from-map` reads actionc map `signature` rows, so the VM
can print named Action! call/return boundaries and decode call arguments from
the implemented ABI. Direct call arguments are shown from their fixed homes:
byte offset 0 in `A`, offset 1 in `X`, offset 2 in `Y`, and spill bytes in
zero page starting at `$A3`. This works for both MIR and legacy binaries as
long as the map was emitted by the same build as the object being traced.

Use `--trace-action-calls-from-listing target/tn-mir.lst` when only
routine names and call boundaries are needed. The map form is preferred for
debugging calls because it also contains parameter names, widths, and return
types.

## Code write protection

Protect generated PROC ranges from accidental writes:

```sh
--protect-code-from-listing target/tn-mir.lst
```

If the listing contains a zero-length PROC like `PROC Error $3892..$3892`,
the VM parser currently rejects it. Filter that line into a temporary listing:

```sh
rg -v "PROC Error" target/tn-mir.lst > target/tn-mir-protect.lst
```

Then use `--protect-code-from-listing target/tn-mir-protect.lst`.

## Keyboard and Q input

Queue an Atari key immediately:

```sh
--key-code C
```

Queue at a specific PC:

```sh
--key-at-pc '$72A8:C'
--key-at-pc-after '$7934:$72A8:C'
```

Synthetic Q: input uses ATASCII EOL for `\n`:

```sh
--q-input 'C\n'
--q-input-at-pc '$4000:C\n'
```

## Practical trace recipe for TN startup

```sh
cargo run --quiet --manifest-path ../action-compiler-vm/Cargo.toml -- run \
  --cart roms/action.rom \
  --os roms/rev02.rom \
  --load-object target/tn-mir.xex \
  --host-file '*.*:target/tn-dir.txt' \
  --poke '$0700=$4D' --poke '$076F=$A9' --poke '$070B=$01' --poke '$070A=$08' \
  --trace-range '$7934:$79DF' \
  --trace-range '$57D4:$5F3E' \
  --trace-action-calls-from-map target/tn-mir.map \
  --trace-cio \
  --history 160 \
  --dump-screen-on-stop \
  --max-steps 5000000
```

If this reaches `Handle` and the latest CIO directory read buffers are above
the emitted code end, startup directory storage is no longer overwriting code.
