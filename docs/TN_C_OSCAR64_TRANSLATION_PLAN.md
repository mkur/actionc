# TN C translation and Oscar64 comparison plan

Status: implementation in progress, prepared 2026-11-10. Slice 0 is qualified;
application translation follows. Translate the maintained TN application into C, produce a
working Atari program with Oscar64, and use verified output to guide actionc
optimization. Implement in vertical slices and commit each qualified slice.

## Scope and frozen reference

Port `samples/tn/modern/TN.ACT`, its shared `LIB.ACT`, and subsequently the
additional behavior in `TNDBG.ACT`. Preserve the current single-directory
implementation. The batch/paging redesign belongs to `tn-ng` and is outside
this port; the [sample README](../samples/tn/README.md) describes that boundary.

Start from actionc/source revision `d11fe9ca20edfc1975489453d39bf0364c3795ff`.
Freeze a complete include/input manifest, compiler versions and binary hashes,
ROM and MyDOS image hashes, options, VM revision and generated artifacts before
translation. Record entry sources and included files individually. Rebuild
cartridge and standalone MIR6502 references with that pinned compiler; use
standalone as the primary comparison for included runtime costs. Keep cartridge
results as a separately labelled deployment comparison.

The existing [pointer-retention scorecard](benchmarks/6502-pointer-retention/results.json)
records cartridge TN/TNDBG XEX sizes of 9,891/12,987 bytes. Those are reference
measurements, not a claim about standalone size or the future C port.
Select and pin an exact Oscar64 revision in slice 0. The earlier
[Mandelbrot comparison](OSCAR64_MANDELBROT_CODEGEN_COMPARISON.md) supplies a
measurement precedent, not current TN results or a mandatory tool version.

Completion means a working full TN port and qualified TNDBG variant, a repeatable
build/validation workflow, and a report explaining the largest code-generation
differences. Initial kernels provide early evidence; they do not substitute for
the full application. Compiler optimization changes are subsequent work.

## Translation contract

Maintain a routine inventory mapping every Action! routine, machine block,
static data block and external dependency to its C implementation, adapter,
validation cases and completion status. Add source references to ported routines.
Translate the existing algorithms, routine boundaries and access order before
considering an optional idiomatic C variant.

| Source property | Required C treatment |
| --- | --- |
| `BYTE`/`CHAR`, `CARD`, `INT` | Explicit unsigned 8-bit, unsigned 16-bit and signed 16-bit roles; assert actual target widths, pointer size and little-endian representation. |
| Assignment, updates and arithmetic | Preserve truncation points, unsigned wrap, signedness, shifts and comparisons. Use unsigned intermediates where required; avoid C signed-overflow assumptions. |
| Expression evaluation | Introduce ordinary temporaries when needed to preserve Action! capture/evaluation order. C argument evaluation order must not choose a different result. |
| `FOR`, `WHILE`, `DO` and `EXIT` | Preserve entered versus header-tested behavior, inclusive bounds, early exits, byte wrap and post-loop counter values. Do not translate every `FOR` mechanically into a zero-trip C loop. |
| Local storage | Preserve initialized local persistence, including `InitPanels.times`, and other state read across calls. Audit address-taking, uninitialized reads and reentry before choosing C automatic versus static storage. |
| Arrays and pointer cells | Preserve fixed backing versus rebindable address descriptors, byte/word indexing, original capacities and aliases. `v` is a rebound word-address table; it is not interchangeable with an arbitrary host pointer array. |
| Strings and menus | Preserve counted strings, raw menu terminators, ATASCII `$9B`, screen codes, inverse bytes and exact literal bytes. Do not replace them with NUL strings or host/PETSCII text conversions. |
| Records | Assert `PanelState` size 9 and every field offset, `DirectoryRow` size 18 and every field offset. Use byte-backed accessors if target C layout cannot express those exact objects. |
| Hardware and OS cells | Bind exact addresses with access qualifiers justified by actual external mutability; preserve byte/word aliases and access order. Do not mark all TN state volatile. |
| Existing machine routines | Retain their functional algorithms and observable accesses through audited ABI adapters. Attribute assembly bodies and adapter overhead separately from compiled C. |

Do not replace `Sort`, string handling, the arena allocator or rendering with
`qsort`, libc strings, `malloc`, or new algorithms. Keep tables, counts, screen
geometry, MyDOS commands and menu/dispatch behavior unchanged. Avoid `restrict`,
forced zero-page placement of new hot state, page alignment or hand-written
assembly for formerly high-level code in the reference port.

### Equivalence and C's memory rules

The full port must match supported application behavior, including persistent
state, packed buffers, external aliases and error recovery. That does not imply
that ordinary C reproduces every numerically possible Action! memory access.
In particular, an incoming address may point into Action! parameter homes, and
Action! address arithmetic can wrap through `$FFFF`. Ordinary C automatic
variables and pointer arithmetic can give the optimizer stronger guarantees.

Record each such boundary explicitly. Use 16-bit address arithmetic with a
qualified target access adapter where the application requires it. For probes
requiring arbitrary wrap or parameter-home aliases, add a separate byte-backed
address-space/storage model when feasible; count its adapter cost and assumptions.
Do not claim that a native-pointer C result proves an Action! rewrite safe.
A host build must use bounded objects or an explicit memory model, not casts of
16-bit Atari addresses to host pointers. Host execution supplements target VM
execution and does not establish the target ABI or layout.

Any idiomatic C rewrite, stronger contract or manually tuned variant must be a
separate source/configuration with its own result rows. Never replace the
faithful reference silently to obtain better numbers.

## Platform and runtime design

Use Oscar64 native 6502 output for the Atari target. Its
[official manual](https://github.com/drmortalwombat/oscar64/blob/main/oscar64.md)
documents `-tm=atari`, `-n`, `-Os`, `-O3`, `-g`, and assembly/map/intermediate
listings. Verify the pinned compiler's actual startup, load format and ABI with
a small executed probe; target selection alone does not qualify a TN executable.
If necessary, supply a project-local startup and deterministic Atari XEX packer
for native segments. Check segment extents, entry vectors, initialization and
loaded bytes. Do not assume that renaming a PRG produces an Atari load file.

Before combining the program, qualify a complete RAM/zero-page reservation map:

- Program origin `$2C00`; window arena `$2000..$27FF`; source absolute cells,
  including `i/j` at `$E0/$E1`, `screen` at `$E6/$E7`, `allocp` at `$E8/$E9`,
  cursor/window state, machine scratch, IOCBs and MyDOS bindings.
- Oscar64 compiler registers, temporary/static frame storage, CPU stack,
  software stack, interrupt save areas, startup and any runtime helpers.
- Screen memory, panel backing and tables, all BSS/static homes, and the copy
  buffer derived from the end of reserved program storage. Translate
  `SET BUFFER=*` using linker-derived extents, never a hard-coded old address.
- Reserve software-stack and runtime space before publishing copy-buffer
  capacity against `MEMTOP`. The copy buffer must not consume compiler stack
  or heap reservations. Detect overlap and insufficient capacity explicitly.

Retain source-visible fixed cells. Relocate compiler-owned scratch or add a
small bridge where supported; never silently move a TN external binding.
If the pinned target cannot support the reservation map, report the platform
blocker before completing the application port.

Create a narrow platform boundary for CIO, keyboard input, screen access,
program entry/exit and nonlocal error recovery. Port Action! argument/result
handling to Oscar64's ABI; do not call an Action! machine body directly with C
arguments. Check actual register, flag, scratch, stack and interrupt obligations.
For inherited pure machine bodies such as `Internal`, retain a comparable
callable body/control where feasible and report compiler inlining separately.

`_Cio` can jump through `ErrorHandler`; `NavError` resets CPU SP and re-enters
application control. Prove recovery for both the CPU stack and any Oscar64
software-stack/frame state. Repeated errors from nested C calls must not leak
frames or return through abandoned frames. A restart trampoline or explicit
recovery loop needs a documented state/trace equivalence mapping; an ordinary C
return is not an adequate replacement. Preserve `Quit`'s cartridge/self-test
handoffs in the selected environment through qualified terminal adapters.

Use the same OS/MyDOS environment for matched full-program runs. Do not invoke
Action! cartridge services in the primary C standalone build. Keep direct OS
and inherited assembly costs visible; a stubbed-CIO test is not disk validation.

## Proposed implementation homes

| Path | Purpose |
| --- | --- |
| `samples/tn/c/` | Maintained faithful C port, shared definitions, byte data, platform/ABI adapters, TN/TNDBG entry configurations and build instructions. |
| `tools/compare6502/tn/` | Pinned tool/input manifest, reproducible build driver, native artifact/symbol adapters, workloads, validation and report commands. |
| `tools/vm-runtime-tests/tests/tn_c_reference.rs` | Proposed focused execution target consuming independently built C and Action! images through compiler-specific adapters. |
| `surveys/tn/c-oscar64/` | Tracked routine inventory, semantic differences, compact correctness/size/cycle reports and ranked findings. |
| `build/tn-c-oscar64/` | Ignored executables, compiler binaries, listings, maps, IR dumps, raw profiles and logs keyed by manifest/configuration. |

Keep test-only observation entries and mailboxes outside production program
logic. Separate instrumented qualification images from production-size images;
record their relationship. Reuse the pinned `actionc-vm` dependency already in
the VM test crate. Reuse TN workload data/oracles where applicable, but leave
Action!'s existing call harness intact: it assumes A/X/Y and `$A3` argument
placement and `$A0` results. Add an Oscar64-specific invocation/observation
adapter or a compiled C dispatcher with a runtime-populated mailbox.

Do not force production exports/no-inline simply to make map lookup convenient.
Diagnostic callable builds may preserve entries, while production builds retain
normal reachability/inlining. Missing inlined routine symbols mean a measured
region/call-chain result, not a zero-byte standalone routine.

## Implementation slices and acceptance

Each slice must have passing focused checks, an updated inventory/report and a
separate commit. Freeze accepted inputs/artifacts before changing the next slice.

### 0. Freeze references and prove the toolchain

Pin both compilers, all TN includes, flags and execution images. Build current
MIR6502 TN/TNDBG in cart and standalone modes. Establish Oscar64 `-Os -g` and
`-O3 -g` native profiles and execute a small Atari artifact from its real startup.
Verify widths, records, counted-byte data, byte/word argument/results and build
reproducibility. Produce the reservation map and initial routine inventory.

Acceptance: hashed manifests, load/startup/ABI probes pass, target listings and
IR are available, and no implicit C64 startup or extra-ROM assumption remains.

### 1. Establish TN storage and the platform bridge

Implement exact external bindings, packed layouts, arena state, strings/data,
CIO/keyboard adapters and inherited machine-call bridges. Spike nested nonlocal
error recovery and terminal handoffs before translating the main UI. Add the
compiler-specific VM adapter and a runtime-input mailbox for kernel probes.

Acceptance: map overlap checks, all conversion bytes, ABI/carry/stack guards,
repeated initialization, repeated nested error/reentry and terminal-path tests
pass. Hardware/OS boundary traces identify accesses that must remain observable.
Unresolved recovery semantics block full-port qualification explicitly.

### 2. Translate and qualify computation kernels

Port `Strcpy`, `Strcat`, `Instr`, `Fnamecmp`, `Sort`, `Convert`, `PrintFname`,
`Print`, and the `SetWin` conversion regions with their real state/callees.
Use ordinary source translations; measure inherited assembly and adapters
separately. Preserve both `SetWin` loop structure and global versus local
indices. Do not replace the production call graph with benchmark dispatch.

Acceptance: both Oscar64 profiles and MIR6502 pass independent expected-value,
full-buffer, input-preservation and termination checks. Cover empty/short/maximal
counted strings within routine preconditions, comparison equal prefixes with a
terminating difference, directory/protection markers, conversion byte values,
byte-index boundaries, carry and application-valid overlap. Record unsupported
raw-address cases separately. Publish an initial code/cycle gap report; kernels
alone are not the finished TN port.

### 3. Integrate directory and two-panel behavior

Port complete `SetWin`, panel backing, `Path`, drawing, tagging/predicates,
selection and initialization. Preserve 65 slots, 19-byte reads, 18-byte row
stride, 1,171-byte panel directory buffers, nine-byte panel state and path layout.
Use the maintained `main` fixtures rather than the `tn-ng` directory model.

Acceptance: adapt existing TN panel/directory oracles for 0/1/63/64-file
scenarios; check sorted rows, raw tag masks, exact operation filenames, guards,
both MyDOS directory bindings, same-panel/other-panel transitions, refresh,
RAM-disk selection and persistent initialization. Stubbed I/O is labelled.

### 4. Complete LIB, UI and command dispatch

Finish windows, keyboard/editing, menu traversal, highlighting, screen save/
restore, numeric printing, `Handle` and `MakeJmp`. Preserve raw menu byte blocks,
key order, unknown-key behavior and empty-directory command gates. Keep the
original dispatch algorithm in C without introducing function-pointer tables.

Acceptance: reuse menu/dispatch cases through adapters, compare logical screen
bytes and ordered external events, exercise repeated input and arena/CPU/software
stack balance. Full TN runs to the actual command loop with unported operations
explicitly identified; no stub counts as an implemented command.

### 5. Finish file operations and real Atari execution

Port Copy, View, Rename, Delete, MkDir, Attrib, Format, NewDrive, screen switching,
startup, `NavError` and `Quit`. Preserve copy continuation, channel cleanup,
disk-swap prompts, tagging/selection restoration and exact CIO command arguments.
Resolve all startup, copy-buffer and nonlocal-recovery obligations from slice 1.

Acceptance: deterministic VM file-operation scripts check channels, names,
chunk bytes, partial transfers, EOF versus error, retry/abort and recovery state.
Then execute full images under an Atari emulator with pinned MyDOS and disposable
ATR fixtures: launch, directory navigation, two-panel switches, view/copy/rename/
create/delete/attributes and controlled format/error/exit paths. Preserve source
ATR fixtures; destructive command scenarios use generated copies. Record the
actual scenario outcomes; VM stubs cannot substitute for this execution gate.

### 6. Add TNDBG and qualify the complete port

Port the additional TNDBG commands, counters, inspection and copy diagnostics.
Prefer shared C implementation plus explicit debug configuration while preserving
each source variant's behavior. Complete the routine inventory, with no unexplained
omissions or production stubs. Rerun affected application cases for both variants
and both Oscar64 profiles. Validate reserved memory and copy capacity per build.

Acceptance: full TN and TNDBG behavior is qualified in VM and emulator runs,
builds reproduce from pinned inputs, and all supported semantic differences and
adapter costs are visible. No performance win is required for a correct port.

### 7. Publish comparison and compiler recommendations

Measure production kernels, call chains and whole applications using the protocol
below. Investigate the largest gaps through Oscar64 IR/assembly and Action! NIR,
MIR and assembly. Rank findings by demonstrated bytes/cycles and generality.
Classify scalar promotion, SSA destruction/copies, placement, range/alias facts,
ABI/inlining, instruction choice, source semantics and runtime differences
separately. Turn only verified opportunities into subsequent compiler plans.

Acceptance: reproducible scorecard and saved artifacts explain the results;
no causal claim that SSA alone explains a gap, and no predicted saving is
presented as measured. Implementation is complete only after slices 0–7.

## Comparison and validation protocol

Use production `-Os` and `-O3` profiles with each exact command recorded. Add
no-inline/fixed-call diagnostic controls only where needed to isolate a cause;
label their changed compilation policy. Allow normal whole-program optimization
in production builds and include caller/callee savings when inlining changes
routine boundaries. Supply runtime inputs and observe outputs so benchmarks
cannot disappear or become constant answers.

For matched comparisons, use the same CPU/memory profile, OS bindings, RAM
ceiling and available zero-page budget. Report compiler-owned zero-page use and
fixed source reservations separately. Record any environment that cannot be
matched as a separate deployment result. Repeat representative cases across page
boundaries and relocated nonfixed storage; choose valid nonoverlapping layouts
rather than assuming every full image fits every origin. Test wrap only through
a qualified address model. Interrupt/DMA-free VM cycles describe CPU work;
separate actual emulator/hardware elapsed time and I/O wait.

Report per variant/profile:

- Output-file bytes, emitted code/constants/runtime/adapters, BSS/static homes,
  CPU/software-stack bounds and measured peak usage, zero page, total reserved
  RAM, and remaining copy-buffer capacity. Avoid double-counting aliased storage
  or shared helpers; do not treat absent BSS bytes as free memory.
- Kernel and operation cycles, workload counts, call/helper traffic and pointer
  setup/spill traffic. Include calls in region totals and separate test-adapter
  overhead. Count inherited assembly independently from compiled high-level code.
- Correctness scenarios, full buffer/guard checks, ordered external-access/CIO
  traces, stack/arena balance, source mapping, compiler/input/image hashes and
  failures. Record repeated compiler timing samples separately from target cycles.

Use independent expected data, including existing host-side TN models. Comparing
two binaries with each other alone is insufficient. Extend or share scenario data
without rewriting proven Action!-specific harness assumptions unnecessarily.
Test host text parsing/instrumentation with actual LF and CRLF inputs, following
[AGENTS.md](../AGENTS.md); preserve exact ATASCII and binary bytes. Save structured
symbols from compiler maps/debug output where possible, avoiding address patterns
that depend on a single listing generation. Consume generated assembly directly;
there is no need to disassemble Action! binaries.

This is sample/tool/test work. Run the new comparison checks, the affected TN VM
and integration targets, and the relevant generator checks when fixtures change.
Do not run every backend or the full NIR matrix by default. If a compiler/shared
IR fix becomes necessary, make it a separate slice with the repository's required
contract checks, then refreeze affected comparison baselines.

## Evidence and checkpoints

Use [existing TN VM cases](../tools/vm-runtime-tests/tests/tn_dispatch.rs) and their
[panel](../tools/vm-runtime-tests/tests/support/tn_panels.rs),
[directory](../tools/vm-runtime-tests/tests/support/tn_directory.rs),
[menu](../tools/vm-runtime-tests/tests/support/tn_menus.rs) and
[copy](../tools/vm-runtime-tests/tests/support/tn_copy.rs) models as initial
application oracles. The current tests stub portions of UI/CIO and therefore
establish only the stated scenarios, not complete Atari integration.

The [external code-generation review](TN_6502_CODEGEN_REVIEW_2026-11-10.md) and
[pointer-retention implementation](MIR6502_POINTER_RETENTION_IMPLEMENTATION_PLAN.md)
provide hypotheses and current blockers. The C experiment should confirm,
refine or reject those hypotheses before expanding MIR6502 architecture.

Checkpoint after slice 2: publish early kernel findings and platform blockers.
Continue toward the full port without treating early wins as completion.
Checkpoint after slice 5: verify that the actual TN application works on MyDOS.
Final checkpoint: both variants qualified, compact comparison evidence tracked,
large artifacts reproducible, and the next compiler work ranked by measured value.

## Implementation evidence

Slice 0 freezes the pinned toolchain/source baseline and routine inventory in
[the qualification directory](../surveys/tn/c-oscar64/README.md). Both native
Oscar64 profiles pass actual-startup ABI/layout checks, six runtime-input VM
executions and actual LF/CRLF compilation equality. The loader adapter handles
Oscar64 RUNAD `$02E0` explicitly; the pinned VM convenience helper expects the
Action! `$02E2` INITAD convention. Source fixed-cell guards pass with the reduced
compiler temporary bank. Application-wide footprint and recovery proofs remain
slice 1 obligations. Reproduction is in the [port README](../samples/tn/c/README.md).
