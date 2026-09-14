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

Use `--no-codegen-opt` to disable target temporary forwarding, instruction
selection, branch relaxation, pointer-alignment, control-flow selection and
register allocation
independently of NIR optimization. The
measurement example accepts the same flag; with it the
target passes retain conservative stack homes. Use the recorded compiler
revision as well when reproducing a baseline across shared NIR changes.

The initial native subset covers integer arithmetic and casts, logical shifts,
comparisons, IF/CASE/loops, direct and typed indirect calls, recursive automatic
frames, pointers, one-dimensional arrays, record fields and overlapping copies.
The ordinary `MATH.INTEGER.AsrI`/`AsrLI` library also executes on MC68000.
Integer multiplication, division and remainder are supported at all three
widths. Division by zero reports a terminal RuntimeFault and cannot resume.
REAL operations, OS/runtime adapters, foreign
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

Matrix1 also runs all 252 reference cases in both native modes, covering
LONGINT, INT, CARD and BYTE elements across three square/rectangular shapes.
Its adapter compares every matrix element and the signed checksum/status.

Binary search adds all 1,153 cases across LONGINT, BYTE, INT and CARD records
in both modes. SHA-0 adds 155 complete state/schedule cases in each mode, using
explicit byte decoding and retaining the reference digest and counter behavior.

The native DSP acceptance tests execute all 181 jfdctint cases, all 2,713 ADPCM
decoder checkpoints and all 1,561 encoder checkpoints in each NIR mode. DCT
checks both passes and rounding; ADPCM checks complete intermediate state and
the final report. Test adapters use compiler symbols and numeric endian
conversion, and exercise LF/CRLF source instrumentation without fixed addresses.

Measure ordinary benchmark entry points with:

```sh
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --example code_quality
```

Optional benchmark names restrict the run, for example `-- matrix1 sha`.
The CSV reports initialized executable bytes (including prefetch padding),
executed instructions through the completion trampoline, and the largest
individual emitted frame reservation. Frame size is not peak stack usage.
It also reports stack byte reads/writes, including arguments and return
addresses. Configuration, compiler revision/status and input Git hashes go to
stderr, so redirect stdout to a CSV and stderr to a companion log when retaining
a measurement. Existing CSV columns retain their meanings.
Use `--no-forward-temporaries`, `--no-select-instructions` or
`--no-relax-branches`, `--no-pointer-alignment`, `--no-control-flow` or
`--no-register-allocation` to isolate one target optimization;
`--no-codegen-opt` disables all six. `--no-opt`
restricts this example to raw NIR. Unknown options
and benchmark names are rejected.
SHA hashes `abc`; other programs run their default benchmark entry. Each run
checks completion and its expected result. Host execution time is not measured.
`--native-promotion` explicitly selects the default broader NIR promotion for
native loops; `--conservative-promotion` selects the existing profitability
policy. This choice is independent of target switches and is ignored in raw
NIR mode. Register allocation is enabled by default; `--register-allocation`
explicitly enables it. Wide native pointer storage facts are recognized by shared NIR
analysis under both policies, so older compiler revisions remain the reference
for the exact pre-migration baseline.
The [initial baseline](../../docs/mir68k-code-quality-baseline.csv) precedes
temporary forwarding, instruction selection and branch relaxation. The
[current measurements](../../docs/mir68k-code-quality-current.csv) and
[milestone comparison](../../docs/MIR68K_CODE_QUALITY_PLAN.md) record the result.

Compare equivalent C compiled by MC68000 GCC against Action! with
`python3 tools/compare_mir68k_c.py` from the repository root. Both use this r68k
harness and the same reference vectors. The optional cross-toolchain setup,
measurement contract and artifacts are described in the
[C reference README](../mir68k-c-reference/README.md); the
[comparison report](../../docs/MIR68K_C_COMPARISON.md) records the initial gaps.
