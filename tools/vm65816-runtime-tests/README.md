# 65816 CPU test drive

This independent workspace evaluates jgenesis's Rust `wdc65816-emu` core for
the [Exec readiness requirements](../../docs/MIR65816_EXEC_READINESS_REQUIREMENTS.md).
It executes literal machine code, without actionc, ROMs, Atari OS, or SNES
devices. It does not yet load compiler artifacts or qualify MIR65816 emission.

The subsequent [X65 checkpoint](../../docs/MIR65816_CPU_EXECUTION_CHECKPOINT.md)
records the chosen Rust port and VM integration. This workspace is retained as
the jgenesis comparison experiment.

## Dependency and build

The dependency is pinned to jgenesis revision
[`22f62e89f6faf33164411987e4d52703196cc8d1`](https://github.com/jsgroth/jgenesis/tree/22f62e89f6faf33164411987e4d52703196cc8d1)
and its transitive dependencies are locked in `Cargo.lock`.
The core and common Rust libraries build without the emulator GUI or SDL.

The first attempt used HEAD `d19ee94f64798946e30d8be7a3dc6f1f9f8732c8`.
That revision's shared audio code requires the algebraic floating-point APIs
from Rust 1.98 and fails with this repository's Rust 1.95 toolchain. The pin
predates that audio change; `git diff` confirms the entire `cpu/wdc65816-emu`
directory is unchanged between the two revisions. No upstream code was patched.
The enclosing jgenesis workspace version at the selected revision is 0.14.0.

Run from the repository root:

```sh
cargo test --locked --manifest-path tools/vm65816-runtime-tests/Cargo.toml
cargo test --locked --release --manifest-path tools/vm65816-runtime-tests/Cargo.toml
```

## Executed coverage

`tests/qualification.rs` covers:

- Reset-vector startup, native-mode entry, stack and direct-page initialization.
- All four native M/X combinations, hidden accumulator B preservation, and
  truncation of indexes when switching to eight-bit index mode.
- 28 ADC/SBC cases with independent host result/carry/overflow/negative/zero
  expectations, decimal addition, and a counted loop exceeding 255 iterations.
- Little-endian word accesses across a bank boundary, long indexed addresses,
  direct-page three-byte pointers, and a stack-relative word round trip.
- Nested JSL/RTL calls across three program banks, exact return-address bytes,
  an indirect long jump, and PC wrapping within its program bank.
- IRQ masking, nested status saves/restores, native interrupt frames, and
  assembly save/restore of every architectural register in all four widths.
- NMI under IRQ masking, a held NMI level without repeated entry, a subsequent
  edge, and NMI interrupting and returning through an IRQ handler.
- IRQ assertion at each cycle of a long word store, with both bytes written
  before interrupt entry.
- MVN and MVP restarted through IRQ/RTI at eight assertion positions each,
  including index wrapping, exact copy counts, and restored A/X/Y/DBR.
- WAI wake-up with masked IRQ, cloning an in-flight instruction and reproducing
  its remaining bus trace, a one-byte device access with an unmapped neighbor,
  code/stack/memory guards, and finite execution budgets.

The encodings and expectations follow the
[WDC W65C816S datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf),
particularly the instruction tables and sections 2, 3 and 7. The external CPU
source is not used to calculate expected arithmetic, frames, or copied bytes.

## Confirmed limitations

Two additional tests describe expected hardware behavior and reproduce upstream
failures. They are explicitly ignored during normal runs. To run them:

```sh
cargo test --locked --manifest-path tools/vm65816-runtime-tests/Cargo.toml \
  --test limitations -- --ignored --nocapture
```

**This command currently fails both tests.** Their assertions have not been
weakened to accept the observed behavior.

1. **A short raw NMI pulse can be lost.** A pulse asserted for two complete
   ticks inside a six-cycle LDA-long instruction is deasserted before the
   core's interrupt polling point. No NMI occurs. The
   [instruction engine](https://github.com/jsgroth/jgenesis/blob/22f62e89f6faf33164411987e4d52703196cc8d1/cpu/wdc65816-emu/src/core/instructions.rs)
   polls signals at selected cycles; the SNES integration supplies pending
   interrupt state through the bus. Our probe deliberately supplies raw levels
   without stretching or latching them. Held-level NMI tests pass, but that
   does not establish raw pin behavior. A future adapter or upstream correction
   must preserve edges, including a second edge during interrupt entry, and
   have its own qualification before claiming arbitrary-cycle NMI support.
2. **`reset()` retains D and DBR.** Starting with D=$2345 and DBR=$67, reset
   leaves both values unchanged instead of clearing them. The successful
   bootstrap test starts from a newly constructed CPU, where those registers
   are already zero. Reusing an instance for hardware reset requires a
   correction or an explicitly tested reset adapter.

Other limits found by source inspection:

- The bus interface has no ABORT input; host guard failures are not hardware
  ABORT exceptions.
- `reset()` performs its vector reads directly; exact reset-cycle timing is
  not implemented by that API.
- This is an instruction/cycle-oriented CPU interface, not a model of every
  W65C816S pin. It provides no VDA/VPA/VPB signals, and some cycles are reported
  only as `idle()`.

## Harness contract and assessment

The bus stores a 16 MiB address space but allows access only to explicitly
mapped regions. Code and vectors can be read-only; stacks and data are writable.
Violations panic on the host with address and cycle information. `tick()` calls
the core once; `step()` completes an instruction or interrupt entry with a
32-tick limit. WAI/STP may remain idle, so bounded `run_until`/`run_to` are used
for completion. Trace entries record byte reads/writes or idle calls with the
current tick number. Direct reset-vector reads are outside that tick accounting.
Trace storage is unbounded within a run; this small qualification harness is
not yet the long-running benchmark runner.

**Use this core for initial emitted-code bring-up, with a conditional decision
for Exec interrupt qualification.** Banked memory, far calls, mode changes,
interrupt frames and block-move restart work in the tested cases. Keep the
dependency external and pinned. Address NMI edge preservation and reset
semantics before expanding the accepted interrupt model. No full ISA sweep,
real-hardware comparison, compiler-generated context switch, or G1–G6 readiness
claim follows from these tests.

## AltirraSDL as a second implementation

Use AltirraSDL's 65816 support for independent cross-checks and subsequent Atari
machine integration. Its locally installed
[AltirraBridge](../../docs/ALTIRRA_BRIDGE_USAGE.md) also has a headless server,
binary loading, memory inspection, breakpoints, and frame stepping. This can
complement the embedded Rust CPU tests.

A separate local headless instance was started and queried successfully. The
installed SDK's `REGS` command reports a CPU mode but documents only PC/A/X/Y/S/P;
its documented commands do not provide the full native register set, direct
IRQ/NMI pin injection, or individual CPU-cycle stepping. Enabling the Rapidus
device alone still reported `mode=6502` immediately after its paused reset.
That observation concerns this bridge configuration, not AltirraSDL's ability
to emulate the 65816.

An equivalent automated cross-check still needs an explicit 65816 setup and
access to the required state and interrupt controls. The NMI/reset reproducers
above have **not** been executed in AltirraSDL. No changes were made to the
installed bridge or the user's emulator settings.

Recorded locally on 2026-09-15: macOS ARM64, Rust 1.95.0. The qualification
target passed all 24 tests in debug and release builds; explicitly running the
two limitation reproducers in debug failed both as described above. Remote
platform CI was not run for this probe.
