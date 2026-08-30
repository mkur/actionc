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

`demoscene/unlimited-bobs.act` cycles four paths through persistent ANTIC
mode-E bitmaps, choosing a random non-grey hue and using palette fades for each
animation. It uses continuous phase catch-up across its four buffers and a
bottom text row showing the current animation and displayed bob count. The
demo declares `ORG $A000`, keeping the program above its four fixed screen
buffers. Compile with `--runtime standalone`; an explicit `--origin` remains
available when deliberately overriding the source placement.

`real-basics.act` is the complete introductory program from the
[native REAL tutorial](../docs/tutorials/REAL.md).

`graphics/fedora.act` translates an Atari BASIC 3D sine-surface plot whose
rendered shape resembles a fedora. It uses the portable `MATH.Sqr` and
`MATH.Sin` native REAL procedures.

Archived disk images, byte-exact extractions, raw ATASCII sidecars, and original
compiler outputs live under `corpora/`. Survey scripts and generated comparison
reports live under `surveys/`.
