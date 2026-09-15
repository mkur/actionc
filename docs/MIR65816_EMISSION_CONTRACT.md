# Native 65816 scalar emission

Slice 4 adds freestanding machine-code emission for `wdc-65816-native` under
[`action65816.native.v1`](MIR65816_PHYSICAL_ABI_V1.md). The output executes on
the VM's independent 24-bit bus. It does not establish the context-switch,
interrupt or complete kernel-subset acceptance gates.

## Compile an image

The initial driver is `actionc-65816`. It has separate platform options from
the Atari and Amiga drivers. The existing `actionc-emit` command still provides
65816 SemIR/NIR inspection.

Save this layout as `layout.json`; addresses in JSON are decimal:

```json
{
  "code_origin": 98304,
  "data_origin": 1179648,
  "stack_overflow": 294912,
  "nmi_extra_stack": 0,
  "imports": []
}
```

The example places code from `$018000`, data from `$120000`, and expects the
platform's raw stack-overflow adapter at `$048000`. These are explicit layout
choices, not Atari board reservations. Create the output directory before
compiling:

```sh
mkdir -p build
cargo run --locked --bin actionc-65816 -- \
  --layout layout.json -o build/scalar.a816.json \
  fixtures/runtime/native65816_scalar.act
```

`--no-opt` disables shared NIR optimization. Optimized compilation uses native
loop promotion. Repeated `--module-path` options add module directories.
Compilation validates the image before publishing one self-contained JSON
file by rename. Loaded sources and the layout file cannot be output targets;
a compilation failure leaves an existing output intact.

The library entry points are
[`compiler::native65816`](../src/compiler/native65816.rs),
[`mir65816::emit::materialize`](../src/mir65816/emit/mod.rs), and
[`mir65816::image::link`](../src/mir65816/image.rs).
`Prepared::compile` accepts the explicit link layout. No emulator is a compiler
dependency.

## Supported operations

- BYTE, CARD/INT, ADDRESS/SIZE, data/code pointer storage, and LONGCARD/LONGINT
  retain their physical widths. Direct calls and returns use the v1 scalar ABI.
- Loads, stores and address formation cover automatic objects, incoming
  arguments, globals, absolute addresses, pointer dereferences and indexed
  fields/elements. Pointer arithmetic and constant-stride indexing retain all
  24 bits. Static initializers, zero-fill, aliases and low/high/bank relocations
  are linked by stable identities.
- Integer addition, subtraction, negation, AND/OR/XOR, all six comparisons and
  integer/pointer casts are emitted. Signed comparison and signed widening use
  the retained typed facts. Arithmetic follows the NIR operation's width;
  notably, Action! unary minus on SIZE currently produces INT. Use a SIZE
  subtraction when the intended operation is modular 24-bit subtraction.
- Branches, loops, direct recursion and block-parameter transfers are supported.
  Parallel edge copies first save all sources, so loops can swap live values.
- Volatile accesses remain ordered byte accesses. A byte operation does not
  touch its neighbor. Wider volatile operations are not claimed to be atomic.

Multiply, divide, remainder, shifts, indirect calls, aggregate byte-copy
operations, by-value aggregate interfaces, REAL, foreign code, unresolved
runtime/builtin calls and terminal exits have explicit diagnostics. A local
aggregate initializer may require an unsupported byte copy. Scalar field and
element access does not imply support for whole-aggregate copying.

Small-model emission is unsupported; its existing lowering/planning policy is
preserved. Source ORG/SET origins, fixed routine placement and top-level
executable statements require a separate startup/platform contract.

### Declaration-address limitation

The shared declaration-address resolver still uses 16-bit addresses. A bare
declaration such as `VOLATILE BYTE io=$F00000` can reach NIR as initialized data
instead of a hardware alias. The new driver rejects wide numeric bare
initializers in global and local declarations. Use bracketed initialized data
or explicit 24-bit pointer casts and accesses for banked memory. Bank-zero
absolute aliases are exercised by the volatile execution test. Generalizing
the shared declaration-address resolver remains compiler work before full R1
acceptance.

## Allocation and stack checks

Every MIR temporary gets an invocation-owned stack slot. Edge-copy storage is
also in the fixed frame. This conservative allocator favors straightforward
reentrancy over small frames; there is no register allocation or slot reuse yet.

The allocated even fixed frame must fit 254 bytes. Incoming offsets are
recomputed after allocation. Every emitted stack-relative byte access is
checked against `1..255`, including accesses to argument values after reserving
outgoing space. No displacement is truncated.

At entry, emitted code checks the frame reservation against the current
domain's stack floor and ceiling. Before a direct call it checks `O + 3`, then
reserves O, zeroes the entire outgoing area and writes arguments. There are no
additional temporary pushes or compiler helper calls in this subset.
Caller cleanup and frame release preserve A/X through the specified Y-based
sequence. Byte results zero A's unused high byte; 24-bit results zero X's high
byte.

`AllocatedFrame` and image routine metadata report the final fixed frame,
spill bytes and exact **local** reservation/transfer peak. This excludes the
callee's own checked reservations and the platform's interrupt headroom. It is
not a whole-task bound, particularly with recursion. The earlier abstract MIR
plan remains explicitly unallocated.

The loader/platform must establish native mode, M=X=0, decimal clear, DBR=0,
an aligned per-domain direct page, even entry S and valid v1 arguments/return
bytes. It must reserve task headroom `26 + nmi_extra_stack` or IRQ headroom
`13 + nmi_extra_stack` when initializing the domain's floor. Emitted checks
preserve I. Failure transfers by JML to the configured nonreturning
`__a816_stack_overflow_v1` adapter with required bytes in A, unchanged S in X,
and S unchanged. The image does not supply reset, task entry, IRQ, COP or NMI
stubs.

## Images, placement and assembly

`actionc-65816-image`, version 1, contains initialized segments, separate
zero-fill regions, exports, data symbols, assembly imports and the platform
stack contract. The ABI and target identities are checked when loading JSON.
The platform loads declared regions and calls the exported program entry.
External address/alias declarations do not allocate or clear memory.

Each emitted routine stays within one code bank, leaving the bank's last byte
unused. The linker advances to the next bank when necessary and rejects a
routine larger than that placement strategy permits. JSL calls and JML edges
are relocated with full addresses. No continuation relies on PC bank wrapping.
Out-of-range relocations, unresolved symbols, cyclic/out-of-bounds aliases and
overlapping image/import regions are rejected.

Declare assembly services with ordinary `PUBLIC EXTERNAL` Action! interfaces
in a module. Discover referenced interface identities and argument layouts:

```sh
cargo run --locked --bin actionc-65816 -- --emit-interfaces program.act
```

Add an entry to `imports` for each required service:

```text
symbol          runtime interface ID from --emit-interfaces
signature       structural signature ID from --emit-interfaces
abi             "action65816.native.v1"
address, size   actual assembled code range
stack_peak      assembly's local reservation peak below its entry S
checks_stack    true: assembly performs its own required reservation checks
```

ABI/signature mismatches and missing bindings fail linking. Imports are
ordinary returning v1 routines, with conservative call effects and the current
execution domain. Their stack declarations are platform obligations, not
proofs obtained by disassembling their bytes. The qualification fixture uses
leaf assembly routines with no local pushes and zero local peak.

The compiler exports argument offsets/widths/alignment, result width, code
address, signature identity and allocated frame costs. Names are display
metadata. The independent assembly fixture hand-packs the published mixed
example and consumes only exported code addresses; it does not derive expected
offsets or results from compiler layout helpers.

## Executable evidence and remaining work

[`tools/native65816-runtime-tests`](../tools/native65816-runtime-tests/README.md)
loads serialized compiler images into `actionc-vm::native65816`, pinned to
`56ddc5c5de41f0e7294e87c440869550eaf53292`. Handwritten callers/callees are
assembled with ca65/ld65. Memory regions and execution budgets are explicit.

The corpus covers all scalar widths, arithmetic boundaries, raw/optimized
loops, recursion, live local addresses, cross-bank code/data, exact volatile
accesses, the mixed ABI example, padding, unused result bits, scratch clobbers,
stack balance and guard bytes. Fault probes verify that floor violations,
subtraction underflow and ceiling violations transfer before stack writes.
Both enabled and disabled I states are preserved in the call/interop probes.

The [implementation plan](MIR65816_IMPLEMENTATION_PLAN.md) tracks validation
results and remaining slices. Indirect transfer is slice 5. Context fabrication
and IRQ/COP stubs are slice 6; asynchronous/two-context qualification is slice
7. The VM's REP/SEP/RTI timing limitations still apply. No emitted context
switch or full Exec readiness is claimed by this scalar corpus.
