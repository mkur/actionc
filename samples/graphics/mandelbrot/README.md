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
point. `fractal/mandelbrot.act` is their shared library module; it has no Main
procedure and should be imported by one of those programs.

The commands above build the current checkout. To refresh an installed
`actionc` after updating the repository, run:

```sh
cargo install --path . --locked --force --bin actionc
actionc --runtime standalone --mode mir6502 samples/graphics/mandelbrot/mbfixed.act
```

An older installed compiler can report `hexadecimal constant is too large`
for `$04000000`. The current compiler accepts this 32-bit LONGCARD literal;
refresh the installed compiler to enable that support.

Load the generated Atari executable in an emulator or on an Atari XL/XE. The
sample draws progressively and leaves the completed image on screen. Both
samples build in Compatibility, Optimized classic, and MIR6502 with either
runtime. Their project-local module is discovered relative to the source file.

The display uses Graphics(31), a full-screen 160x192 four-color bitmap, with
an Atari palette. It keeps the original eight pairs of C64 two-bit dither
patterns. Logical row `py` fills physical rows in
`[floor(py*192/100), floor((py+1)*192/100))`: 92 logical rows receive both
pattern halves and eight receive only the upper half. All 160x100 numerical
samples are preserved; this is an Atari display adaptation, not a byte-identical
C64 screen. Pixels reaching the iteration cap remain black.

Run `cargo test --locked --test oscar64_mandelbrot` from tools/vm-runtime-tests.
Four tests cover 2,437 VM executions: 2,424 numerical cases, six probe-output
runs, five rendered logical rows in each of six configurations, and a complete
16,000-pixel render in standalone MIR6502. The tests compare plotted colors,
untouched pixels, and palette registers with an independent integer oracle.
The pinned VM models CIO graphics calls; this checks the complete rendered
pixel image, not ANTIC scanout or the OS's screen-memory layout. Numerical
standalone runs need no ROMs; the printing and graphics tests load the OS.

Set `ACTIONC_MANDELBROT_ARTIFACT_DIR` when running the VM target to retain the
observed images as linear 160x192 two-bit `.bin` files (40 bytes per row,
leftmost pixel in bits 7..6), plus execution reports. These files pack the VM's
observed pixels for comparison; they are not dumps of Atari screen RAM.

The [implementation plan](../../../docs/Q4_12_MANDELBROT_IMPLEMENTATION_PLAN.md)
records the arithmetic contract and acceptance checks. The initial port uses
general LONGINT arithmetic and per-pixel Plot calls; speed optimization remains
a separate follow-up.

For reference, the instrumented full standalone MIR6502 render completes in
524,360,385 VM steps / 1,904,712,001 modeled CPU cycles and produces a 3,197-byte
load file. These measurements include the test completion call and the VM's
CIO interception; they are observations, not Atari wall-clock timings or
performance assertions.
