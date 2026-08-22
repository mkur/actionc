# Action! benchmark suite

This directory ports the language-supported kernels from Zbyti's
[Atari 800XL Mad Pascal Benchmark Suite](https://github.com/zbyti/a8-mad-pascal-bench-suite)
to Action!.  It is intended for compiler and backend comparisons, not as a
claim that unlike numeric formats or runtime libraries have identical costs.

`suite.act` runs all 16 ports and selects the modern profile with a source
annotation because two benchmarks use native `REAL`. `suite-nongraphics.act`
runs the ten supported benchmarks that do not manipulate a graphics display or
graphics buffer. An explicit command-line mode still overrides either source
annotation. `suite-compat.act` omits the two `REAL` benchmarks and can be
compiled with the compatibility profile. All roots use the named modules below
`bench/`; the source directory is therefore also the module root.

## Included benchmarks

| Mad Pascal unit | Action! module | Result |
| --- | --- | --- |
| `flames_pointer` | `BENCH.FLAMES_POINTER` | iterations in 250 RTC ticks |
| `flames_array` | `BENCH.FLAMES_ARRAY` | iterations in 250 RTC ticks |
| `landscape` | `BENCH.LANDSCAPE` | elapsed RTC ticks |
| `chessboard` | `BENCH.CHESSBOARD` | iterations in 200 RTC ticks |
| `lipsum` | `BENCH.LIPSUM` | elapsed RTC ticks |
| `qr_1d` | `BENCH.QR_1D` | iterations in 200 RTC ticks |
| `countdown_for` | `BENCH.COUNTDOWN_FOR` | elapsed RTC ticks |
| `countdown_while` | `BENCH.COUNTDOWN_WHILE` | elapsed RTC ticks |
| `sieve1028` | `BENCH.SIEVE1028` | elapsed RTC ticks |
| `sieve1899` | `BENCH.SIEVE1899` | elapsed RTC ticks |
| `bsort` | `BENCH.BSORT` | elapsed RTC ticks |
| `montecarlo` | `BENCH.MONTECARLO` | elapsed RTC ticks |
| `ludolphian` | `BENCH.LUDOLPHIAN` | elapsed RTC ticks; native `REAL` adaptation |
| `yoshplus` | `BENCH.YOSHPLUS` | iterations in 100 RTC ticks |
| `guessing` | `BENCH.GUESSING` | elapsed RTC ticks |
| `floating_single` | `BENCH.FLOATING_REAL` | elapsed RTC ticks; native `REAL` adaptation |

The screen-writing kernels use OS graphics setup instead of the Pascal suite's
custom display-list and vertical-blank UI. The two flame kernels retain their
fixed-address buffer calculations but do not install the original DLI-based
presentation layer. Their measured loops and memory-access patterns remain
intact. The harness reports standard kernels in Atari RTC ticks and preserves
the original time-window/iteration scoring for the four frame-rate kernels and
`yoshplus`.

Mad Pascal `SINGLE` is a four-byte IEEE-style binary value. Native Action 2027
`REAL` is Atari OS six-byte packed decimal, so the two floating-point results
measure the corresponding Action! implementation rather than identical
numeric machinery. They require a compatible Atari OS FPP under either
runtime.

## Deliberate omissions

- `matrix_trans` and `qr_2d` require multidimensional arrays.
- `md5_hash` requires Mad Pascal's external MD5 unit and native 32-bit
  arithmetic that Action! does not provide.
- `permutation` and `queens` are recursive. Action! routine parameters and
  locals have static storage, so directly translating these routines would
  silently produce invalid reentrant behavior.

## Build and run

Build the complete suite with either maintained modern backend:

```sh
# Compile from the repository root without an Action! cartridge dependency.
cargo run --bin actionc -- --mode optimized --runtime standalone \
  samples/modules/benchmarks/suite.act

cargo run --bin actionc -- --mode mir6502 --runtime standalone \
  samples/modules/benchmarks/suite.act
```

Build the non-`REAL` subset with the compatibility profile:

```sh
cargo run --bin actionc -- --mode compatibility --runtime standalone \
  samples/modules/benchmarks/suite-compat.act
```

To launch it directly through the repository runner:

```sh
cargo run --bin actionc-run -- --mode mir6502 --no-cart \
  samples/modules/benchmarks/suite.act
```

Run only the non-graphics benchmarks with:

```sh
cargo run --bin actionc-run -- --mode optimized --no-cart \
  samples/modules/benchmarks/suite-nongraphics.act
```

Plain `actionc suite.act` still defaults to the cartridge runtime. Such an
object must be launched with an Action! cartridge installed. Use
`--runtime standalone` when compiling manually, or `actionc-run --no-cart`,
when no cartridge is present.

Results stay on screen after the suite completes. Iteration scores are printed
as five decimal digits, matching the original suite's display precision. The
runner announces each benchmark before it starts; several complete-suite
kernels intentionally take a substantial amount of time on a stock 6502.
