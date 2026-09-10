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

The graphical Atari front end is the next slice of the
[implementation plan](../../../docs/Q4_12_MANDELBROT_IMPLEMENTATION_PLAN.md).
The numerical fixture runs in all three modes with both runtimes; standalone
numerical execution needs no ROMs. Run `cargo test --locked --test
oscar64_mandelbrot` from tools/vm-runtime-tests. The printing probe needs the OS.
