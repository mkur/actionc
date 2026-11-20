# MIR6502 block-local spills at calls

Mandelbrot's `Iterate` now needs zero inline spill bytes on both runtimes. Three compiler-owned byte slots previously stayed inline because each slot had accesses on both sides of a call. The values occupying those slots had already been consumed. The allocator now checks reads at each intervening call and liveness immediately afterwards. A value needed across even one call keeps its inline storage. [Allocator](../../../src/mir6502/materialize/spills.rs#L1913) · [Paired clobbering-call fixture](../../../fixtures/runtime/block_local_spill_calls.act)

## Mandelbrot

Full XEX lengths include storage and runtime code. Measurements use optimized MIR6502 at origin `$3000`, comparing `7e9c7fc3` with `c204a020` on identical program sources. Frame cycles run from the executable entry through the return from row 191 of the unmodified Atari sample. All 30,720 pixels in each image matched the integer oracle. [Atari Mandelbrot](../../../samples/graphics/mandelbrot/mbfixed.act) · [VM measurement and oracle](../../../tools/vm-runtime-tests/tests/oscar64_mandelbrot.rs#L460)

| Measure | Cartridge before → after | Standalone before → after |
| --- | ---: | ---: |
| XEX bytes | 1,871 → 1,896 (+25) | 2,336 → 2,270 (−66) |
| Frame cycles | 887,991,631 → 885,969,228 (−2,022,403) | 900,232,963 → 885,796,535 (−14,436,428) |
| Bytes of code in `Iterate` | 481 → 482 | 519 → 482 |
| Inline spill bytes in `Iterate` | 3 → 0 | 3 → 0 |
| Absolute spill instructions in `Iterate` | 37 → 0 | 37 → 0 |
| Named ZP homes in `Iterate` | 6 → 0 | 0 → 0 |

Both compilers inline the two calls to `SqrWide`. Each expanded square calls `Mult32`, and the cross product calls `MulFloor`. On cartridge the new layout puts `Mult32` immediately before `DrawRow` to reduce branch page penalties. The inliner moves the helper after saving the materialized program. The routine order then differs from the saved checkpoint, so the later named-home pass skips its trial. The six affected homes are `cx`, `cy`, `x`, `y`, `nextX` and `i`. Two viewport routines each grow by seven code bytes, while `DrawRow` shrinks by twelve. Both revisions also reject an expanded read footprint in `MulFloor` during core materialization. The helper reordering accounts for the difference in checkpoint availability. [Helper placement](../../../src/mir6502/materialize/inlining.rs#L884) · [Checkpoint validation](../../../src/mir6502/preparation.rs#L43) · [Named-home admission](../../../src/mir6502/parameter_homes.rs#L45)

All 135 direct calls per image returned the expected iteration count and restored the stack pointer. Timings include the same register setup and JSR on each run. At `(0,0)` the baseline executed 1,184 absolute spill accesses and the new image executes zero. The one-iteration cartridge case takes five more cycles, even though the full frame is faster.

| Q4.12 inputs as signed integers | Iterations | Cartridge cycles before → after | Standalone cycles before → after |
| --- | ---: | ---: | ---: |
| `(0, 0)` | 32 | 30,206 → 29,960 | 31,496 → 29,960 |
| `(8192, 0)` | 1 | 1,856 → 1,861 | 1,917 → 1,861 |
| `(-3072, 512)` | 27 | 72,980 → 72,777 | 74,081 → 72,777 |

To reproduce the full-frame qualification with the test ROMs installed, run the following command from the repository root.

```sh
cargo test --release --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test oscar64_mandelbrot time_unmodified_mandelbrot_through_last_row \
  -- --ignored --nocapture
```

## TN and Monte Carlo

| Program at `$3000` | Cartridge XEX bytes | Standalone XEX bytes |
| --- | ---: | ---: |
| TN | 9,539 → 9,531 | 9,678 → 9,670 |
| Non-graphics benchmark suite | 4,041 → 4,030 | 4,768 → 4,757 |

TN saves four code bytes in `FormatDirectoryRow`, two in `Window` and two bytes of storage. The comparison includes the same pending TN I/O source and review changes on both sides. Dispatch, panel state, tags, key input, CIO and file-copy controls passed. The panel and copy matrix also passed for both TN and TNDBG at the comparison origin on both runtimes. [TN VM tests](../../../tools/vm-runtime-tests/tests/tn_dispatch.rs)

Monte Carlo's `Run` shrinks from 239 to 229 code bytes and releases one inline byte. Its hot low-byte spill now uses `$EF`. The direct VM run saves 100,000 cycles on both runtimes, from 8,523,467 to 8,423,467 on cartridge and from 8,583,467 to 8,483,467 standalone. To supply identical random input, the harness redirects the operands of the two absolute POKEY loads to a RAM byte, preserving their instruction lengths and timing. It supplies the high byte of a 32-bit linear congruential generator seeded with 1, using multiplier 1,664,525 and increment 1,013,904,223. All four runs consumed 20,000 bytes, counted 7,815 hits in 10,000 probes and stored 31,260 as the estimate. [Monte Carlo kernel](../../../samples/benchmarks/bench/montecarlo.act)

## Other checks

The paired placement control compiles and executes on cartridge and standalone with LF and CRLF input. A callee overwrites the entire private pool. A byte used by the call or live until the next read retains its previous value, and a fresh definition after the call can use ZP. The cross-call and parameter-home integration targets passed, including MADS reconstruction and relocation in the cross-call target. [Allocator controls](../../../src/mir6502/materialize/block_local_spill_tests.rs) · [VM control](../../../tools/vm-runtime-tests/tests/cross_call_homes.rs)

The cheaper spills also affect the stride-20 address cost test. For the byte-indexed `Address`, the multiply-based sequence costs 73 bytes at selection versus 80 for shift/add. The emitted routine is 72 bytes. For `ReadViaLocal`, the helper-free sequence wins at 107 versus 112 bytes. The arithmetic inlining test checks the stored result and every declared ABI return byte for both 16-bit and 32-bit functions. [Address selection](../../../src/mir6502/materialize/address_values.rs) · [Arithmetic inlining controls](../../../src/mir6502/materialize/inlining_scalar_tests.rs)

Six existing failures reproduced on the starting compiler. Four are materializer assertions covering constructor pointer storage and scalar-call capture. The other two are the frozen `CheckRoom` size (73 actual, 89 expected on both revisions) and the late-spill cleanup proof-blocker assertion. The VM home-invalidation target passed.
