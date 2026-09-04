# Samples

This directory is for source files a user might reasonably compile or read as
examples.

```text
*.act       small standalone Action! examples
benchmarks/ compiler benchmark suite and its benchmark modules
graphics/   graphics demonstrations
toolkit/    maintained modernized ACTION! Toolkit programs
tn/         maintained TOMS Navigator sources
vbxe/       VBXE detection, framebuffer, and ray-tracing examples
```

`inline-asm-fine-scroll.act` demonstrates MADS-style inline assembly with
Action objects, a statically relocated ANTIC display list, a `SCREEN`-encoded
display buffer, and an immediate vertical-blank routine.

`rainbow_asm.act` is a compact inline-assembly version of the original
`rainbow.act` raster effect.

See [`vbxe/README.md`](vbxe/README.md) for the VBXE configuration and build
instructions.

`demoscene/midpoint-displacement.act` generates a one-dimensional midpoint-
displacement landscape using integer arithmetic and draws it in high-resolution
graphics mode.

`demoscene/unlimited-bobs.act` cycles four paths through three persistent ANTIC
mode-E bitmaps, choosing a random non-grey hue and using palette fades for each
animation. It uses continuous phase catch-up across its three buffers and a
bottom text row showing the current animation and displayed bob count. The
demo declares `ORG $8000`, keeping the program above its three fixed screen
buffers. Compile with `--runtime standalone`; an explicit `--origin` remains
available when deliberately overriding the source placement.

`real-basics.act` is the complete introductory program from the
[native REAL tutorial](../docs/tutorials/REAL.md).

`graphics/fedora.act` translates an Atari BASIC 3D sine-surface plot whose
rendered shape resembles a fedora. It uses the portable `MATH.Sqr` and
`MATH.Sin` native REAL procedures.

`graphics/landscape.act` draws a procedurally perturbed, layered landscape in
GTIA mode 9 and leaves the completed image on screen. Its source is also
accepted by the original Action! cartridge compiler.

`graphics/unknown-pleasures/` contains data-driven renderings of the CP1919
pulse plot: a stock Graphics 8 version with 300 independent horizontal samples
and a VBXE SR320 version with quarter-scanline grayscale antialiasing.

[`benchmarks/`](benchmarks/README.md) contains the Action! port of the Atari
Mad Pascal benchmark suite, including complete, compatibility, and
non-graphics runners.

Archived disk images, byte-exact extractions, raw ATASCII sidecars, and original
compiler outputs live under `corpora/`. Survey scripts and generated comparison
reports live under `surveys/`.

## Build coverage

Every Action-family source in this directory has an explicit role in
`tests/sample_build_matrix.rs`:

- **Executable** sources have one or more known-good compiler, runtime, and
  module-path combinations. Release-tier cases cover the supported classic
  backend; advertised MIR6502 combinations are tracked separately as
  experimental.
- **Dependencies** name the executable samples which consume them.
- **Source-only** files carry a concrete reason why they are retained without
  an executable build contract.

The catalog test fails when a source is added, removed, renamed, duplicated, or
left without a role. Executable checks compile through the public compiler API,
parse the resulting Atari load file, verify `RUNAD`, and enforce any declared
origin or reserved-memory constraints.

Run the same gates locally with:

```sh
cargo test --test sample_build_matrix sample_catalog_
cargo test --test sample_build_matrix release_sample_builds_produce_valid_load_files
cargo test --test sample_build_matrix advertised_mir6502_sample_builds_produce_valid_load_files
```

The build cases are deliberately explicit rather than a Cartesian product.
Adding a sample therefore advertises only combinations that are actually
maintained and tested.
