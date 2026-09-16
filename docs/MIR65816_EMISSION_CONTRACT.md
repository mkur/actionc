# Native 65816 emission

The compiler emits freestanding machine code for `wdc-65816-native` under
[`action65816.native.v1`](MIR65816_PHYSICAL_ABI_V1.md).
[Initial Exec acceptance](MIR65816_EXEC_ACCEPTANCE.md) covers the subset below
on the VM's independent 24-bit bus, including context switching and interrupts.

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
  retain their physical widths. Direct/typed indirect calls and returns use the v1 scalar ABI.
- Loads, stores and address formation cover automatic objects, incoming
  arguments, globals, absolute addresses, pointer dereferences and indexed
  fields/elements. Pointer arithmetic and constant-stride indexing retain all
  24 bits. Signed narrow displacements are sign-extended; wide displacements
  use their low 24 bits, with pointer movement modulo 2^24. Static initializers, zero-fill, aliases and low/high/bank relocations
  are linked by stable identities.
- Integer addition, subtraction, negation, AND/OR/XOR, all six comparisons and
  integer/pointer casts are emitted. Signed comparison and signed widening use
  the retained typed facts. Arithmetic follows the NIR operation's width;
  notably, Action! unary minus on SIZE currently produces INT. Use a SIZE
  subtraction when the intended operation is modular 24-bit subtraction.
- Logical left/right shifts operate at the typed width, including signed
  integer operands. A count at least the bit width produces zero, following NIR
  semantics. The bounded shift loop uses only current-domain scratch.
- Ordinary whole-aggregate copies preserve source-value semantics on overlap.
  Byte loops use full-width pointers and per-domain scratch; they make no calls
  or temporary stack pushes. Local initializers execute on each entry, with
  descriptor cells separate from their invocation-owned backing.
- `USE A816MEMORY` with `--module-path runtime/65816` provides
  `Move(BYTE POINTER destination,source SIZE length)` and
  `Clear(BYTE POINTER destination SIZE length)`. Move handles overlap; both
  operate on ordinary contiguous memory and use invocation storage.
- Branches, loops, direct/mutual recursion and block-parameter transfers are supported.
  Parallel edge copies first save all sources, so loops can swap live values.
- Volatile accesses remain ordered byte accesses. A byte operation does not
  touch its neighbor. Wider volatile operations are not claimed to be atomic.

### Scalar instruction selection

MIR65816 owns access-width and addressing selection. Ordinary scalar loads and
stores may use sixteen-bit transfers plus a final byte. Three-byte transfers
between disjoint frame slots (or the same slot), and from a frame slot into
owned direct-page pointer scratch, may instead use overlapping words at offsets
zero and one. This touches no fourth byte; overlapping word transfers are never
used to duplicate external or indirect accesses. Volatile loads and stores keep
their exact ascending byte accesses.

Small indirect field displacements are carried by `[pointer],Y`, including bank
carry. Displacements that cannot accommodate a four-byte scalar within Y use
explicit full-width pointer addition. Address formation and aggregate copies
materialize any deferred displacement before consuming the pointer itself.

Accumulator-width knowledge is local to emitted instruction sequences and is
discarded at labels. Scalar operations may retain their final width; calls and
MIR control-flow boundaries restore sixteen-bit A. Procedure frame teardown
does not preserve an unused accumulator result. These choices change neither
the public ABI nor NIR memory effects, and do not allocate persistent values in
call-clobbered scratch.

Multiply, divide, remainder, by-value aggregate interfaces, REAL, foreign code,
unresolved runtime/builtin calls and source terminal exits have explicit
diagnostics. Volatile aggregate copies are rejected: use a deliberate scalar
byte-access protocol for such hardware. Freestanding terminal faults are supplied
through the platform assembly interface.

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
the shared declaration-address resolver remains outside the advertised initial
subset; banked MMIO uses explicit pointers.

## Allocation and stack checks

Temporary locations explicitly distinguish stack and direct-page homes. The
selector consumes a verified pointer-leaf plan when eligible. Its whitelist admits
only a single bounded block of ordinary three-byte pointer loads/stores and a
void return, with no indexes or calls. These operations may touch only their
allocated homes and addressed memory, using A/Y/flags without extra scratch.
Closed def/use intervals prevent reuse during a multi-instruction operation.
Three ABI pointer slots (D+0, D+3, D+6) are allocated deterministically; pressure
or an unsupported operation rejects the entire candidate before emission.
The allocation verifier checks identities, widths, ownership, lifetime overlap
and frame accounting. Stack fallback and edge-copy storage remain invocation
owned. No A/X/Y register allocation is introduced.

The allocated even fixed frame must fit 254 bytes. Incoming offsets are
recomputed after allocation. Every emitted stack-relative byte access is
checked against `1..255`, including accesses to argument values after reserving
outgoing space. No displacement is truncated.

At entry, emitted code checks the frame reservation against the current
domain's stack floor and ceiling. Before a call it checks `O + 3` (direct) or `O + 6` (indirect), then
reserves O, zeroes the entire outgoing area and writes arguments. There are no
additional temporary pushes beyond the declared transfer in emitted operations.
The source memory helpers use ordinary checked calls. Indirect calls capture the callable before PHK/PER and the
stack-synthesized RTL transfer; decrementing the target PC does not borrow
from its bank. Same-bank PER continuation/range checks run after placement.
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
and S unchanged. The platform assembles the separate
[context bridge](MIR65816_CONTEXT_INTERFACE.md) for task entry, IRQ, COP and NMI;
reset/startup and board-specific vector installation remain platform work.

## Images, placement and assembly

`actionc-65816-image`, version 3, contains initialized segments, separate
zero-fill regions, exports, data symbols, assembly imports and the platform
stack contract. The ABI and target identities are checked when loading JSON.
The platform loads declared regions and calls the exported program entry.
External address/alias declarations do not allocate or clear memory.
Version 1 and 2 images must be recompiled; the physical ABI remains v1.
Temporary maps contain `id`, `size` and a tagged `home`: either
`{"kind":"stack","displacement":N}` or `{"kind":"direct_page","offset":N}`.
Stack homes are checked against the allocated frame; DP pointer homes must
occupy one of the three owned ABI slots and cannot coexist with calls. DP
values are not stack spills. Lifetime and scratch-clobber proofs are checked
against typed MIR before selection, not inferred from the final map.

Optional `read_only_origin` and `zero_fill_origin` layout fields independently
place immutable data/templates and wholly zero-filled writable objects. When
omitted, they share `data_origin`. An initialized object with a zero-filled
tail stays contiguous. `ImageEnd` uses the highest allocated end. The platform
reserves bank-zero stacks/domains separately and checks cross-allocation overlap.

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
irq_effect      "preserve" (default), "save_disable" or "restore"
```

ABI/signature mismatches and missing bindings fail linking. Imports are
ordinary returning v1 routines, with conservative call effects and the current
execution domain. Their stack declarations are platform obligations, not
proofs obtained by disassembling their bytes. The IRQ primitives are explicit exceptions to ordinary I preservation:
`save_disable` requires zero arguments and a BYTE result; `restore` requires a
single BYTE argument and no result. Incompatible signatures fail linking. All
source calls remain conservative memory barriers; these declarations do not
relax aliasing or optimizer ordering. See the context interface for stack costs.

The compiler exports argument offsets/widths/alignment and body displacements,
result width, code address, signature identity, allocated frame objects and
temporaries, outgoing call/transfer costs, and the local stack peak.
`whole_task_stack_bound` is null. Image verification checks map extents and cost
consistency; it is not a verifier of arbitrary replacement machine bytes. Names
are display metadata. The independent assembly fixture hand-packs the published mixed
example and consumes only exported code addresses; it does not derive expected
offsets or results from compiler layout helpers.

## Disassembly and executable evidence

```sh
python3 tools/disassemble65816.py build/scalar.a816.json > build/scalar.asm
```

This disassembler reads emitted routine bytes, tracks their explicit M/X width
changes and rejects unknown/truncated encodings. Imported assembly remains
external; retain its ca65 listing and symbols.

[`tools/native65816-runtime-tests`](../tools/native65816-runtime-tests/README.md)
loads serialized compiler images into the pinned native VM with the qualified
status-timing patch. Handwritten callers/callees are assembled with ca65/ld65.
Memory regions, instruction/cycle budgets, guards and expected values are
explicit. Both raw and optimized NIR are exercised.

The [acceptance result](MIR65816_EXEC_ACCEPTANCE.md) records all 24 native tests,
the independent CPU corrections, G1–G6 evidence and remaining platform limits.
The [implementation plan](MIR65816_IMPLEMENTATION_PLAN.md) records the separately
committed slices. New operations, helpers, ABI changes or wider nesting policies
require corresponding execution qualification.
