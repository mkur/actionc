# VBXE samples

These samples target a VBXE FX 1.2x core at either standard register page,
`$D640` or `$D740`:

- `detect.act` reports the detected register page and revision;
- `gradient.act` validates the shared SR320 display and framebuffer;
- `shared/` contains reusable VBXE framebuffer support;
- `raytracer/spheres/` contains the original sphere kernel, probe, and renderer;
- `raytracer/neon/` contains the neon-planet scene, palette, probe, and renderer;
- `raytracer/fuji/` contains the extruded-Fuji scene, palette, probe, and renderer.

Build the spheres ray tracer as a standalone XEX:

```sh
actionc --module-path samples/vbxe --profile modern \
  --runtime standalone samples/vbxe/raytracer/spheres/spheres_raytracer.act
```

Build the neon planet as a standalone XEX:

```sh
actionc --module-path samples/vbxe --profile modern \
  --runtime standalone samples/vbxe/raytracer/neon/neon_raytracer.act
```

Build the 3D Fuji as a standalone XEX:

```sh
actionc --module-path samples/vbxe --profile modern \
  --runtime standalone samples/vbxe/raytracer/fuji/fuji_raytracer.act
```

Enable a VBXE FX 1.2x expansion in Altirra, then open `raytracer.xex` or
one of the other generated XEX files as an executable image. None of these
programs requires the Action! cartridge. The original ray tracer keeps the
BASIC program's field of view while tracing every pixel of a 320 by 192 SR320
overlay independently.

AltirraSDL also requires the VBXE device to be added to the emulated machine.
If a renderer appears black, build and run `gradient.act` first: a visible
gradient confirms that the SR320 overlay and VBXE memory window are configured.
The Fuji renderer paints its blue backdrop before starting the slower
native-REAL geometry, so it remains visibly active while the first object row
is traced.

The Fuji renderer starts with 32 by 32 pixel tiles, displaying each traced
sample immediately. Later passes use 16, 8, 4, 2, and finally 1 pixel tiles.
The complete composition therefore appears as a coarse mosaic before the much
slower native-REAL geometry sharpens it into the final image.

The framebuffer uses a 512-byte stride and twelve 8KB regions in VBXE local
memory. MEMAC exposes one region at a time through `$A000-$BFFF`; compiler tests
verify that both maintained backends keep the standalone program outside that
CPU window.

The sphere kernel in `spheres/spheres_scene.act` is independent of VBXE. It
preserves the BASIC camera, two alternating unit spheres, reflections, and
checkerboard floor. It uses all 256 entries of a monotonic cold-steel VBXE
palette instead of the original 16-level ordered dither. Representative rays
are executed against the Atari OS floating-point package in the VM test suite.

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

The Fuji scene is a third native-REAL kernel. It constructs the familiar
three-band silhouette from readable curves rather than a bitmap or imported
mesh, extrudes that silhouette through a finite depth, and rotates it around
the vertical axis. Rays that enter the near silhouette hit the red front face;
rays that enter only the far silhouette cross a side wall. Binary refinement
locates those side hits, and transformed surface normals give the extrusion
its darker directional shading. A dedicated VBXE palette separates the deep
blue backdrop, warm stars, bright face, and darker red side material.

All complete renderers progressively refine the same banked 320 by 192
framebuffer and are checked with both maintained compiler backends to keep all
load segments outside the `$A000-$BFFF` MEMAC window. Their hardware-independent
scene probes also execute in the 6502 VM to keep representative pixels stable
across the classic and MIR6502 backends.
