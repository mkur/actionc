# Atari SIO

`read-sector.act` reads sector 1 from D1: into a 128-byte buffer using
`ATARI.SIO`. It demonstrates device/command constants, auxiliary word/byte
views, a transfer variant and a returned result handled with CASE.

Build from the repository root:

```sh
cargo run --bin actionc -- --profile modern --backend mir6502 --runtime standalone -o build/read-sector.xex samples/sio/read-sector.act
```

Modern `--backend classic` and `--runtime cart` are also supported. The new
types require actionc's modern profile; the original cartridge compiler cannot
compile this source. A compatible Atari OS is required with either runtime.

Mount a known test ATR whose first sector is 128 bytes as D1: and run the
program on an Atari or emulator. It prints `Sector read` on success, otherwise
the raw numeric OS status (for example, 138 for a timeout). The program only
reads; it does not interpret the sector contents or use DOS file operations.
Avoid concurrent DOS access to the test disk.

The caller supplies accessible buffer storage and the correct transfer size.
SIO forwards input fields unchanged, and a failed read can modify part of the
buffer. The library calls the OS once per request; OS retries remain in effect.
Calls belong in normal foreground code, with OS interrupts and memory mapping
available.

The [library design](../../docs/ATARI_SIO_LIBRARY_DESIGN.md) describes the
contract and planned FujiNet network, JSON and adapter layers. At this stage
the transport and constants are implemented; the optional disk helpers and
FujiNet convenience modules are still planned.
