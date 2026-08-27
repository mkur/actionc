# VBXE samples

These samples target a VBXE FX 1.2x core at either standard register page,
`$D640` or `$D740`:

- `detect.act` reports the detected register page and revision;
- `gradient.act` validates the SR320 display, palette, and framebuffer banks;
- `ray-kernel-probe.act` runs selected hardware-independent reference rays;
- `raytracer.act` renders the complete progressive ray-traced image;
- `neon-scene-probe.act` checks representative REAL scene rays;
- `neon_planet.act` renders a colorful ringed planet above a reflective grid.

Build the ray tracer as a standalone XEX:

```sh
actionc --profile modern --runtime standalone samples/vbxe/raytracer.act
```

Build the neon planet as a standalone XEX:

```sh
actionc --profile modern --runtime standalone samples/vbxe/neon_planet.act
```

Enable a VBXE FX 1.2x expansion in Altirra, then open `raytracer.xex` or
`neon_planet.xex` as an executable image. Neither program requires the Action!
cartridge. The ray tracer keeps the original BASIC program's field of view
while tracing every pixel of a 320 by 192 SR320 overlay independently.

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

The neon planet is a second native-REAL ray tracer rather than a screen-space
illustration. It traces continuous perspective camera rays and analytically
intersects a sphere, tilted annulus, and floor plane. The floor reflection
traces a mirrored virtual sphere, while coherent surface normals provide
directional diffuse and Blinn-Phong-style specular lighting. Its cast shadow is
another sphere-ray occlusion test. Palette indexes divide into four 64-entry
material families for the sky, warm planet, cyan rings, and magenta floor. Once
rendering finishes, the brightest ring and grid colors pulse through palette
writes without changing framebuffer pixels. This makes the programmable VBXE
palette visible as an active display feature rather than only a setup step.

Both complete renderers progressively refine the same banked 320 by 192
framebuffer and are checked with both maintained compiler backends to keep all
load segments outside the `$A000-$BFFF` MEMAC window.
