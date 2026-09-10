# Oscar64 fixed-point Mandelbrot

The shared `FRACTAL.MANDELBROT` kernel ports Oscar64's
[`samples/fractals/mbfixed.c`](https://github.com/drmortalwombat/oscar64/blob/8deb94c4d762bab3aa60c9565412691f01021bbb/samples/fractals/mbfixed.c)
at revision `8deb94c4d762bab3aa60c9565412691f01021bbb`, by drmortalwombat and
contributors. Adapted source retains GPL-3.0 attribution; see the repository
[license](../../../LICENSE). Q4.12 arithmetic is provided by the first-party
[MATH.Q4_12 library](../../../docs/FIXED_POINT_Q4_12.md).

The kernel retains 160x100 pixel coordinates, 32 iterations, the pre-update
escape test, and the original separately shifted squares and floor-rounded
cross product. The coordinate coefficients are the original truncated values
22937 and 25165, with offsets 10240 and 4915. Coordinate products are
nonnegative, so Q8.8's truncating Mul produces the same values as the original.
The signed recurrence uses Q4.12 MulFloor explicitly.

Keeping squares as unscaled LONGCARD values (Q8.24) preserves the radius test
against `$04000000`. Returning a rescaled 16-bit square would lose information.
Changing the cross product to truncation changes iteration counts at 180 of
the 16,000 viewport pixels; pixel (139,10) escapes at iteration 2 with the
original rule, versus 3 with truncation. These comparisons use independent
host integer calculations, not a run of the Oscar64 binary.

Run the small probe from the repository root:

```sh
cargo run --bin actionc -- --mode optimized --runtime standalone samples/graphics/mandelbrot/probe.act
```

Expected output:

```text
At c=0: 32
At c=2: 1
Pixel (139,10): 2
```

Build the graphical Atari sample from the repository root:

```sh
cargo run --bin actionc -- --mode mir6502 --runtime standalone samples/graphics/mandelbrot/mbfixed.act
```

`mbfixed.act` is the graphics entry point and `probe.act` is the printing entry
point. `fractal/mandelbrot.act` is their shared library module; it contains
functions but no entry PROC and must be imported by one of those programs.
The compiler rejects direct executable compilation of this library.

The commands above build the current checkout. To refresh an installed
`actionc` after updating the repository, run:

```sh
cargo install --path . --locked --force --bin actionc
actionc --runtime standalone --mode mir6502 samples/graphics/mandelbrot/mbfixed.act
```

An older installed compiler can report `hexadecimal constant is too large`
for `$04000000`. The current compiler accepts this 32-bit LONGCARD literal;
refresh the installed compiler to enable that support.

Load the generated `mbfixed.xex` in an emulator or on an Atari XL/XE. The
sample draws progressively and leaves the completed image on screen. Both
samples build in Compatibility, Optimized classic, and MIR6502 with either
runtime. Their project-local module is discovered relative to the source file.

The display uses Graphics(31), a full-screen 160x192 four-color bitmap. Every
one of its 30,720 pixels is calculated independently; there is no row replication
or image resampling. The original eight pairs of C64 two-bit dither patterns
use physical row parity, so their alternation remains continuous. Pixels
reaching the iteration cap remain black.

The shared `ViewportX(px,width)` and `ViewportY(py,height)` functions map display
pixels directly into raw Q4.12 coordinates:

```text
cx = floor(px * 14336 / width) - 10240
cy = floor(py * 9830 / height) - 4915
```

Use nonzero dimensions with pixel indexes below the corresponding dimension.
Products widen to LONGINT before multiplication. This spans the representable
bounds -2.5 through 1 horizontally and approximately -1.2 through 1.2 vertically,
excluding the right/bottom edge. The display mapping avoids the original
coefficient pre-rounding. `XCoord`, `YCoord`, and `Pixel` retain the original
160x100 Oscar64 sampling contract for the numerical probe and conformance tests.

The earlier display compressed 100 logical rows into 192 screen rows. Eight
single-height rows interrupted its dither pairs and produced horizontal seams;
direct physical-pixel sampling replaces that adaptation.

Run `cargo test --locked --test oscar64_mandelbrot` from tools/vm-runtime-tests.
Five tests cover 5,341 VM executions: 2,424 original numerical cases, 2,904
dimension-aware coordinate cases, six probe-output runs, twelve selected rows
in each of six configurations, and a complete 30,720-pixel render in standalone
MIR6502. The tests compare plotted colors,
untouched pixels, and palette registers with an independent integer oracle.
The pinned VM models CIO graphics calls; this checks the complete rendered
pixel image, not ANTIC scanout or the OS's screen-memory layout. Numerical
standalone runs need no ROMs; the printing and graphics tests load the OS.

A manual full render also completed in Atari800 with XL hardware, the bundled
AltirraOS ROM, and BASIC disabled, using the real OS graphics routines. All
7,680 bytes of screen RAM matched the oracle-checked VM image, and the final
row counter and palette matched. This supplements the automated CIO-model
tests with a complete native OS render check.

Set `ACTIONC_MANDELBROT_ARTIFACT_DIR` when running the VM target to retain the
observed images as linear 160x192 two-bit `.bin` files (40 bytes per row,
leftmost pixel in bits 7..6), plus execution reports. These files pack the VM's
observed pixels for comparison; they are not dumps of Atari screen RAM.

The [original implementation plan](../../../docs/Q4_12_MANDELBROT_IMPLEMENTATION_PLAN.md)
records the arithmetic contract, and the
[display plan](../../../docs/MANDELBROT_NATIVE_RESOLUTION_AND_VBXE_PLAN.md)
covers direct sampling and VBXE. The port uses
general LONGINT arithmetic and per-pixel Plot calls; speed optimization remains
a separate follow-up.
