# Emitted native 65816 execution

This isolated workspace tests the
[native emitter](../../docs/MIR65816_EMISSION_CONTRACT.md) and
[context bridge](../../docs/MIR65816_CONTEXT_INTERFACE.md). It loads serialized
JSON images and executes their bytes on the VM's independent native 24-bit bus.
The compiler does not depend on the VM.

## Reproduce qualification

Install Rust, Python 3.12+, git, ca65 and ld65. From the actionc repository root:

```sh
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
python3 tools/native65816-runtime-tests/qualify.py --cpu
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
| `indirect` | 3 | Targets `$050000`/`$06FFFF`, all scalar results, assembly arguments and six-byte transfer overflow checks. |
| `contexts` | 6 | First-task bytes, yield/exit, full register/flag restoration in every M/X mode, invalid COP/domain paths and NMI through IRQ transition windows. |
| `pointer_allocation` | 1 | Generated/reference unlink comparison, code/cycle budgets, bank crossings, aliased neighbors, exact traces and LF/CRLF inputs. |
| `memory` | 8 | Pointer results and bank-crossing unlink, field offsets around the Y limit, exact volatile three-byte traces, absolute array indices, logical shifts, record/overlap copies and signed/wide pointer offsets. |
| `effects` | 1 | Nested IRQ tokens, pending IRQ, protected multiword writes, polling/reloads and exact volatile traces under optimization. |
| `preemption` | 2 | Two live recursive contexts and shared memory helpers; every reached enabled instruction address plus two seeded IRQ/NMI schedules. |
| `stack_faults` | 2 | Floor/ceiling/underflow and call transients, with raw fault A/X/S state verified before prohibited writes. |

Both raw and optimized NIR are covered. All **29 tests passed in debug and
release** on 2026-09-16. Local tools: Rust 1.95.0, ca65/ld65 2.18, macOS ARM64.
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
