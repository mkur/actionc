# Samples

This directory is for source files a user might reasonably compile or read as
examples.

```text
*.act       small standalone Action! examples
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

`graphics/sine-surface.act` translates an Atari BASIC 3D surface plot and uses
the portable `MATH.Sqr` and `MATH.Sin` native REAL procedures.

Archived disk images, byte-exact extractions, raw ATASCII sidecars, and original
compiler outputs live under `corpora/`. Survey scripts and generated comparison
reports live under `surveys/`.
