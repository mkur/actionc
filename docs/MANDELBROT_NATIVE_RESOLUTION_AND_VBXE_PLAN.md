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
were initially planned. The VBXE image oracle exposed the general MIR6502
index-width defect below; commit that prerequisite repair separately and run
the required NIR and full compiler checks.

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

## Prerequisite compiler correction

The first VBXE selected-row render passed in Optimized classic but failed in
MIR6502. A fused `destination(index)=value+1` byte store used Y alone even when
`index` was a CARD. At index 256 it overwrote index zero. Element width does not
prove index width. Byte-value and byte-arithmetic store selectors now require
a known byte index for this compact path; word indexes use the existing full
address calculation. NIR and the sample source contract remain unchanged.

A focused final-code regression reproduces the pre-fix failure at index 256.
It checks 864 combinations of pointer/array backing, value/add/sub/XOR stores,
raw/optimized materialization, nine indexes through 769, and six byte values,
including unaligned bases, byte overflow, page carries and guarded memory.
The WARP.DEM materialized-MIR quality baseline intentionally changes from 26
to 32 explicit indexed-address operations (11 to 15 with global-address bases).
Unproven indexes now retain address preparation; the existing 6,730-byte output
budget still passes. This is a code-generation correctness change, not a NIR
fixture or printer change.

Validation passes: the focused regression, NIR fixture snapshots, the NIR sweep
(49/49), and the full compiler suite (3,080 passed, 22 existing ignored). The
complete Mandelbrot VM target also passes all eight tests / 5,352 executions
with the corrected compiler.
