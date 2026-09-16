# Native 65816 acceptance for initial Exec work

Date: 2026-09-16. **G1–G6 pass for the initial subset described here**, using
`wdc-65816-native`, ABI `action65816.native.v1`, image format version 2, and
both raw and optimized NIR. Ordinary standalone Exec implementation can begin
on this boundary. The next platform milestone is a real-machine bootstrap and
interrupt smoke test; these results establish emulator execution only.

This result completes slices 5–7 of the
[implementation plan](MIR65816_IMPLEMENTATION_PLAN.md) against the
[readiness requirements](MIR65816_EXEC_READINESS_REQUIREMENTS.md). It includes
no scheduler, allocator, IPC system, device drivers or GEM integration.

## Supported compiler boundary

The [emission contract](MIR65816_EMISSION_CONTRACT.md) is the source of truth for
source operations and output. The initial kernel subset includes:

- 8/16/24/32-bit integer storage, arithmetic, casts, comparisons, bit operations
  and logical shifts; branches, loops and early returns.
- Records and arrays in memory, natural field/element layouts, addressable
  automatic storage, per-entry initialization and explicit 24-bit pointers.
- Direct and typed far indirect calls, scalar/pointer results, mutable value
  parameters, recursion and mutual recursion.
- Ordinary aggregate copies and the reentrant `A816MEMORY.Move`/`Clear` helpers,
  including overlapping movement and bank crossings.
- Scalar volatile accesses, conservative call barriers, and assembly
  save/disable/restore IRQ primitives with nested tokens.
- Checked native frames, assembly imports/exports, distinct code/read-only/
  writable/zero-fill placement, allocated frame maps and disassembly.

[Physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md) and the
[context interface](MIR65816_CONTEXT_INTERFACE.md) define stack/domain ownership,
first-task fabrication, task exit, IRQ/COP dispatch and bounded assembly NMI.
The initial frame limit is 254 bytes including spills; every incoming/outgoing
stack-relative byte must also fit displacement 1..255. Each routine reports a
local peak. Recursion and indirect depth have no inferred whole-task bound.

## Gate evidence

The [independent execution workspace](../tools/native65816-runtime-tests/README.md)
loads serialized compiler images and assembles handwritten code with ca65/ld65.
The VM executes machine bytes. Expected results come from literal assembly
layouts, host integer calculations and explicit memory expectations.

| Gate | Executed evidence |
| --- | --- |
| G1 | `arithmetic`, `execution`, `memory`: all scalar widths, signed boundaries, shift counts, natural record/array layout, initialized local descriptors, full-width pointer offsets including negative INT and SIZE above 64 KiB, overlap-safe movement/clear across banks, exact memory and guards. |
| G2 | `interop`, `indirect`, `contexts`: assembly calls both ways, mixed arguments, all scalar results, targets `$050000`/`$06FFFF`, scratch clobbers, stack balance, exact 19-byte first-task image, yield and return/exit. |
| G3 | `execution`, `memory`, `preemption`: per-entry arrays, mutable parameters, recursion/mutual recursion, escaped live local addresses, two simultaneously live contexts entering the same routines and memory helpers. |
| G4 | `preemption`: IRQ asserted at 2,942 raw and 2,716 optimized distinct reachable enabled instruction addresses. `contexts`: full A/X/Y/S/D/DBR/P/PC/PBR restoration for every M/X combination, hidden B, status flags, NMI while masked and through IRQ stack/domain transitions. Seeded IRQ/NMI runs cover both modes. |
| G5 | `effects`: nested tokens with I initially clear/set, IRQ held pending across a multiword update, normal and volatile values reread after dispatch, exact `write/read/write/read` byte MMIO trace and unmapped neighbors. |
| G6 | `stack_faults`, compiler emission/CLI/ABI tests: faults before prohibited writes, frame/argument limits, bad placement/relocations/imports, image round trips, separate section origins and allocated frame maps, explicit unsupported-operation diagnostics. |

IRQ injection visits each distinct enabled instruction address reached by the
bounded corpus's baseline execution, once per optimization mode. It does not
enumerate every possible dynamic occurrence or program. Each selected run holds
IRQ until the assembled dispatcher acknowledges it, and then checks completion,
results, stack guards and domain ownership. All switching is performed by the
assembly bridge, never by host register replacement during an injected run.

Seeded schedules use `0x81620260916` and `0x5eedcafe`, with at least 250 CPU cycles
between NMI assertions. The bridge performs no nested NMI. The platform must
provide equivalent gating; masking IRQ does not mask NMI.

## CPU qualification and reproducibility

The VM base is `56ddc5c5de41f0e7294e87c440869550eaf53292`, with the REP/SEP/RTI
status-timing corrections committed separately in actionc-vm as `da81c1e`.
The checked-in [patch](../tools/native65816-runtime-tests/vm-status-timing.patch)
reproduces that CPU change without requiring publication of the VM commit.
Its SHA-256 is
`afd5efebc9f0d153ef928c953af657af24cc5f059a1dca7482e3a3fa325f4cac`.

Eight CPU tests pass in debug/release, including the active timing regressions,
the independent 1,610-case instruction ROM on the corrected production core,
and the original C equivalence corpus under an explicit test-only compatibility
mode. The latter preserves comparison against the original timing; it is not
used for compiler execution. All 30 native VM CPU/bus tests also pass.
See the [CPU checkpoint](MIR65816_CPU_EXECUTION_CHECKPOINT.md) for scope.

From the actionc repository root, with Python 3.12+, Rust, ca65 and ld65:

```sh
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
python3 tools/native65816-runtime-tests/qualify.py --cpu
```

Each successful native run stores a manifest under
`tools/native65816-runtime-tests/target/qualification/run-*/`. It records compiler
revision and source/fixture hashes, VM base/patch, tool versions, command, seeds
and artifact hashes. Context tests also save source, the linked image, assembled
bridge and memory layout. The checked-in
[acceptance record](abi/action65816-native-v1-qualification.json) identifies the
two-context images and the compiler source tree used for this result.

Local toolchain: Rust 1.95.0, ca65/ld65 2.18, macOS ARM64. Results:

- All 24 native execution tests passed in debug and release.
- NIR snapshots and all 51 NIR fixtures passed without snapshot changes.
- Full compiler suite: 3,243 passed, 24 ignored, two existing TN sample failures.
  Validation used an isolated checkout excluding unrelated user edits/programs.
  The failures are `sample_catalog_classifies_every_action_source` (four TN
  sources missing catalog entries) and `parses_all_sample_programs`
  (`LOCATION.ACT` lacks imported DIR constants when analyzed alone).
- Emission, CLI and shared-NIR regressions passed, including rejection of
  volatile aggregate copy in both optimization modes. Artifact export and
  disassembly were checked against current version-2 images.

## Limits retained for initial Exec

- Use explicit pointer accesses for banked MMIO. Numeric bare declaration
  aliases above bank zero are diagnosed; the legacy resolver is still 16-bit.
- Multiplication/division/remainder, REAL, by-value aggregate interfaces,
  small-model emission and unsupported foreign/runtime operations remain
  diagnosed. Record address scaling does not require a multiplication helper.
- Whole volatile aggregates require an explicit byte protocol and are rejected.
  Wider scalar accesses use ordered bytes and are not atomic. Critical sections
  protect against IRQ only; NMI and hardware agents require their own policy.
- NMI is bounded, assembly-only and non-switching. IRQ dispatch has its own
  domain/stack and cannot block, yield or enable IRQ. No MVN/MVP is emitted.
- ABORT is not qualified. Complete external CPU conformance and custom-board
  timing, reset, vectors, acknowledgement and memory mapping remain platform
  validation. Replacing the bridge's NMI body requires new qualification.
- The platform owns nonoverlapping bank-zero reservations, static/zero-fill
  loading, native startup, task/resource lifetimes, stack sizing and the raw
  nonreturning fault/exit adapters. Image maps report costs, not a scheduler.
