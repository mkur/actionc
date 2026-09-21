# Native 65816 emission

The [experimental o65 profile](MIR65816_O65_PROFILE.md) retains typed emission
fixups for a separate relocatable output path. Its
[implementation status](MIR65816_O65_IMPLEMENTATION_PLAN.md) is tracked per slice.

The compiler emits freestanding machine code for `wdc-65816-native` under
[`action65816.native.v1`](MIR65816_PHYSICAL_ABI_V1.md).
[Initial Exec acceptance](MIR65816_EXEC_ACCEPTANCE.md) covers the subset below
on the VM's independent 24-bit bus, including context switching and interrupts.

## Compile an image

The initial driver is `actionc-65816`. It has separate platform options from
the Atari and Amiga drivers. The existing `actionc-emit` command still provides
65816 SemIR/NIR inspection.

Save this layout as `layout.json`. Addresses accept decimal JSON numbers or
quoted hexadecimal strings with a `0x`, `0X` or `$` prefix:

```json
{
  "code_origin": "0x018000",
  "data_origin": "0x120000",
  "stack_overflow": "0x048000",
  "nmi_extra_stack": 0,
  "imports": []
}
```

The same syntax applies to optional `read_only_origin`, `zero_fill_origin`,
and each assembly import's `address`. For example, `98304`, `"0x018000"` and
`"$018000"` specify the same address. Hex digits are case-insensitive; bare
`0x018000` is invalid JSON. Addresses must fit in 24 bits. Sizes, stack budgets
and symbol/signature IDs remain decimal JSON numbers. Emitted image JSON
continues to use numeric addresses.

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

## Experimental o65 output

Save these experimental options as `o65-options.json`:

```json
{"profile":"actionc.o65.experimental.v1","nmi_extra_stack":0,"imports":[]}
```

Compile with:

```sh
cargo run --locked --bin actionc-65816 -- --format o65-experimental \
  --o65-options o65-options.json -o build/scalar.o65 \
  fixtures/runtime/native65816_scalar.act
```

`--layout` is exclusive to JSON output. Both formats accept `--no-opt` and
module paths, and protect all loaded inputs when publishing output. For named
assembly imports, use `--emit-interfaces` to obtain the stable interface IDs,
then add options entries with `symbol`, explicit ASCII `name`, `stack_peak`,
`checks_stack: true`, optional `irq_effect` and `domains` (task=1, IRQ=2, both=3).
Final addresses are supplied to the reference relocator, not these options.

The library APIs are `Prepared::compile_o65`, `native65816::write_o65`, and
`mir65816::o65::{inspect, relocate}`. `relocate` takes serialized bytes and a
`Placement` with section bases, allowed/reserved regions, matching NMI allowance
and named `Provider` contracts/addresses/extents. It returns private loaded
regions, BSS and ABI/maps through `RelocatedImage` accessors. It does not write
guest memory, allocate task contexts or implement an Exec816 application loader.
See the [profile](MIR65816_O65_PROFILE.md) for the admitted subset and rejections.

The [o65 qualification](abi/action65816-o65-qualification.json) executes raw and
optimized files at two independent text/data/BSS placements, including imports,
multi-bank code, stack failures and preempted tasks. The complete 44-test native
suite passes in debug and release. JSON transport v3, physical ABI v1 and
generated stack checks are unchanged.

## Instruction state boundary

Native selection uses the private `TrackedEmitter65816` in
[tracked.rs](../src/mir65816/emit/tracked.rs). Its closed instruction forms select
both encoding and effects. The mutable encoder and `State65816` are private to
the facade; selection can inspect finalized bytes and metadata but cannot write
raw instructions or attach caller-supplied effects. Linking retains its existing
ability to patch finalized `Code` buffers.

The state owns width-qualified immutable A/X/Y values, N/Z provenance, C/V,
execution modes and environment, exact private stack-home generations, stack
movement and the existing single-use adjacent-word permission. DP and unknown
writes conservatively invalidate memory relations. Source memory is never cached.
Calls clear value/flag/home relations; I preservation becomes unknown because
import IRQ effects are resolved later by linking. Every label discards value/flag
optimization facts. Internal labels revoke mode-omission permission. Reachable
MIR entries may retain A16 permission only under a checked native A16/X16 body
contract: the ABI/prologue supplies the initial edge, and every CFG predecessor,
including both branch arms and later-emitted backedges, must discharge its
execution/stack obligation before finalization. Unproved/dead entries retain
explicit mode requests. Seeded label environments alone are not proof; a missing,
duplicate or incompatible transfer is rejected. This never retains values,
home relations or forwarding permission across joins, nor omits a needed SEP.

TSC/TCS use bounded stack-address equations. The body anchor, outgoing argument
displacement and transfer pushes are distinct: JSL has a three-byte peak, and
indirect PHK/PER/PHA/RTL has a six-byte peak with return facts applied at resume.
The original guards, overflow A/X/S state, homes, stores and ABI remain unchanged.
The foundation was byte-identical; subsequent checked MIR-entry width omission
removes only redundant REP instructions and shifts code positions accordingly.

The default-off `native65816-state-proof` feature exposes only immutable snapshots
and checked probes through `emit::proof`. Ordinary compilation collects no trace.
Qualification compares known values and simultaneous register/home/NZ relations
against independent VM execution and ca65 encodings, including rebased o65 code.
See the [implementation plan](MIR65816_STATE_TRACKER_IMPLEMENTATION_PLAN.md) and
[design](MIR65816_STATE_TRACKER_DESIGN.md) for the foundation and deferred work.

## Supported operations

Adjacent eligible word operations may forward a private stack temporary in A16.
An ordinary direct, nonindexed two-byte Load or native word ADD/SUB establishes
the fact only after its retained private store. The next native ADD/SUB,
materialized/fused word comparison, A16 return, or ordinary direct two-byte
Store may omit that same temporary's LDA. Selection checks TempId, the exact
allocated slot, zero transient stack displacement, known A16 and an unchanged
instruction/label cursor. The producer's full-word N/Z must still match A;
unchanged A alone is insufficient. Comparison operand swaps retain identity.

All stores and homes remain. Calls, helpers, labels, edges, stack movement,
other operations, intervening instructions and mode changes invalidate the
fact. Volatile, indirect/indexed, DP, byte and wider transfers retain their
original paths. No source-memory access is cached or reordered. Complete
operand/extent preflight still runs before a load is omitted. Each omission
removes only two private stack-byte reads; ABI, allocation, guard and interrupt
contracts are unchanged.

`Code.mir_spans` is nonserialized emission proof metadata keyed by MIR block and
operation index (ops.len() denotes the terminator; a fused comparison includes
its edges). Qualification combines these ranges with verified MIR identities,
allocated homes and actual machine instructions. It does not use the metadata
to execute code or publish it in image/o65 formats.

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

Nonempty edges whose arguments and parameters are all exactly two bytes may use
native A16 LDA/STA. Complete preflight checks stack sources, authoritative mutable
parameter homes, destinations, the target label and the two accessed bytes of
each existing four-byte staging slot, including transient S movement. A single
word assignment loads its entire source into A before storing directly to the
destination; its staging reservation and validation remain, without any staging
access. Self-copies still load and store. For multi-word edges, all sources are
captured in staging before any destination is assigned. Mixed-width and legal
unsupported nonempty edges retain bytewise emission. Internal labels reset mode
permission; proved MIR entries use the contract above. Word edges restore A16
when needed. No DP traffic, pushes, calls or wider
external memory accesses are introduced. Frame allocation, guard costs and
multi-word per-byte private stack traffic are unchanged. Each direct single-word
copy removes two private stack byte reads and two writes; word loads read both
bytes before the corresponding store.

Empty edges validate the target and arity, restore A16 only when local mode
knowledge requires it, and emit the existing typed JML. They never select A8.
Known A16 needs no mode instruction; A8 or unknown knowledge requires REP #$20.
Internal branch labels still revoke omission permission. This changes no branch decision,
nonempty copy, stack guard, frame, register value or data-memory access; it does
not introduce fallthrough elimination, jump threading or branch relaxation.

### Scalar instruction selection

Two-byte integer ADD/SUB may use native sixteen-bit A with one ADC/SBC and
an immediate store to the existing two-byte stack result home. Eligible operands
are two-byte stack temps/parameters and U8/U16 constants; U8 constants are
zero-extended and signed widening remains an explicit Cast. Selection checks
both bytes of every source/destination against the stack-displacement limit,
including transient S movement, before changing code or mode knowledge.
Legal unsupported forms retain bytewise emission; malformed locations remain
errors. This path uses no DP scratch, temporary pushes, or persistent registers.
It preserves the current allocation, call barriers, guards, and ABI. A volatile
load captured in a private temp may feed word arithmetic; the original memory
access itself is neither combined nor widened.

Two-byte comparisons may use one native CMP for equality/inequality (signed or
unsigned) and unsigned ordering, using the same checked word sources. The result
must have an exact one-byte stack home. Materialized results are stored as 0 or 1
in A8. Selection checks the destination and both inputs before changing code, labels or mode
knowledge. Signed ordering and legal unsupported sources/destinations retain
bytewise emission; malformed homes remain errors. CMP flags are consumed within
the operation before loading the Boolean, except for the adjacent branch fusion
described below. Both inputs are read before the result store, allowing
existing dead-input slot reuse. Complete-word reads may increase private stack
read traffic compared with the old high-byte early exit; original volatile or
aliased source accesses remain separate and unchanged. No DP scratch, pushes,
helpers, X/Y use, allocation change or ABI change is introduced.

A final eligible word Compare followed immediately by Branch may consume CMP's
C/Z flags directly when a routine-wide use proof establishes exactly one use:
that Branch condition. Other block conditions, edge arguments, returns and all
operation inputs (including addresses and indirect calls) disqualify fusion.
No flags cross an intervening operation or block. Both edge trampolines retain
parallel copies, typed JML fixups and A16 successor state, even for equal
targets with different arguments. Calls and source-memory operations stay in
place. Unsupported or nonadjacent pairs use ordinary materialization/branching.
The Boolean's home is still validated and reserved, with unchanged allocation,
storage maps and stack guards, but no 0/1 is written for an eliminated branch-only
value. Each executed fusion removes exactly one Boolean stack write and reload;
word reads and edge-copy traffic are unchanged. No value resides in flags or DP
across a call or another MIR operation. Preemption must preserve live A/P through
the adjacent load, CMP and conditional/JML sequence.

An A16 ABI return may load a U8/U16 immediate or an exact two-byte stack
temp/parameter directly into A16. It reuses the complete-word displacement
checks and explicit-cast rules above, including authoritative mutable-parameter
homes. Preflight failure changes no return-preparation bytes or mode knowledge;
legal unsupported operands retain generic return preparation. Other result
homes retain their defined high-bit guarantees. X is unspecified for A16 results
and is not cleared by this path. Both paths use the same frame teardown and RTL;
nonzero-frame teardown preserves A through Y. Selected preparation uses no DP
scratch, push, helper, or assumption about a preceding operation's register value.

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
and frame accounting. No A/X/Y register allocation is introduced.

Other routines use invocation-owned stack temporaries with CFG-aware lifetime
reuse. Backward fixed-point liveness includes indirect address bases, indexes,
call targets/arguments/results, returns, edge arguments and block parameters.
All inputs, outputs and values live across an operation interfere for its entire
instruction sequence, including dead outputs that selection still writes. Block
parameters, even unused ones, interfere with each other and successor live-ins.
Parallel-edge staging slots remain separate: selection saves every source before
writing any destination. This permits cyclic copies without destroying live-ins.

Only MIR value temporaries share storage. Frame objects, addressed locals and
mutable parameters retain their dedicated homes. No temporary address escapes,
and no alias-sensitive load forwarding or memory reordering is performed.
Values live across calls and helpers remain on the invocation's stack, outside
call-clobbered registers and DP scratch. Allocation is deterministic (descending
width, then interference count, then ID; first available aligned byte range),
and is rechecked against liveness, byte extents, frame objects, staging slots
and final accounting before selection. It need not find the minimum frame.
Both raw and optimized emission use allocation; `--no-opt` controls NIR passes.
See the [measurements and scope](MIR65816_TEMPORARY_ALLOCATION.md).

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
values are not stack spills. Multiple temporary IDs can share stack bytes;
their individual widths remain exact and spill bytes count physical extent,
not the sum of temporary widths. Lifetime and scratch-clobber proofs are checked
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
