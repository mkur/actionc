# Action! 65816 compiler requirements before Exec

## Purpose and readiness decision

Exec will be a standalone, preemptively multitasking system for custom Atari
65C816 machines. Action! will implement its ordinary kernel code; a small
assembly layer will handle startup, interrupt entry, context switching and
machine primitives. GEM will be an optional GUI above Exec.

**Begin Exec implementation when the mandatory contracts below have executable
acceptance evidence.** Parsing Action!, producing verified NIR, or describing a
65816 frame is insufficient. The compiler must emit code that survives two
independent execution contexts entering the same routines and runtime helpers.

This is a compiler readiness contract, not an Exec implementation plan. The
qualification harness needs only two preallocated contexts and an assembly
switch routine; it does not need a scheduler, allocator, message system or GEM.
Kernel API design can proceed while these compiler gates are being completed.

## Inspected baseline

Source inspection: 2026-09-15, actionc commit
`c73818d91a2b458aa0d65fcb30d2e3ca844699ec`. This note records no new compiler or
emulator test results.

| Area | Existing foundation | Remaining readiness evidence |
| --- | --- | --- |
| Target model | [Native and small profiles](../src/target.rs), including native activation, pointer widths and layout. | A selected executable profile with a complete physical ABI. |
| Invocation storage | [Native routine contract](NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md) and [acceptance tests](../tests/native_routine_abi.rs) cover automatic storage, recursive activation and native MIR plans. | Independent execution of emitted 65816 frames, calls and helpers. |
| 65816 backend | [MIR65816](../src/mir65816/mod.rs) and [lowering](../src/mir65816/lower.rs) plan frames, mode boundaries and near/far calls. The module explicitly stops before register allocation and emission. | Instruction selection, allocation, encoding, linking and execution. |
| Native types | [Native type contract](NATIVE_TYPE_SURFACE_IMPLEMENTATION_PLAN.md) includes wide integers, typed callables, `ADDRESS` and `SIZE`. | Correct 65816 instructions and runtime support for the selected kernel subset. |
| Executable validation | [MIR68K](MIR68K_CHECKPOINT.md) demonstrates independent execution of emitted bytes. | Equivalent 65816 qualification; 68k execution does not qualify this backend. |

**The inspected 65816 backend is not yet ready for Exec implementation.** Reuse
the shared semantic and NIR work, then complete and qualify the target backend.

## Mandatory compiler contracts

### R1. One explicit native profile and a useful kernel subset

Qualify `wdc-65816-native` first. Its current layout is little endian, with
three-byte data and code pointers, 24-bit `ADDRESS` and `SIZE`, and native
invocation storage. `BYTE`/`CHAR` remain eight bits, `CARD`/`INT` sixteen, and
`LONGCARD`/`LONGINT` thirty-two. Preserve the target's natural record layout;
publish field offsets, padding, alignment, array stride and descriptor layout.
Do not infer record packing from the three-byte pointer size.

The initial emitted subset must support:

- integer loads/stores, casts, addition/subtraction, comparisons, bit operations
  and shifts at the widths above, including signedness and carry boundaries;
- conditional control flow, loops, early returns and local initialization;
- addressable local/global records and fixed arrays, field access, typed
  pointers, pointer arithmetic and the layout queries needed by an allocator;
- direct calls, typed indirect calls, scalar/pointer results, mutable value
  parameters, recursion and mutually recursive calls;
- copying and clearing ordinary memory, including overlap-safe movement where
  required by the language operation or explicitly selected primitive.

Records passed by pointer are sufficient for initial task, list and message
structures. Define the supported subset explicitly. Every unsupported operation
or runtime binding must produce a diagnostic, never a successful partial image.
Preserve existing Action! evaluation, conversion and overflow rules.

### R2. A complete, versioned ABI that assembly can implement

The selected contract is now [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md),
with [machine-readable layouts](abi/action65816-native-v1.json). This specifies
the decisions below; implementation and G2–G4 execution evidence remain required.

Publish the physical application binary interface (ABI): argument and result
homes for each supported width, stack argument order, return-address layout,
frame reservation, stack cleanup, register preservation and scratch ownership.
Include direct calls, indirect calls, callbacks, entry and all exit paths.

Keep the planned native boundary: `E=0`, with 16-bit accumulator and indexes
(`M=0`, `X=0`), and far `JSL`/`RTL` calls. Specify decimal-mode requirements,
direct-page register (`D`) and data-bank register (`DBR`) conventions. Internal
width changes must remain valid across control-flow joins, calls and interrupts.
Any near-call optimization must preserve the public far-call contract.

Provide a supported way to import assembly routines and export Action! routines
with stable symbols, signatures and effects. Independently assembled code must
both call Action! and be callable from it. An external assembler/linker bridge
is sufficient; a full inline assembler is not a prerequisite.

Assembly must be able to construct a context's initial stack, enter an Action!
task with its argument, and direct an unexpected task return to a declared exit
stub. Publish machine-readable offsets or generated constants for the shared
layout so compiler and assembly definitions cannot silently diverge.

### R3. Reentrant storage throughout generated code and helpers

Every live invocation must own its mutable parameters, locals, local array
backing, descriptors and temporaries. An initializer runs on each entry. An
address-taken local must remain attached to that invocation through nested calls
and suspension. Its lifetime ends when the invocation returns.

Audit every emitted helper, including copies, arithmetic, indirect-call
trampolines and fault paths. Mutable compiler workspace must be on the current
stack, in an explicitly owned context area, or otherwise covered by a documented
save/restore protocol. Fixed shared argument cells or temporary pointers are
unacceptable without such a protocol. Global kernel objects remain shared and
require synchronization in the kernel.

Publish the complete compiler/runtime state inventory, including direct-page
workspace, software-stack pointers if used, and any hidden runtime state. Saving
`D` alone does not save the memory it points to. Task-private workspace also
does not automatically make a helper safe when an interrupt handler re-enters
it on the same task; interrupt code needs a separate workspace or a documented
restriction on callable helpers.

### R4. Preemption and interrupt interoperability

The assembly save/restore contract must preserve full `A` (including its high
byte when `M=1`), `X`, `Y`, `S`, `D`, `DBR`, `PBR`, `PC` and processor status,
plus the compiler-owned state from R3. Ordinary call-preserved registers are
only part of a suspended task's state. Native task execution must keep `E=0`.

IRQ/NMI entry must save interrupted state before establishing the Action! call
boundary. Returning must restore the exact interrupted mode and execution
point. Test interruptions in prologues, epilogues, multi-instruction arithmetic,
width changes and runtime helpers, not only at source-level call boundaries.
If block-move instructions are emitted, cover their restart and bank-register
effects as well.

Document which helpers an interrupt handler may call and the supported nesting
policy. NMI entry must remain safe during IRQ masking, although it need not
perform task switching. Any compiler-inserted interrupt masking must have a
documented, bounded purpose; disabling interrupts around entire routines is
not an acceptable substitute for reentrancy.

The register, stack and addressing constraints here follow the
[WDC W65C816S datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf),
especially sections 2 and 3. The compiler ABI and test requirements are project
decisions built on those constraints.

### R5. Correct banked addressing and honest stack limits

Data pointers must retain all 24 bits in storage, arithmetic, calls and results.
Exercise objects and copies crossing `$xx:FFFF`, offsets above 64 KiB, pointer
fields in records and calls into multiple code banks. Define address-space
overflow and narrowing behavior explicitly; no silent bank truncation.

The linker must respect program-bank boundaries: place code fragments within
a bank or emit explicit transfers between banks. Validate branch ranges, call
targets and relocations after final placement. Long data addresses must work
regardless of the code bank and within the documented `DBR` convention.

The hardware stack and direct-page storage require bank-zero memory. Publish
their reservations and alignment constraints so Exec can allocate independent
contexts. The current frame planner limits its initial `d,S` strategy to 255
bytes. This is a backend frame-addressing limit, not the size of the native
hardware stack. Retaining a small-frame limit is acceptable initially if final
code enforces it and all qualification fixtures fit.

Report actual frame and call-stack costs after allocation: locals, spills,
saved registers, outgoing arguments, return addresses and maximum transient
stack movement. Document interrupt-entry headroom separately: an interrupt
pushes onto the interrupted hardware stack before a stub can change stacks.
Reject unsupported frames rather than truncating offsets. Record recursion and
unknown indirect depth as unbounded/unknown; do not claim a whole-program stack
bound without a depth assumption. A separate software stack is optional.

### R6. Hardware access and synchronization effects

Support explicit volatile memory access and absolute hardware aliases in the
qualified source subset. Define the emitted access width, byte order and number
of bus accesses. Optimizations must preserve required hardware reads/writes and
their order; a byte register must not acquire a wider neighboring access.

Provide assembly primitives for saving/disabling IRQ state and restoring the
previous state, with compiler memory barriers at both boundaries. Nested
critical sections must preserve the outer interrupt state. Memory operations
inside the section must not move outside it, and relevant cached values must
be invalidated across calls or assembly that can modify them.

Volatile does not make a compound or multi-instruction update atomic. Exec will
protect shared lists, queues and counters through explicit synchronization.
Document which single operations, if any, can be relied upon as indivisible for
the selected interrupt model. IRQ masking must not imply protection from NMI
or external hardware agents.

### R7. Freestanding executable output and debuggable placement

A reproducible compiler invocation must produce loadable 65816 machine code
with resolved symbols and no dependency on GEM, Atari OS services, the Action!
cartridge, Amiga libraries or a host-provided heap. Startup must have a documented
entry contract, initialize static data and zero-fill storage as specified, and
accept the stack/workspace supplied by the assembly bootstrap. Runtime faults
must reach an explicit freestanding handler or terminal trap.

Support separate placement of executable code, read-only data, writable data,
zero-fill storage and reserved bank-zero areas. A fixed-address linked image is
sufficient. Resolve external assembly symbols, function-pointer initializers and
data relocations with range and overlap checks; no dynamic loader is required.

Emit a map containing full-width addresses, entry points, section extents,
global symbols, automatic-object frame offsets and the stack costs from R5.
Provide an inspectable machine listing or a supported disassembly path. Execution
must consume the binary artifact, not reconstructed IR or debug text.

The [native image format](NATIVE_IMAGE_FORMAT.md) is a useful precedent, but its
current MC68000 layout and validation rules must be generalized explicitly
before it can carry little-endian, three-byte 65816 pointers and values.

## Executable acceptance gates

These are required future tests, not a report of existing 65816 coverage. Use an
independent 65816 emulator executing emitted bytes, with an assembly harness that
does not obtain expected results by interpreting the compiler's IR. Qualify the
emulator's relevant interrupt, width and bank behavior against the CPU contract.

| Gate | Required evidence | Contracts |
| --- | --- | --- |
| G1: scalar and memory execution | Boundary-value arithmetic/casts at 8/16/24/32 bits; branches and loops; record layout and array stride; pointer round trips; copy/clear including supported overlap and bank crossings. Compare complete expected memory and guard regions. | R1, R5 |
| G2: physical ABI | Action!-to-assembly and assembly-to-Action! calls, all supported argument/result widths, direct and indirect calls across banks, preserved registers, balanced stacks, and a fabricated first-task frame reaching its return stub. | R2, R7 |
| G3: invocation isolation | Recursion, mutual recursion, mutable parameters, per-entry initialized local arrays/records and escaped live local addresses. Two contexts enter the same routine and helper while both activations are live; assert distinct storage and correct results. | R2, R3 |
| G4: asynchronous suspension | Inject IRQ at each reachable instruction boundary in a small bounded corpus; switch between two preallocated contexts and verify resumed state and output. Cover all emitted mode changes and helper sequences. Exercise NMI while IRQ is masked and the declared nesting policy. Add reproducible seeded longer runs. | R3, R4, R5 |
| G5: effects and critical sections | Record exact volatile byte-access traces; verify an IRQ-updated polling value is reread. Test nested disable/restore with IRQ initially enabled and disabled, pending IRQ delivery, and protected multiword updates under optimization. | R4, R6 |
| G6: image and limits | Load a standalone image with code/data in multiple banks and correct initialization; verify map/frame facts against execution. Reject oversized frames, invalid relocation/branch placement, unavailable bindings and unsupported source features with useful diagnostics. | R1, R5, R7 |

Run the applicable corpus with raw and optimized NIR, and through each codegen
configuration advertised for initial Exec use. Check stack/workspace canaries,
unexpected memory writes, termination and fault behavior, not only a printed
answer. The interrupt harness must honor masking and the declared nesting
policy; it must never force a switch through a protected assembly sequence.

Keep the fixtures and reproduction commands in the repository. Record compiler
revision, target/ABI version, build options, assembler/linker and emulator
versions, image hash, memory map, interrupt schedule/seed and expected results.
When shared compiler infrastructure changes, run the existing required checks
and retain classic 6502 and 68k regression coverage.

Before declaring a compiler revision ready, all six gates must pass with no
known correctness defect in the qualified subset, and the ABI/state inventory
must match emitted code. Record an acceptance result linked from this note.
A real-machine bootstrap/interrupt smoke test is the next platform milestone;
emulator qualification alone does not establish custom-board behavior.

## Features that can follow the initial gate

- The `wdc-65816-small` executable profile, near/far mixed-model interfaces and
  code-size optimizations beyond the qualified native profile.
- General multiplication/division helpers beyond operations needed to implement
  the initial subset, REAL, strings, formatted I/O and a full standard runtime.
- Aggregate arguments/results by value, advanced language features and a full
  inline assembler, provided unsupported uses are diagnosed.
- Larger-frame strategies, stacks outside bank zero through software support,
  dynamic linking, loadable modules and advanced debug formats.
- Calypsi C ABI interoperability, GEM adapters, graphics and legacy Atari OS
  compatibility. GEM's compiler and pointer representation do not determine the
  native Exec ABI; any later bridge needs its own explicit contract.
- Competitive benchmark results, aggressive optimization and complete IDE or
  emulator integration. Correct, inspectable code and measured stack use are
  sufficient to start.

After acceptance, Exec owns scheduling policy, task/resource lifetimes, memory
allocation, synchronization APIs, message passing and device drivers. The
compiler owns faithful code generation and the documented machine contract on
which those facilities rely.
