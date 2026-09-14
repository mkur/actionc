# MIR68K minimal Amiga executable

Status: implementation in progress. The plan was committed as `d72ae47`.
Baseline: `ac64b98`, after
the [public CLI and benchmark milestone](MIR68K_CLI_AND_BENCHMARK_IMPLEMENTATION_PLAN.md).

Slice 1 is complete. External declarations now carry a verified service ID;
runtime selection checks the supported SYS signatures by ID. Native preparation
is separate from linking, and the four classic library adapters execute through
the bounded OS shim with captured narrow values and preserved native registers.
The existing SYS path uses external RoutineIds, so the new identity lives on
the routine-entry fact; it does not replace ordinary call-site identities.
No printed fixture changed. NIR snapshots, all 51 NIR sweep fixtures, the full
root `cargo test` in an isolated checkout, and all 82 native tests pass.
The adapter offsets/register maps were checked against the official NDK 3.2
rev4 archive. HUNK output and Amiga startup/console composition remain next.

Deliver an Action! program that compiles through MIR68K to one relocatable
Amiga executable, prints text and integers from the Shell, and returns cleanly.
Preserve the existing bare-image/r68k workflow and the internal native ABI.

## Milestone decisions

The initial acceptance platform is an original MC68000 with AmigaOS 3.1,
launched from the Shell in the locally available vAmiga. This is the planned
compatibility floor; it becomes a
support claim only after validation. Older releases need separate checks. Use only
classic 68K library calls, with no 68020 instructions or AmigaOS 4 interfaces.
The acceptance setup supplies at least 1 MiB RAM and a 64 KiB command stack.
Measure the samples' actual stack usage rather than treating this stack setting
as a compiler guarantee for arbitrary programs.

The proposed public command is:

```sh
actionc --target motorola-68000 --runtime amiga \
  -o build/hello.amiga samples/amiga/hello.act
```

`--runtime amiga` selects Amiga startup, runtime adapters and HUNK output.
The output is a single executable; `.amiga` is a naming convention, not a loader
requirement. The default output name is `<source-stem>.amiga`. Bare remains the
default for `--target motorola-68000`. An explicit `--origin`, source ORG or
Atari SET origin is invalid for the Amiga path: the OS chooses load addresses.
These commands and sample paths are planned, not available at this baseline.

Keep the existing parameterless PROC program entry. Normal completion returns
status 0 to the Shell. Startup failure, console failure and a typed runtime
fault return status 20 after cleanup. Fault messages retain the specific
Action! reason; they do not reuse Atari Error numbers. User-selected exit codes
and argument parsing can follow later without changing ordinary routine calls.

The first public console surface is the existing `SYS` subset: `Put`, `PutE`,
`Print`, `PrintE`, `PrintB`, `PrintBE`, `PrintC`, `PrintCE`, `PrintI` and
`PrintIE`. This covers strings, BYTE, CARD and INT without introducing language
syntax or expanding every Atari runtime binding. LONGINT/LONGCARD formatting
is a follow-up; the benchmark smoke test below has a checksum that fits CARD.

Workbench startup, graphics, input, general file APIs, callbacks, interrupts,
resident commands, dynamic linking, C interoperability, ELF, REAL arithmetic,
multidimensional arrays, a classic 68K backend and broad optimization are outside
this milestone. Do not ship a Workbench icon or claim Workbench compatibility.

## Inspected baseline and ownership

- `src/compiler/native.rs` loads sources/modules, selects reachable SemIR,
  verifies/optimizes NIR, materializes MIR68K, and immediately links at a fixed
  origin. Its public result contains an absolute `NativeImage`.
- `src/mir68k/machine.rs` already has typed code/data targets and original-68000
  instructions, including displacement-based JSR. `encode.rs` resolves symbolic
  operands while encoding; `image.rs` resolves initializer relocations as well.
  Neither exposes an object with retained section-relative fixups today.
- Native data distinguishes aliases, backing, descriptors, initialized bytes
  and zero-fill. The bare image's symbols and version-2 artifact use absolute
  or frame-relative locations. They must not be reinterpreted as relocatable.
- `NirRuntimeBinding` and `RuntimeSymbolId` already represent declared services.
  MIR68K carries those IDs but materialization rejects runtime calls. The Atari
  `Runtime` enum and `runtime_bindings.rs` assume cart/standalone implementations;
  adding an Amiga value there mechanically would spread Atari assumptions.
- `src/mir68k/runtime.rs` currently maps typed faults to TRAP #14. That is a bare
  harness transport and cannot serve as Amiga process termination.
- `tools/vm68k-runtime-tests` executes public artifacts with r68k and guarded
  memory. Its current completion trampoline and register checks describe the
  internal Action! ABI, not an Amiga Shell entry.

SemIR continues to own callable meaning and source typing. NIR carries verified
service identity, signatures and conservative effects. The compiler/runtime
selection layer binds supported interface declarations to typed service IDs;
MIR68K receives those bindings and never identifies services by printed names
or looks back into SemIR. MIR68K owns adapter instructions and register usage.
Emission owns section placement, fixups and HUNK serialization. The VM owns
test loading and OS-call simulation.

## Delivery order

Implement **1 → 2a → 2b → 3a → 3b → 4**. The sub-slices separate linker changes
from format writing, and process lifetime from console formatting. Each ends
with an executable or independently checked result. Commit the plan first when
implementation is authorized, then commit each completed sub-slice with its
validation. Start final CI without waiting for it to finish.

| Slice | Reviewable result | Principal checks |
| --- | --- | --- |
| 1 | Typed platform service/adaptor boundary | Signature, register and effects checks in r68k |
| 2a | Section-relative native object and bare-link consumer | Relocation inventory and unchanged bare execution |
| 2b | Bounded HUNK writer and independent test loader | Byte fixtures, invalid inputs and separate load bases |
| 3a | Shell startup, cleanup and terminal fault path | Resource/stack lifetime and failure injection |
| 3b | Public Amiga CLI and console subset | Compiler subprocess to HUNK to captured output |
| 4 | AmigaOS smoke tests, documentation and CI integration | Real Shell execution plus cross-platform regression coverage |

## Slice 1: Platform runtime and adapter boundary

Introduce explicit native runtime selection, conceptually `Bare` and `AmigaDos`,
without changing the existing Atari runtime defaults. Keep the public bare
compile API working. Add an Amiga compile entry/options/result as needed; factor
the existing frontend preparation only enough to avoid a second semantic/NIR
pipeline. An Amiga result must not contain a fictitious absolute NativeImage.

Define a small compiler-owned table of platform services with stable IDs,
verified signatures, memory effects and terminal/returning behavior. Bind only
the supported SYS console declarations. Private startup/write services need not
become source-level public functions. Unsupported reachable services produce a
diagnostic naming the declaration and selected runtime; unused SYS declarations
do not require implementations. Never route an unrecognized service through a
guessed address, the Atari cartridge or a stringly MIR builtin.

Keep ordinary Action! arguments in their existing stack slots. Generate target
adapters that marshal captured arguments into the OS registers, invoke the
library vector, and translate results back. Use typed platform routine/data
identities distinct from user RoutineIds; include them in target verification
and relocation accounting. Check the actual classic SDK vector offsets and
register contracts in one audited constants table before using them.

The existing ABI preserves D2–D7/A2–A5 and restores A6/A7. In particular, loading
DOS arguments into D2/D3 is itself a clobber that an adapter must undo. Capture
stack arguments before replacing the Action! A6 frame pointer with a library
base; restore the frame pointer before frame-relative accesses. Preserve values
across calls using the documented ABI, not assumed spare registers. Model the
full instruction/CCR effects so forwarding and allocation cannot cross an OS
call incorrectly.

The initial foreign calls are Exec OpenLibrary/CloseLibrary and DOS Output/Write.
Their classic argument/result registers are documented in the official
[OpenLibrary](https://developer.amigaos3.net/autodocs/exec.library/OpenLibrary.html),
[CloseLibrary](https://developer.amigaos3.net/autodocs/exec.library/CloseLibrary.html),
[Output](https://developer.amigaos3.net/autodocs/dos.library/Output.html) and
[Write](https://developer.amigaos3.net/autodocs/dos.library/Write.html) autodocs.
Keep library-base/LVO details in this target boundary, not in NIR signatures.

Add a small test-only OS shim to the existing r68k workspace. Install library
vector stubs in protected synthetic memory and intercept only their host-service
entry points. Execute the compiler-generated adapter instructions normally;
do not intercept SYS calls and print directly from the host. The shim records
arguments, deliberately destroys permitted scratch registers/flags, and checks
preserved registers and argument evaluation order.

Acceptance: a typed adapter transfers a byte, word, long and pointer correctly;
values live across its call survive optimized and conservative codegen; missing
bindings and signature mismatches fail explicitly. Bare fault tests still use
their original transport. Update the execution boundary with the new contract.

## Slice 2a: Preserve relocations through native emission

Introduce a target-owned relocatable object between physical MIR68K and final
linking. It contains stable section IDs, initialized bytes, allocation extents,
alignment, typed symbols and explicit fixups. Object symbols are section-relative,
absolute or frame-relative. A fixup records its source section/offset, encoding
and width, target identity and checked addend. Source display names remain
metadata. Compiler-owned startup/adapter targets participate in the same model.

Have instruction encoding report relocation sites while writing operands.
Do not discover them by scanning bytes, subtracting a chosen origin, comparing
two linked images or parsing listings. Preserve initializer fixups for routine
pointers, array descriptors/backing, data pointers and aliases. Null and numeric
constants are not relocations merely because they resemble an address.

Use one code section, one initialized data section and an optional BSS section
for the first Amiga layout. Place all instructions, including startup/adapters,
before final code layout and branch relaxation. Relative branches are resolved
only within the code section; cross-section addresses use full-width fixups.
Keep complete objects contiguous: an initialized object with a zero tail stays
in DATA, with that tail materialized. Entirely zero-initialized objects can use
BSS. Do not split one array or record across independently allocated hunks.

The bare linker becomes a consumer that resolves all fixups to the existing
NativeImage. Keep section grouping a consumer layout policy so Amiga DATA/BSS
packing does not reorder the bare image's objects. Preserve its layout/metadata
contract and existing byte fixtures;
if an unavoidable layout change is proposed, isolate it and document it before
updating expectations. Keep JSON version 2 as the linked bare transport.

For the Amiga consumer, reject unsupported selected-byte/narrow relocations of
movable addresses, odd relocation sites, and source absolute placements that
require the loader to initialize fixed memory. Absolute external references
remain explicit and unrelocated. `ImageEnd` cannot mean the end of three
noncontiguous allocations: diagnose it for this platform until it has a defined
meaning. Reject unresolved targets and overlapping fixups before serialization.

Acceptance: code-to-code, code-to-data, data-to-code, data-to-data, descriptor,
alias and BSS references retain their identities/addends. Resolve an object at
multiple separated section bases and verify addresses and behavior. Exercise
negative addends, arithmetic overflow, nulls and unsupported encodings. Run
affected bare emission/artifact tests and the complete native execution suite
after the linker consumer changes.

## Slice 2b: HUNK executable output

Write a small deterministic serializer for HUNK_HEADER, HUNK_CODE, HUNK_DATA,
HUNK_BSS, HUNK_RELOC32 and HUNK_END. These records describe allocation sizes,
payloads and longword fixups; relocation sites must be even on the MC68000.
Use the official [AmigaDOS RKM, section 11.2](https://developer.amigaos3.net/sites/default/files/downloads/2024-10/Amiga_ROM_Kernel_Reference_Manual_DOS.pdf)
as the format reference. Pin the checked reference edition in tests/comments.

Keep the writer's accepted subset explicit: no overlays, HUNK_EXT, compact
relocations, shared resident code, debug records or requested CHIP/FAST memory
classes. Emit sizes in checked longword units with deterministic padding; keep
logical object extents separate from section padding. Bound relocation groups
conservatively (at most 65,535 sites per group), emit additional terminated
HUNK_RELOC32 records when needed, and test splitting large groups.
The in-place addend is relative to the target section, never a previous absolute
link address. Define entry at offset zero of the first code payload; reserve
the startup position now using a no-OS entry thunk until slice 3a.

Add a bounded, independent reader/loader in the VM test workspace, not a second
compiler linker disguised as a loader. Validate header counts/sizes, supported
records, payload bounds, target indices, even four-byte patch extents, duplicate
or overlapping patches, terminators and checked allocation arithmetic. Reject
unsupported records explicitly. Do not share serializer parsing decisions or
test expected bytes generated by the writer itself.

Load identical HUNK bytes into at least three deterministic noncontiguous layouts,
including a different ordering of data and BSS in memory. Zero BSS and apply
each relocation once. Keep all mappings inside the harness's address space and
reserved-memory rules; validate the original MC68000 address range separately.
Retain executable prefetch padding at the end of the code allocation.

Acceptance: independently specified byte fixtures validate the writer/reader;
the same emitted program executes at every placement without rebuilding.
Include a function-pointer call, an initialized array descriptor, an alias and
a BSS pointer. Truncated files and invalid fixups fail before guest execution.
The standalone executable does not need the compiler's symbol sidecar to load.

## Slice 3a: Shell startup, cleanup and faults

Emit an OS entry wrapper at the start of CODE and call the existing Action! entry
by its stable ID. Leave source-level top-level executable statements unsupported.
The Shell permits return through its supplied entry return address, with a
status result; cleanup remains the program's responsibility. Workbench startup
has a separate message lifecycle and is excluded here. See the official
[program startup reference](https://wiki.amigaos.net/wiki/Program_Startup).

The wrapper saves the entry registers and original stack state before entering
Action! frames. Obtain ExecBase through the documented classic entry mechanism,
open `dos.library` with the selected version floor, then acquire Output(). Store
only runtime-owned state in the executable's DATA/BSS. Check library-open and
missing-output failures before calling the source entry.

Use one cleanup path for normal return and runtime failure. Close an opened DOS
library exactly once, preserve the chosen exit status across that call, restore
the saved registers/stack and return to the Shell. Output() supplies a borrowed
handle: do not close it. Match each successful library open with a close, as
required by the [OpenLibrary](https://developer.amigaos3.net/autodocs/exec.library/OpenLibrary.html)
and [Output](https://developer.amigaos3.net/autodocs/dos.library/Output.html) contracts.

For a typed Fault, branch to a compiler-owned terminal adapter with the reason.
Restore the saved startup stack context rather than returning through abandoned
Action! frames. Never resume the failed call or perform its pending result store.
Attempt a short fault message only when output is available; failure while
reporting a fault must lead directly to cleanup, without recursive reporting.
Do not install exception vectors or use the bare TRAP #14 transport on Amiga.

Use the inherited command stack; automatic allocation or StackSwap is deferred.
The runtime state belongs to one loaded executable invocation. Resident reuse,
callbacks and concurrent invocation of the same loaded instance are unsupported.
Multiple separately loaded instances must not share host/runtime state.

Acceptance in r68k: normal exit, failed OpenLibrary, missing Output, nested-call
division fault, fault-report failure and repeated fresh launches. Verify balanced
Shell entry state, preserved registers, open/close counts, no close of borrowed
handles, status 0/20 and absence of stores after a fault. Poison caller memory
and use guards to catch stack restoration errors. Keep architectural CPU errors
and instruction-budget exhaustion distinct from Action! faults.

## Slice 3b: Console implementation and public CLI

Implement the SYS subset as normal Action! helpers where practical, backed by
one private byte-span output service and the target OS adapter. Keep decimal
formatting out of the compiler optimizer and HUNK writer. Use the existing
STRING representation and verified static-data facts; do not reinterpret it as
a NUL-terminated C string or assume Atari pointer width.

`Print` writes the counted payload without its length prefix. `Put` writes the
specified byte. `PutE` writes LF (10); `PrintE` and numeric E variants append LF.
Other payload bytes pass unchanged, including NUL and values above 127. There
is no implicit ATASCII conversion. Document printable ASCII plus explicit E
routines as the first portable console examples; source string storage and
Atari console behavior remain unchanged.

The output service must consume partial writes, advance the byte pointer/count,
and stop on a negative result or zero progress with bytes remaining. No call is
needed for an empty span. Guard signed DOS lengths when accepting a native SIZE.
DOS explicitly permits short writes; checking only for -1 is insufficient.
See [Write()](https://developer.amigaos3.net/autodocs/dos.library/Write.html).
An output failure enters the status-20 cleanup path without recursively printing.

Test exact decimal output for BYTE/CARD limits and INT minimum/maximum, zero and
negative values. Handle INT minimum without an overflowing signed negation.
Test empty/max-length strings, byte-exact output, repeated calls, partial writes,
zero progress and failure after a successful prefix. Run optimized and raw NIR,
with a focused independent conservative-codegen adapter case.

Connect `--runtime amiga` to the new API/HUNK writer and add focused configuration
tests. Preserve native source annotations, explicit-option precedence, module
paths and diagnostic mapping. Reject Atari modes/runtimes, incompatible backends
and origin controls clearly. Keep `actionc-emit` NIR/listing inspection working;
Amiga maps use section+offset locations, not invented absolute addresses. The
listing remains inspection text. Automatic `actionc-run` integration is deferred.

Publish the single HUNK output through staged file replacement. Reuse existing
source/include/module collision protection for executable and sidecar outputs,
and leave an existing output intact on failure. Test paths with spaces, a
different working directory, and actual LF/CRLF source/include loading.

Acceptance: invoke the real compiler subprocess once per configuration, remove
source, move the output, load only its HUNK bytes and run startup/console/cleanup
through the OS shim. Assert exact output and return status. The shim must observe
the real library adapters; a source interpreter or direct host SYS substitution
does not count.

## Slice 4: Real AmigaOS acceptance and integration

Add three small samples under `samples/amiga/`:

1. `hello.act`: a greeting and integer output through the supported SYS API.
2. `integer-array.act`: signed arithmetic, CASE/loops, static/automatic arrays
   and a function pointer, ending in a deterministic report.
3. `insertsort.act`: a driver around the unchanged shared insertion-sort kernel,
   printing all ten sorted values, checksum 65 and the existing CheckResult().
   Check every element in the driver/test; the checksum alone cannot prove sorting.

Add a separate division-by-zero probe for nonzero exit and clean Shell recovery.
Keep benchmark sources/reference vectors unchanged; only the Amiga driver adds
console reporting. Use compiler layout metadata for host inspection, never
addresses chosen for the old 6502 fixtures.

Use the locally installed vAmiga for real AmigaOS acceptance. Inspection on
2026-09-14 found `/Applications/vAmiga.app`, version 4.5, build 260807. This
confirms emulator availability; the active CPU configuration and booted
Kickstart/AmigaOS versions still need verification during setup.

Run the exact public-CLI output in vAmiga booted into the selected AmigaOS
version. Prepare a dedicated test configuration using a copy of the existing
OS setup, preserving the user's saved machines and disk images. Verify the
MC68000, memory and command-stack settings before recording results. Record
the actual vAmiga version, configuration, Kickstart/OS versions and executable
transfer procedure so another run can reproduce the test. Check available
automation during setup; a documented manual smoke run is sufficient for this
milestone. Keep vAmiga integration in validation tooling, outside the compiler.
Do not bundle ROMs or OS disk images in the repo.

Provide a reproducible Shell script that sets the stack, runs each executable,
captures output and records return status immediately after each command. Test
both console output and file redirection, repeated launches, and a command run
successfully after the deliberate fault. Treat an OS failure or missing success
marker as a failed smoke run, even if an emulator process itself exits normally.

The mandatory Linux/Windows/macOS tests use r68k and the bounded OS shim without
ROM dependencies. Real AmigaOS validation is a separate explicit integration
check, manual if necessary; do not make the default CI silently skip its compiler
or runtime coverage when no OS image is installed. A passing shim does not prove
compatibility with the real loader/libraries. If OS setup is unavailable, report
that remaining acceptance item and do not mark the milestone complete.

Update README/USAGE, the native runner guide, execution boundary and a focused
Amiga usage page. Document supported calls, string/newline behavior, minimum
tested OS, stack setup, build/run commands, return codes and unsupported launch
modes. Keep examples beginner-readable and implementation details in developer
documents. Add links in the documentation index.

## Validation and completion

Choose local checks from each slice's affected consumers. Adapter-only work
needs focused runtime/ABI tests; linker changes need emission/artifact coverage
and the native suite; CLI integration needs native and representative Atari CLI
regressions. New formatting/library code needs its own boundary cases. Do not
rerun every 6502 benchmark for documentation or Amiga-only library edits.

Any change to shared semantic lowering, NIR, verifier/printer or their contracts
requires the repository checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Run the complete native workspace after integrated target changes:

```sh
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml
```

Keep the existing CI coverage and compiler-subprocess helper. Batch cases per
compiled program; do not rebuild per load address or fault injection. Normalize
host CRLF before newline-sensitive instrumentation, exercise both conventions
through the real path, and compare HUNK/console bytes exactly. Protect unrelated
worktree changes; use an isolated checkout if an unrelated sample prevents a
required broad check. Check documentation links and whitespace for this plan.

Completion requires all six sub-slices, the single-file CLI workflow, successful
execution of identical bytes at independently chosen section bases, correct
resource and fault cleanup, exact sample output under real AmigaOS, and passing
affected local checks. Record compiler revision and validation environment.
Dispatch full CI for the final commit and report its actual status without
waiting; pending CI is pending evidence, not a local failure or a claimed pass.
