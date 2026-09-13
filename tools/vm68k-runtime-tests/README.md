# MC68000 execution tests

Run `cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml`.
The independent workspace pins [r68k 0.2.2](https://docs.rs/r68k/0.2.2/r68k/).
The compiler does not depend on the emulator.

The adapter maps explicit RAM regions in a 1 MiB address space, uses big-endian
reset vectors, and calls the entry through a protected JSR/TRAP trampoline.
Completion requires the trampoline trap, a balanced stack, and intact D2–D7
and A2–A6. Code, vectors and unmapped stack guards reject writes. Unmapped reads
and writes are latched host-memory violations, not emulated hardware bus errors.
CPU address errors, illegal instructions, stray traps, stopped/halted states and
instruction-budget exhaustion have separate results. Failure reports retain
32 recent instruction addresses/opcodes and all registers.

`run` budgets attempted instructions (including faults and the completion trap).
It steps r68k through `execute_with_state(1, ...)`, whose argument is **cycles**.
Intercepted exceptions consume one synthetic cycle and terminate the run;
cycle counts are diagnostic, not a claim of exception timing accuracy.

`tests/qualification.rs` uses literal encodings independent of actionc, with
references to the [Motorola programmer's manual](https://www.nxp.com/docs/en/reference-manual/M68000PRM.pdf).
Its tests cover transfers, byte order, condition flags, branches, nested stack
frames, exceptions, guards, timeout and isolation between parallel instances.

Compile and run a source file with:

```sh
cargo run --manifest-path tools/vm68k-runtime-tests/Cargo.toml -- program.act --origin 0x10000 --budget 1000000
```

Use `--no-opt` to execute raw verified NIR lowering. The runner prints supported
scalar globals by compiler-emitted symbol, with signed decimal and hexadecimal
values. `actionc::compiler::native::compile_file` accepts full-width origins,
module search paths and a project root; it returns a native image and physical
instruction listing without a 6502 runtime or Atari output wrapper.

The initial native subset covers integer arithmetic and casts, logical shifts,
comparisons, IF/CASE/loops, direct and typed indirect calls, recursive automatic
frames, pointers, one-dimensional arrays, record fields and overlapping copies.
The ordinary `MATH.INTEGER.AsrI`/`AsrLI` library also executes on MC68000.
Multiply/divide/remainder helpers, REAL operations, OS/runtime adapters, foreign
machine code and executable top-level statements are not yet supported.
This is a bare CPU development path; Amiga startup/object formats and the
public `actionc` output CLI remain follow-up work.

The acceptance benchmark is the shared TACLeBench insertion-sort algorithm:
209 independent C-reference cases, compiled once per optimization configuration
and executed in 418 fresh VMs. Its state is addressed by compiler symbols and
serialized in target byte order. No benchmark address belongs to a 6502 map.
CI runs this workspace on Linux, Windows and macOS; local validation alone does
not establish the status of those remote jobs.

Use `--dump build/probe` to write a version-1 JSON manifest, exact `.segmentN.bin`
files, and a `.machine.txt` physical instruction listing. The manifest records
the target, entry, initialized/zero-fill regions, stable symbol identities,
absolute/frame locations, scalar types and array layout. Dumps describe the
compiled image before execution. For example, from the repository root:

```sh
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml -- fixtures/runtime/tacle/insertsort/insertsort.act --dump build/insertsort
```
