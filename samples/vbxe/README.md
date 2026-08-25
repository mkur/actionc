# VBXE samples

These samples target a VBXE FX 1.2x core at either standard register page,
`$D640` or `$D740`:

- `detect.act` reports the detected register page and revision;
- `gradient.act` validates the SR320 display, palette, and framebuffer banks;
- `ray-kernel-probe.act` runs selected hardware-independent reference rays;
- `raytracer.act` renders the complete progressive ray-traced image.

Build the ray tracer as a standalone XEX:

```sh
actionc --profile modern --runtime standalone samples/vbxe/raytracer.act
```

Enable a VBXE FX 1.2x expansion in Altirra, then open `raytracer.xex` as an
executable image. The program does not require the Action! cartridge. It keeps
the original BASIC program's field of view while tracing every pixel of a
320 by 192 SR320 overlay independently.

The first pass traces rows 0, 64, and 128 and temporarily expands each result
over 64 display lines. Later passes fill the midpoints at progressively smaller
spacings until every source row has been calculated exactly once. The image
therefore appears quickly and sharpens in place while the much slower REAL
geometry continues.

The framebuffer uses a 512-byte stride and twelve 8KB regions in VBXE local
memory. MEMAC exposes one region at a time through `$A000-$BFFF`; compiler tests
verify that both maintained backends keep the standalone program outside that
CPU window.

The ray kernel in `ray.act` is independent of VBXE. It preserves the BASIC
camera, two alternating unit spheres, reflections, and checkerboard floor. It
uses all 256 entries of a monotonic cold-steel VBXE palette instead of the
original 16-level ordered dither. Representative rays are executed against the
Atari OS floating-point package in the VM test suite.
