# Native-resolution Atari and VBXE Mandelbrot

Status: slice 1 complete; VBXE slice in progress. Commit each tested slice.

## Contract

Keep the pinned Oscar64 160x100 coordinate/probe conformance API unchanged.
Both displays call the same Q4.12 Iterate kernel (32 updates, wide radius test,
explicit floor rounding). Add dimension-aware display coordinates:
`cx = floor(px * 14336 / width) - 10240` and
`cy = floor(py * 9830 / height) - 4915`, in raw Q4.12 units. Dimensions are
nonzero and pixel indexes range from zero through dimension minus one. Signed
LONGINT intermediates preserve the products. This samples the representable
viewport bounds directly, without the original coefficient pre-rounding.

1. **Native Atari display.** Calculate all 160x192 pixels independently. Keep
   four colors and the original dither pattern pairs, selecting their phase
   from physical row parity. Remove row replication and the periodic seams it
   caused. Update the independent image oracle and check both row parities,
   former seam locations, and a complete image. Preserve the original numerical
   port tests. Build all six modes/runtimes and repeat a full native-OS render
   in Atari800.
2. **VBXE display.** Add a standalone 320x192 SR320 renderer with direct
   palette colors and no dithering or replicated rows/pixels. Reuse the existing
   samples/vbxe/shared/screen.act layer (512-byte stride, twelve 8KB banks,
   CPU window $A000-$BFFF), and keep generated segments outside its window.
   Use an explicit missing/incompatible-VBXE message. Check both register pages,
   palette writes, XDL, banked framebuffer pixels and padding through a focused
   VM hook model; include representative rows in both maintained backends and
   one full image. Document that this model does not emulate VBXE scanout.

## Acceptance

Run the sample build catalog, original Mandelbrot numerical/probe checks, and
focused display VM checks from tools/vm-runtime-tests with --locked. Use host
i64 arithmetic for independent coordinate/iteration expectations. Verify actual
completion, full output regions, and hardware setup; watchdog expiry is failure.
Rebuild the runnable XEX files and document the VBXE module path and emulator
configuration. Update coverage counts and this plan after each slice passes.
No compiler, IR, arithmetic-library, or original conformance-fixture changes
are planned. If such a repair becomes necessary, run the required NIR and full
compiler checks for that separate repair.

## Progress

Slice 1 passes the complete sample build catalog and five Mandelbrot VM tests
(5,341 executions). The native 160x192 image is checked in standalone MIR6502;
twelve selected rows cover both parities and former seam positions in all six
configurations. All original coordinate/recurrence/probe checks still pass.
New coordinate tests cover every horizontal position at widths 160 and 320,
every vertical position at height 192, and large CARD dimensions.

An Atari800 7.1.2 full render with the bundled AltirraOS, XL hardware and BASIC
disabled matches the independent host oracle in all 7,680 bytes of screen RAM,
plus the final row counter and palette. The new viewport has 6,029 capped
pixels and 290,471 updates. The full VM render completes in 1,002,779,718 steps;
these are observations, not performance assertions. No compiler or arithmetic
library changes were needed.
