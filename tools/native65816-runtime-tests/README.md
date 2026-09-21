# Emitted native 65816 execution

This isolated workspace tests the
[native emitter](../../docs/MIR65816_EMISSION_CONTRACT.md) and
[context bridge](../../docs/MIR65816_CONTEXT_INTERFACE.md). It loads serialized
JSON images and experimental o65 applications and executes their bytes on the
VM's independent native 24-bit bus.
The compiler does not depend on the VM.

An opt-in [actionc/vbcc code-quality comparison](../compare65816/README.md)
executes paired C and Action! kernels from external build artifacts. Its
`code_quality` test is ignored by default; it reports incorrect external compiler
outputs as failures and saves the complete measurements for analysis.

## Reproduce qualification

Install Rust, Python 3.12+, git, ca65 and ld65. From the actionc repository root:

```sh
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
python3 tools/native65816-runtime-tests/qualify.py --cpu
python3 tools/native65816-runtime-tests/qualify.py --test comma_groups --test memory --test interop
python3 tools/native65816-runtime-tests/qualify.py --test o65
```

The runner uses VM base `56ddc5c5de41f0e7294e87c440869550eaf53292` plus
[`vm-status-timing.patch`](vm-status-timing.patch), exported from actionc-vm
commit `da81c1e`. REP/SEP/RTI timing corrections are necessary for asynchronous
qualification. It archives the pinned commit from a sibling VM repo when
available, otherwise fetches that exact base into a private temporary checkout.
It checks/applies the patch and verifies cached CPU file hashes on reuse.
It never edits the sibling VM's working tree.

Use this runner instead of bare `cargo test`: the lockfile expects the corrected
CPU path override supplied by the runner. The published VM dependency remains
pinned; the independent VM commit need not already be published. `--prepare-only`
prints the qualified checkout, `--cpu --release` checks its CPU release build,
and other arguments are forwarded to `cargo test`, for example
`--test preemption -- --nocapture` to print instruction-address counts.

The assembler is invoked directly; missing tools fail the tests. No Atari ROM,
OS, device intercept or host scheduler participates. The earlier
`vm65816-runtime-tests` workspace remains the jgenesis comparison experiment.

## Corpus

| Target | Tests | Coverage |
| --- | ---: | --- |
| `arithmetic` | 1 | 72 boundary executions across BYTE/CARD/INT/SIZE/LONGCARD/LONGINT, checked against host arithmetic. |
| `execution` | 5 | Recursion, mutable parameters, loop edges, local addresses/descriptors, record strides, banked code/data and exact volatile byte access. |
| `interop` | 1 | Hand-packed mixed ABI arguments and zero-argument padding, calls both ways, A/X results, unused bits and all 64 scratch bytes clobbered; both I states. |
| `comma_groups` | 1 | Scalar comma groups before contextual types in parameters and fields; mixed-width values, a bank-crossing record, LF/CRLF and both I states. |
| `indirect` | 3 | Targets `$050000`/`$06FFFF`, all scalar results, assembly arguments and six-byte transfer overflow checks. |
| `contexts` | 6 | First-task bytes, yield/exit, full register/flag restoration in every M/X mode, invalid COP/domain paths and NMI through IRQ transition windows. |
| `pointer_allocation` | 2 | Generated/reference unlink; differential stack/DP swaps, chains, field layouts and pressure; mixed arguments, bank crossings, aliasing, exact traces and LF/CRLF. |
| `pointer_preemption` | 2 | Both tasks and IRQ dispatch use the same three-slot leaf; IRQ at 164 raw / 100 optimized task/instruction sites, plus seeded IRQ/NMI. |
| `memory` | 8 | Pointer results and bank-crossing unlink, field offsets around the Y limit, exact volatile three-byte traces, absolute array indices, logical shifts, record/overlap copies and signed/wide pointer offsets. |
| `effects` | 1 | Nested IRQ tokens, pending IRQ, protected multiword writes, polling/reloads and exact volatile traces under optimization. |
| `preemption` | 2 | Two live recursive contexts and shared memory helpers; every reached enabled instruction address plus two seeded IRQ/NMI schedules. |
| `stack_allocation` | 3 | Measured scalar/loop/recursive/indirect call chains with stack ceilings; a long sequence beyond the old allocation limit; live wide values across direct/indirect assembly calls clobbering all DP scratch and A/X/Y. |
| `stack_faults` | 2 | Floor/ceiling/underflow and call transients, with raw fault A/X/S state verified before prohibited writes. |
| `o65` | 7 | Serialized files loaded at two placements; bank carries, BSS, aliases, initialized split/full addresses, moved imports/faults, mixed ABI, multi-bank code and preempted tasks. |

Both raw and optimized NIR are covered. The original **33 tests passed in debug
and release** on 2026-09-17. The later `comma_groups` regression, plus the eight
`memory` tests and `interop`, pass in debug on 2026-09-20; this targeted run does
not claim a new full-suite or release qualification. Local tools: Rust 1.95.0,
ca65/ld65 2.18, macOS ARM64.
Optimized unlink uses 129 bytes, 189 VM cycles and no frame, including checked
entry and RTL; the independent reference uses 127 bytes and 200 cycles. The
[qualification record](../../docs/abi/action65816-pointer-allocation-qualification.json)
binds the 2026-09-17 results to compiler/fixture hashes and context artifacts. Images
use transport v3 and retain physical ABI v1.

The [stack allocation investigation](../../docs/MIR65816_TEMPORARY_ALLOCATION.md)
records the `90bd73e` baseline and reductions from CFG-aware temporary reuse.
Its [2026-09-21 qualification](../../docs/abi/action65816-stack-allocation-qualification.json)
passes all 37 native tests in debug and release, with unchanged ABI and guards.
Run `--test stack_allocation -- --nocapture` to print emitted code sizes, VM
cycles, frames and the observed stack use across complete call chains. Inputs
are supplied after compilation, and both raw and optimized images are executed.

The [o65 qualification](../../docs/abi/action65816-o65-qualification.json) passes
all 44 native tests in debug and release on 2026-09-21. The o65 adapter consumes
only serialized file bytes, placement and provider contracts; compiler results
are discarded before loading. It preserves the JSON harness's guards and bus
permissions. Code is placed at `$100000` and `$600000`, with independently moved
data/BSS and helper/fault addresses. The o65 context test uses two seeded IRQ/NMI
schedules and six selected reachable instruction addresses per mode/placement;
the existing exhaustive JSON context tests also remain in the full suite.

The corrected CPU suite passes eight tests in each build mode. See
[initial Exec acceptance](../../docs/MIR65816_EXEC_ACCEPTANCE.md) for G1–G6,
compiler regressions and qualification limits.

## Interrupt schedules and memory ownership

The two-task fixture uses task domains `$2000`/`$2100`, task stacks
`$4000..$4FFF`/`$5000..$5FFF`, IRQ domain `$2300` and IRQ stack `$6000..$6FFF`.
The assembled bridge starts at `$008000`; emitted code starts at `$018000`,
data at `$120000`, fault handling at `$048000`, and explicit IRQ/NMI/exit
acknowledgements at `$7800..$7803`. Each image/layout records exact extents.
These are test reservations, not an Atari board memory map.

The baseline corpus reaches 2,504 raw and 2,352 optimized distinct enabled
instruction addresses. At each, a separate run holds IRQ until assembly dispatch
acknowledges it, then checks output, guards and domain storage. Seeded runs use
`0x81620260916` and `0x5eedcafe`; NMI pulses are separated by at least 250 cycles.
The pointer fixture additionally injects once per reached `(task domain, PC)`
inside its leaf, so both tasks are checked even when they share instruction
addresses. IRQ dispatch calls that same leaf using the IRQ domain's scratch.
IRQ masking and the bridge's non-nested NMI policy are respected. Each run has a
finite cycle budget; unmapped accesses and code writes fail immediately.

The simpler call probes use an independent caller at `$040000`, a bootstrap
domain at `$002000` and a guarded stack in `$004000..$005FFF`. Assembly layouts
and host expected results are independent of compiler IR/layout helpers.

## Saved artifacts

Successful native runs create separate `target/qualification/run-*/` directories.
The manifest records compiler revision, source/fixture hashes, VM base and patch
hash, assembler/linker/Rust versions, command, seeds and artifact hashes. Context
runs save `.act`, `.a816.json`, `.bridge.bin` and `.layout.json` files. Filtered
runs contain only artifacts produced by the selected tests; they cannot inherit
stale images from an earlier run.

The o65 tests save `.o65`, `.placement.json` and `.metrics.json` artifacts.
Metrics separate machine code, text, data, BSS, descriptor and total file sizes,
and include relocation counts and cycles. Observed stack use is recorded for
the pointer/call and guard-failure probes; `null` means it was not measured.
The qualification record checks identical file hashes across placements and
identical artifacts between debug/release host builds. Decode the standard wire
records with `python3 tools/inspect_o65.py PATH/TO/PROGRAM.o65`.

Inspect an emitted image from the repository root:

```sh
python3 tools/disassemble65816.py PATH/TO/IMAGE.a816.json > image.asm
```

The disassembler supports the qualified emitted instruction subset and rejects
unknown/truncated encodings. Preserve assembler listings for external bridge
code. The [CPU checkpoint](../../docs/MIR65816_CPU_EXECUTION_CHECKPOINT.md) and
[WDC datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf)
state CPU provenance and the hardware boundary. Emulator acceptance is followed
by a custom-board startup/interrupt smoke test.
