# Emitted native 65816 execution

This isolated workspace tests the
[native scalar emitter](../../docs/MIR65816_EMISSION_CONTRACT.md). It loads
self-contained JSON images produced by the public compiler API and executes
their bytes using `actionc-vm::native65816` at revision
`56ddc5c5de41f0e7294e87c440869550eaf53292`. The compiler does not depend on the VM.

Install ca65 and ld65 for the independent handwritten assembly probes, then run:

```sh
cargo test --locked --manifest-path tools/native65816-runtime-tests/Cargo.toml
```

Run that command from the actionc repository root. The dependencies are pinned
by the manifest and `Cargo.lock`. The fixture assembler is invoked directly;
missing assembler tools fail the tests. No Atari ROM, OS or device intercept
participates. The earlier `vm65816-runtime-tests` workspace remains the
jgenesis CPU comparison experiment.

## Tests

- `arithmetic`: 72 executions cover BYTE, CARD, INT, SIZE, LONGCARD and LONGINT
  at zero, carry/borrow, sign boundaries and unequal/equal values, with raw and
  optimized NIR. Host integer results independently check the emitted add,
  subtract, negate, bitwise and six comparison operations.
- `execution`: recursion, mutable parameters, promoted loop edges, local
  addresses passed to another routine, initialized array descriptors, odd
  record strides, callable storage, code in multiple banks, word data crossing
  `$12FFFF`, full 24-bit results and exact volatile byte accesses with an
  unmapped neighbor.
- `interop`: ca65 callers enter Action! and Action! calls leaf assembly, with
  the published BYTE/CARD/pointer/LONGINT layout. It checks all 13 outgoing
  bytes, zero-argument padding, A/X results, narrow unused bits and assembly
  clobbering all 64 scratch bytes. Both I states and both optimization modes
  are covered.
- `indirect`: callable targets at offsets $0000/$FFFF, cross-bank calls, every
  scalar result class, mixed assembly arguments and six-byte transient checks.
- `stack_faults`: floor, unsigned underflow, ceiling and pre-call return-address
  costs. The raw fault adapter receives the required A/X/S state before any
  prohibited stack write.

The harness initializes a bootstrap domain at `$002000`, a guarded stack in
`$004000..$005FFF`, and an independent assembly caller at `$040000`. It maps
only declared memory, rejects writes to code and uses finite CPU-cycle budgets.
These are harness addresses, not a board memory map. IRQ/NMI are not injected
by this slice. Domain tail bytes and stack guards must remain intact.

Instruction encodings and calling sequences follow the
[WDC W65C816S datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf)
and the repository's physical ABI. The assembler fixture supplies independent
code and literal expectations; exported addresses are the only compiler
metadata used to assemble its calls.

All 12 tests passed in debug and release on 2026-09-16. Local toolchain:
Rust 1.95.0, ca65/ld65 2.18, macOS ARM64. The
[CPU checkpoint](../../docs/MIR65816_CPU_EXECUTION_CHECKPOINT.md) documents the
remaining timing limitations. Context switching and G1–G6 acceptance remain
later compiler qualification work.
