# actionc nightly

This is an automated development build from the latest tested commit on
`main`. Nightly behavior and compiler output may change without notice.

The release provides separate archives for:

- Linux x86-64;
- Windows x86-64;
- macOS Apple Silicon;
- macOS Intel.

Each package contains `actionc`, `actionc-run`, `actionc-emit`, build identity,
documentation, and notices/source material for embedded third-party assets.
The package's `licenses/runtime-source` directory contains the exact GPL Action
runtime sources embedded in the compiler. The separate Action! source archive
contains corresponding source for the bundled cartridge. Use `SHA256SUMS` to
verify downloaded assets.

Windows and macOS binaries are currently unsigned. SmartScreen or Gatekeeper
may therefore display a warning. The exact source commit and build toolchain are
recorded in each package's `BUILD-INFO.txt`.
