# MC68000 C reference

Use GCC as an independent code-generation reference for MIR68K. The initial
pair is insertion sort and the 10×10 LONGINT matrix1 variant. These C files
mirror the actual Action! drivers and kernels, including 16-bit loop counters,
32-bit elements, persistent state, command inputs and volatile declarations.
The pinned upstream C oracles remain unchanged and supply expected results.

On macOS, install [Homebrew's m68k-elf-gcc](https://formulae.brew.sh/formula/m68k-elf-gcc):

```sh
brew install m68k-elf-gcc
python3 tools/compare_mir68k_c.py
```

Run from the repository root. Optional benchmark names select a subset:

```sh
python3 tools/compare_mir68k_c.py matrix1
```

`--tool-prefix` selects another compatible GNU cross-toolchain prefix, and
`--build-dir` changes the artifact directory. The recorded baseline uses
GCC 16.2.0 and GNU binutils 2.47.20260726. The script records actual versions,
commands, selected libgcc, source hashes, compiler revision and compiler
worktree status in `build/mir68k-c-reference/toolchain.json`.
The same file records Action! switches; `actionc-options.txt` records the resolved
native settings and input Git hashes. `--no-opt` selects raw Action! NIR;
`--no-codegen-opt` disables target optimization. Individual switches
`--no-forward-temporaries`, `--no-select-instructions`, `--no-relax-branches`
`--no-pointer-alignment` and `--no-control-flow`
support controlled comparisons. These switches leave GCC's configuration alone.
Use a separate `--build-dir` for each experiment to retain its results.

## Comparison contract

- Target the original MC68000 with `-mcpu=68000`, at `-O2` and `-Os`, without
  LTO. This selects the matching `m68000` libgcc variant in the tested package.
  Inspect the recorded library path when changing toolchains. Explicit
  `-msoft-float` unexpectedly selects a different multilib in this package;
  it is unnecessary for these integer-only programs and this CPU target.
- Keep C types explicit with `<stdint.h>`. Do not use `-mshort` to change the
  entire ABI. `-fwrapv` defines the signed add/multiply wrapping used by matrix1;
  unsigned arithmetic wraps normally. Insertion-sort vectors preserve its
  required sentinel ordering. See [GCC's target options](https://gcc.gnu.org/onlinedocs/gcc/M680x0-Options.html)
  and [code-generation options](https://gcc.gnu.org/onlinedocs/gcc/Code-Gen-Options.html).
- Link a freestanding image with no OS startup or libc. GCC's required
  arithmetic helpers and the small memory-copy/clear implementation are linked
  and counted. The memory helpers use ordinary byte loops; this is not an
  optimized libc comparison. Unreferenced sections are discarded.
- Execute both C modes and optimized Action! in the same r68k VM, with the same
  origin, BSS initialization, protected trampoline, register/stack completion
  checks and instruction budget. The two languages retain their own internal
  ABIs; their common external entry takes no arguments and returns normally.
- Validate every insertion-sort vector (209) and every LONGINT/10×10×10 matrix1
  vector (22), including complete arrays, checksums and persistent statistics.
  Numeric reference words are decoded before writing big-endian guest memory.
  The comparison makes 693 reference executions plus six default-input runs.
- Report executable bytes, default-input instructions, default-input stack byte
  reads/writes, and summed reference-vector instructions. Code includes linked
  helpers and four bytes of prefetch padding, but excludes data and BSS.
  Instructions include initialization, result checking and trampoline completion.
  Stack traffic counts guest byte accesses, including arguments and return
  addresses; a longword read contributes four bytes. These are not CPU cycles
  or wall-clock performance claims.

## Artifacts and checks

The build directory contains C assembly, ELF files, linker maps, symbols,
disassemblies, section bytes and GCC `.su` stack reports, plus the Action!
symbol manifest, physical listing and disassembly. `comparison.csv` records
results only after all reference checks pass. GCC stack reports remain separate
because their conventions differ from Action!'s maximum individual frame
reservation; neither is automatically a measurement of peak call-stack usage.

The small `.image` transport lists binutils-extracted sections and symbols.
The harness validates its ranges and maps it into the existing VM. It does not
add an ELF parser or platform startup code to the compiler.

The GCC comparison is an explicit developer command, so ordinary tests do not
require a C cross-compiler. Manifest tests run in the native workspace's normal
test suite and cover LF/CRLF through the real loader, memory permissions,
completion, malformed records, overlapping ranges and symbol extents:

```sh
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --example c_reference
```

See the [measured comparison and improvement priorities](../../docs/MIR68K_C_COMPARISON.md).

## Provenance

Insertion sort derives from Sung-Soo Lim's SNU-RT Benchmark Suite through
[TACLeBench](../../fixtures/runtime/tacle/insertsort/insertsort.c); its license
permits use, modification and redistribution with acknowledgement. Matrix1
derives from Juan Martinez Velarde's DSP-Stone benchmark through
[TACLeBench](../../fixtures/runtime/tacle/matrix1/matrix1.c), with permission to
use, modify and redistribute freely. The C adaptations retain these credits.
