# VBXE Ray Tracer Implementation Plan

## Objective

Build an Action! ray tracer for VBXE based on the Atari BASIC ten-liner used as
the behavioral reference. The renderer traces a full 320 by 192 image while
preserving the field of view of the original 80-pixel BASIC display. The
renderer uses the larger VBXE palette for smoother shading and restrained
color.

The implementation must keep these concerns separate:

```text
BASIC reference -> REAL ray kernel -> BYTE shade -> progressive scheduler
                                                     |
                                                     v
                                  VBXE surface <- XDL, palette, MEMAC
```

VBXE accelerates display, palette changes, clears, and memory copies. It does
not accelerate the floating-point ray calculations, which are expected to be
the dominant cost on a 1.79MHz 6502.

## Target display architecture

The display target is an SR320 8-bit overlay:

- 320 visible pixels by 192 lines;
- one byte per pixel;
- a 512-byte row stride;
- 98,304 bytes of VBXE local memory;
- twelve 8KB framebuffer bands, each containing 16 rows.

The padded stride makes every row start at the same offset within a bank and
prevents scanlines from crossing bank boundaries. MEMAC exposes one band at a
time through a CPU window selected only after checking that the linked program
does not occupy the same address range.

Use a three-byte value for VBXE's 19-bit local addresses. Do not represent a
local VBXE address as an Action! `CARD`.

## Behavioral reference

Before optimizing or visually improving the result, translate the BASIC
listing into structured pseudocode and record:

- camera and projection constants;
- the two sphere centers and sphere-selection rule;
- ray/sphere intersection and reflection order;
- floor-plane intersection and checkerboard expression;
- Atari BASIC `INT`, `SGN`, and rounding behavior;
- maximum reflection depth reached by the reference image;
- the progressive row order and provisional vertical spans.

Split the BASIC `Q` array into independent tables:

- `Dither(16)`;
- two joystick-delta tables of 11 entries each;
- one graphics-mode table of 11 entries;
- `Notes(64)`.

Run the BASIC program in Altirra with neutral joystick input and retain a final
screenshot, Graphics 9 screen-memory capture, selected pixels, and a shade
histogram as the behavioral oracle.

## Slice 1: `ATARI.VBXE`

Add a low-level embedded hardware module containing:

- register offsets for the `$D640-$D65F` and `$D740-$D75F` register banks;
- control-bit and XDL constants;
- palette, blitter, and MEMAC registers;
- volatile access to the selected register bank;
- a three-byte VBXE local-address representation;
- read-only detection of the two common register locations.

Detection must recognize a supported full FX core family instead of requiring
one exact revision byte. It must not write to a candidate register bank before
the read-only probe succeeds.

Acceptance criteria:

- both register locations can be detected;
- the core and minor revisions can be read;
- absence of VBXE is reported without changing video state;
- focused tests cover exported register offsets and module resolution.

## Slice 2: minimal VBXE surface

Add a small high-level screen module for the sample. It owns policy that does
not belong in the hardware module:

- allocation of XDL and framebuffer regions in VBXE RAM;
- SR320 overlay construction;
- 512-byte row stride;
- palette initialization;
- MEMAC bank mapping;
- row-address calculation;
- screen clearing and shutdown.

The first validation sample draws a gradient and visible markers around rows
63/64 and 127/128. It must be tested in Altirra with VBXE at both register
locations.

## Slice 3: native REAL square root

Add a reusable pointer-based square-root routine to `ATARI.REAL`:

```action
FPP.Sqrt(@input,@result)
```

Use bounded Newton iterations and define behavior for zero, positive values,
negative values, and convergence failure. Do not add a by-value REAL call ABI.
Projection constants such as `20*1.7` and `80*1.7` are folded at compile time
and are not recomputed per pixel.

Keep the implementation behind the portable `MATH.Sqrt` facade. Selective
linking lets geometry-only programs use the Atari provider without retaining
unreferenced text and console-I/O routines.

Acceptance criteria:

- zero, one, two, four, small, and large positive values are covered;
- the module analyzes and compiles with both modern backends;
- the existing REAL module API remains compatible.

## Slice 4: faithful ray kernel

Implement a hardware-independent kernel for:

- camera-ray construction and normalization;
- selection of the initial sphere from the horizontal ray direction;
- unit-sphere intersection;
- reflection between the two spheres;
- floor intersection and checkerboard calculation;
- conversion of brightness to a byte shade.

Use native `REAL` only for geometry. Pixel coordinates, table indices,
counters, shade values, and framebuffer addresses remain integer types. Keep
the hot kernel flat because native REAL values currently cross routine
boundaries through pointers rather than by value.

Add a defensive reflection limit only after measuring the reference image, and
choose a value above the observed maximum.

Acceptance criteria:

- representative pixels agree with the behavioral oracle;
- a deterministic checksum and shade histogram are recorded;
- the kernel does not access VBXE hardware directly.

## Slice 5: progressive renderer

Preserve the BASIC program's dyadic refinement. It calculates each final row
once but quickly covers the screen with provisional vertical spans.

For every selected row:

1. trace all 320 SR320 pixels;
2. replicate the row over its current provisional vertical span;
3. continue with the next refinement level.

Start with CPU copies through MEMAC. Once the result is correct, use the VBXE
blitter for clear operations and provisional row replication.

The VBXE renderer replaces the original 4 by 4 ordered dither and 16-level
Graphics 9 mapping with a smooth, monotonic 256-entry cold-steel ramp.

## Slice 6: measurement and optimization

Use `RTCLOK` to record time per pass, scanline, and complete image. Also record
the number of intersections, maximum reflection depth, and total rays.

Optimize in this order:

1. precompute horizontal and vertical camera coordinates;
2. hoist invariant REAL constants;
3. reduce REAL/integer conversions;
4. keep framebuffer calculations integer-only;
5. use the VBXE blitter for row replication;
6. consider fixed-point or lookup approximations only after profiling.

Every optimization must preserve the reference checksum or document the
intended visual change.

## Slice 7: native SR320 color rendering

After the reference renderer is correct and measured:

- optionally retain a fast 80-pixel preview mode for comparison;
- optionally give objects, checkerboard squares, and background separate
  palette ramps.

If native 320 by 192 REAL rendering is not practical, keep a 160-pixel mode as
the default preview and expose full resolution as a final-quality pass.

## Slice 8: interaction, sound, and delivery

Only after rendering is stable:

- port the note data through `ATARI.POKEY`;
- port joystick-controlled color changes;
- add restart and render-mode controls;
- keep sound and input outside the ray kernel.

Ship:

- `embedded/modules/atari/vbxe.act`;
- a reusable VBXE surface module or sample-local support module;
- a VBXE detection/gradient sample;
- `samples/vbxe/raytracer.act`;
- build and Altirra configuration documentation.

The expected build command is:

```sh
actionc --profile modern --runtime standalone samples/vbxe/raytracer.act
```

Execution validation initially uses Altirra because the repository's internal
runner does not emulate VBXE.

## Commit discipline

Each numbered slice is committed independently after its focused tests pass.
Compiler or IR changes are not expected. If a missing compiler capability is
discovered, implement it as its own general-purpose slice with regression tests
rather than adding sample-specific behavior.
