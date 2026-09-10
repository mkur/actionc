# Signed Q8.8

`q8_8.act` demonstrates the embedded `MATH.Q8_8` library: ratio construction,
integer conversion, multiplication, division, truncation, motion accumulation,
and overflow. The stored INT is the mathematical value multiplied by 256.
No decimal fixed point formatter is needed to inspect these exact raw results.

Compile from the repository root:

```sh
cargo run --bin actionc -- --mode optimized --runtime standalone samples/fixed-point/q8_8.act
```

Compatibility, Optimized classic, and MIR6502 are covered with both `--runtime
cart` and `--runtime standalone`. This printing sample needs the Atari OS;
the numerical library itself runs standalone without ROMs.

Expected output:

```text
Signed Q8.8 (raw / 256)
1.5 raw = 384
1.5 * 2 raw = 768
-1 / 3 raw = -85
Trunc(-1.5) = -1
Four steps of 1.5 raw = 1536
Whole position = 6
Half position raw = 768
MaxValue * 2 wrapped raw = -2
```

The VM test checks this output in all six mode/runtime combinations. Run
`cargo test --locked --test fixed_q8_8` from `tools/vm-runtime-tests`.
See the [API guide](../../docs/FIXED_POINT_Q8_8.md) for range, rounding,
wrapping, and division-by-zero behavior.
