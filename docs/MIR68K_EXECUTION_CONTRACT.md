# MIR68K execution boundary

MIR68K consumes verified NIR, after shared aggregate ABI expansion. It never
consults SemIR or recovers executable meaning from names. NIR remains the owner
of Action! evaluation order, scalar widths, casts, control flow and effects.
MIR68K owns the native ABI, physical frames, access strategy and big-endian data
projection. Emission owns instruction encoding and final addresses.

The MIR preserves routine and program-entry IDs, callable signatures, result
homes, parameter and temporary types, block parameters and edge arguments.
External declarations retain their verified external-service ID separately from
their routine ID. Compiler runtime selection binds that ID and validates the
interface signature before supplying an adapter; MIR68K never parses a routine
display name to identify a service. Ordinary calls retain their existing ABI.
Edge arguments transfer in parallel. Comparisons retain operand signedness;
integer signedness applies to all integer widths. Copies preserve both source
and destination volatility and overlap-safe semantics.

Every data object has a stable identity, extent, alignment, writability,
placement, initializer bytes and an explicit zero-fill tail. Global descriptor
cells and their array backing are separate allocations. Statics and
routine-static storage have separate identities; a local ID is scoped by its
routine. Absolute and alias placements do not allocate another object.
Uninitialized automatic storage belongs to an invocation's frame. Local names
and types remain available as symbol metadata, never as address identities.

Relocations retain target identity, address space, width, addend and optional
numeric byte selection. Selected bytes must not become truncated pointer
relocations. Final linking must check address/addend ranges and reject overlap.

`verify_contract` checks stable-identity uniqueness, entry consistency, data
extents, aliases, relocation targets and encodings, storage/callee references,
temporary definitions and widths, call homes, and branch argument widths/arity.
The NIR verifier establishes the source typing and dominance before lowering.
MIR transformations must preserve those guarantees. Unresolved fallthrough and
operations requiring a runtime adapter can still be described by the canary;
executable acceptance must separately reject reachable unsupported forms.

This contract does not make every successfully lowered canary executable.
The supported instruction/runtime subset and its acceptance gates are described
in [the implementation plan](MIR68K_MINIMAL_EXECUTION_PLAN.md).

Physical instructions use typed widths, registers, addressing modes, machine
block IDs and symbolic addresses. The encoder rejects illegal operands and
emits original MC68000 forms only. Conservative conditional branches use an
inverse short branch over an absolute-long JMP. Optional relaxation replaces
branches and absolute jumps to zero-addend machine block labels with Bcc.W or
BRA.W when the signed word displacement fits. Iterative shrinking preserves
that range; distant targets retain the absolute form. Emission resolves each
relative displacement from the opcode address plus two and checks its range.

The encoder records symbolic longword operand sites as it writes instructions.
The relocatable object contains section-relative code/data, explicit initializer
and instruction fixups, allocation extents and typed symbols, without executable
IR. Numeric constants and absolute external references do not acquire movable
relocations. Relative branches resolve only inside CODE. Object verification
rejects missing sections, out-of-bounds and overlapping fixups, invalid encodings
and frame-relative relocation targets. Consumers check full address/addend ranges.

The bare linker consumes that object, allocates code and data at a full-width
even origin, and emits initialized segments plus zero-fill regions. Its layout
remains CODE followed by complete data objects in declaration order; fixed
regions and aliases do not advance allocation. Other consumers can assign
independent section bases. `ImageEnd` requires the contiguous bare layout.
All linked extents fit the 24-bit bus. Executable bytes include four bytes of
prefetch padding. Image verification rejects overlapping regions and an entry
outside code. Image symbols carry absolute or frame-relative locations; array
metadata distinguishes the descriptor from backing and records width, stride
and count. The compiler has no emulator dependency.

The compiler-owned [native artifact format](NATIVE_IMAGE_FORMAT.md) transports
this linked image without executable IR. Import validates stable symbol/layout
tags and region metadata before loading bounded payloads. In-memory and imported
images share region verification and VM symbol access; the VM retains ownership
of reserved-memory, stack, ABI and fault checks.

The native compiler API uses the existing source/module loader, semantic
analysis, reachable SemIR selection and NIR verifier/optimizer. Source-level
startup and origin constraints are diagnosed before the verified backend
boundary. Native output rejects source ORG and Atari SET code-origin controls;
the full-width compile option owns the linked origin. Input origins, including
textual includes, are retained outside the image to prevent output collisions.
Materialization rejects missing/parameterized program entries and
reachable unresolved fallthrough or terminal exits without a final typed fault. A6-relative temporary homes
are separate from automatic objects and the preallocated outgoing area; frame
sizes outside original MC68000 displacement limits are rejected.

Integer materialization supports 8/16/32-bit add, subtract, multiply, divide, remainder, negation, bitwise
operations, logical shifts, casts and signed/unsigned comparisons. Narrow loads
clear unused register bits, signed widening uses explicit EXT instructions, and
comparison results are normalized to 0/1. Dynamic shifts mask the provisional
MC68000 result to zero when the full source count reaches the operand width;
CPU modulo-64 count decoding cannot change Action! semantics.

Instruction selection uses MOVEQ only for a full long value equal to the
sign-extension of an 8-bit immediate. Constant integer operations use immediate
forms, with ADDQ/SUBQ for magnitudes 1–8. Constant shifts are sequences of
original immediate shifts or a zero result for oversized counts; dynamic
counts retain the full-count guard. Constant indexes use their wrapping 32-bit
scaled displacement, and power-of-two strides shift the captured index
directly. These address choices neither widen nor duplicate memory accesses.

Multiplication retains only the resolved result width. Byte/word products use
MULU.W; 32-bit products combine three unsigned 16-bit partial products modulo
2^32, so signed and unsigned bit patterns obey the same wrapping contract.
Only captured MIR values may be reloaded; the source expression is never
reevaluated. The sequence uses the existing D0/D1/A0/A1 scratch set.

Temporary homes are four-byte reservations with each value stored at its
actual width at the start of its home. Edge transfers stage all sources in a
separate frame area before writing destinations. Conditional edges use separate
machine blocks, so only the chosen edge's parallel transfer executes.

Optional block-local temporary forwarding tracks equal-width values in data
registers after materialization. A retained register may replace a private
temporary reload; the load disappears only if its NZVC effects are dead.
Big-endian partial reads cannot use a cached value of a different width. Every
instruction's register and CCR effects are explicit in this pass; calls,
branches and source-visible writes end forwarding. A private temporary store
can disappear only when no routine instruction still references that home and
its flags are dead. Frame reservations, edge staging and source-visible memory
accesses remain unchanged. The conservative materialization option permits
differential execution independently of NIR optimization.

Native calls use an outgoing area at the bottom of the caller's A6 frame.
Arguments and indirect targets are captured before this area is written. After
JSR and LINK, incoming arguments begin at A6+8. Slots are even-sized; a BYTE
occupies the first byte of its slot. Mutated or address-taken parameters are
copied to invocation-local homes. Scalars return in D0 and pointers in A0.
D0/D1/A0/A1 are scratch. Division temporarily borrows D2/D3, saving them in
A0/A1 and restoring them before completion; it makes no calls while they are
borrowed. D2–D7/A2–A5 remain preserved and A6/A7 are restored.
Classic Amiga library adapters accept a full-width library base followed by
typed Action! stack arguments. They capture arguments before loading the
library base into A6, extend narrow scalars, and use the audited library vector.
D2/D3 and the Action! frame pointer are saved before marshalling and restored
through SP after the OS call. Pointer results move from the OS D0 to Action! A0.
OS calls remain full register/CCR and conservative memory-effect barriers.
Final frame reservations are checked for overlap and signed-16 displacement
limits, including incoming arguments and fixed-size copy staging.

Memory addressing preserves full-width indexes. Constant stride scaling uses
32-bit shifts/adds, including non-power-of-two record strides. Known aligned
word/long accesses use native instructions; unknown or odd alignment uses
big-endian byte accesses. Volatile reads and writes do not duplicate or widen
observable byte accesses. Fixed-size copies stage the complete source before
writing the destination, preserving overlapping value semantics. Automatic
array descriptors, backing, and initialization remain invocation-local.

Indirect base alignment comes from the verified NIR alignment analysis. Each
stronger claim carries an opaque receipt identifying its routine, block and
captured value; MIR verification rejects missing or mismatched receipts and
invalid alignment values. A typed-MIR rewrite that changes the proving SSA
definitions must discard or recompute these receipts. The current backend
preserves those definitions through materialization. One effective-address
predicate checks base alignment, displacement and index stride. Word and long
accesses both require even addresses, and `pointer_alignment` can disable the
new indirect-access selection for differential execution.

Pointer cell alignment and descriptor initializers are not pointee guarantees.
The first analysis tracks private scalar cells and SSA values, with conservative
joins, unknown entry values and explicit call effects. Mutable descriptor
loads remain unknown; direct arrays and addresses of frame objects can prove
alignment without an interprocedural descriptor immutability analysis.

Control-flow selection fuses a block's final comparison with its branch only
when the boolean has exactly one use, that branch's condition. It selects the
same width and signedness as numeric comparison emission. Numeric results still
normalize to 0/1, and unfused conditions accept any nonzero value. Edge copies
execute only on the selected edge and retain parallel-assignment semantics.
Physical fallthrough removes jumps to the immediately following label or
inverts a conditional branch when that permits fallthrough. Labels remain
stable, and checked branch relaxation runs after layout simplification.
The `control_flow` option disables this selection for differential execution.

Native compilation has an explicit NIR promotion policy, independent of
materialization options. `NativeLoops` exposes eligible automatic counters,
accumulators and pointers as SSA values and block parameters. It is the native
default; `Conservative` retains the earlier profitability policy. Home elision removes unused local
cells before MIR frame planning; removed cells have no emitted memory symbol.
Alignment receipts are derived from the resulting verified NIR, after promotion.

Bounded allocation retains selected SSA values in D4–D7 across blocks and calls.
It checks dominating definitions, computes backward liveness including edge
arguments, and verifies deterministic assignments against interference and the
reserved scratch-register set. Values live repeatedly in loops have priority;
unassigned values keep private stack homes. No typed SSA definition or alignment
receipt is rewritten. Only used preserved registers receive save slots, and
every normal return restores them after materializing its result.

Parallel edge copies use final register/stack locations, omit identical homes,
and break cycles with one normalized longword spill slot. Narrow stack values
are read at their declared width before normalization; a big-endian byte/word
home is never reinterpreted as the low bits of a longword. Frame layout follows
allocation, retaining source-visible objects and distinguishing saved-register
bytes from spill bytes. Unused and register-held temps have no stack reservation.
The `register_allocation` switch retains the conservative path for differential
execution. Block-local forwarding receives only the remaining private spill
homes and continues to respect widths, calls and flag liveness.

Qualified symbol names are attached as display metadata by the compiler API;
no machine decision depends on source/linker spelling. Parameters and automatic
objects have frame-relative symbol locations, including the actual source
parameter names. Routine symbols report their emitted code extents.

Division and remainder consume operands already converted to the NIR result
width/domain. A 32-step unsigned restoring loop operates on magnitudes; signed
correction produces truncation toward zero and a remainder with the dividend's
sign. MIN/-1 wraps and MIN MOD -1 is zero. Runtime zero is checked before any
result store, including a divisor that becomes zero after conversion.

`Mir68kOp::Fault(RuntimeFault)` preserves NIR's typed, non-returning reason.
Verification requires it to be the final operation of an Exit block; it has no
ordinary callable signature or stack arguments. The bare native adapter puts
an explicitly mapped reason code in D0 and executes TRAP #14. A following
self-loop prevents continuation even if an adapter incorrectly returns. The
r68k harness reports RuntimeFault separately from architectural exceptions and
completion, and latches it so subsequent run requests execute no instructions.
The mapping in `mir68k::runtime` is independent of Atari Error numbers. Other
native platforms must provide this adapter before executing these images.
