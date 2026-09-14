# MIR68K execution boundary

MIR68K consumes verified NIR, after shared aggregate ABI expansion. It never
consults SemIR or recovers executable meaning from names. NIR remains the owner
of Action! evaluation order, scalar widths, casts, control flow and effects.
MIR68K owns the native ABI, physical frames, access strategy and big-endian data
projection. Emission owns instruction encoding and final addresses.

The MIR preserves routine and program-entry IDs, callable signatures, result
homes, parameter and temporary types, block parameters and edge arguments.
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
emits original MC68000 forms only. Conditional branches use an inverse short
branch over an absolute-long JMP, so their size is fixed without relaxation.

The linker allocates code and data at a full-width even origin, resolves stable
IDs and checked addends, and emits initialized segments plus zero-fill regions.
All linked extents fit the 24-bit bus. Executable bytes include four bytes of
prefetch padding. Image verification rejects overlapping regions and an entry
outside code. Image symbols carry absolute or frame-relative locations; array
metadata distinguishes the descriptor from backing and records width, stride
and count. The compiler has no emulator dependency.

The native compiler API uses the existing source/module loader, semantic
analysis, reachable SemIR selection and NIR verifier/optimizer. Source-level
startup and origin constraints are diagnosed before the verified backend
boundary. Materialization rejects missing/parameterized program entries and
reachable unresolved fallthrough or terminal exits. A6-relative temporary homes
are separate from automatic objects and the preallocated outgoing area; frame
sizes outside original MC68000 displacement limits are rejected.

Integer materialization supports 8/16/32-bit add, subtract, multiply, negation, bitwise
operations, logical shifts, casts and signed/unsigned comparisons. Narrow loads
clear unused register bits, signed widening uses explicit EXT instructions, and
comparison results are normalized to 0/1. Dynamic shifts mask the provisional
MC68000 result to zero when the full source count reaches the operand width;
CPU modulo-64 count decoding cannot change Action! semantics.

Multiplication retains only the resolved result width. Byte/word products use
MULU.W; 32-bit products combine three unsigned 16-bit partial products modulo
2^32, so signed and unsigned bit patterns obey the same wrapping contract.
Only captured MIR values may be reloaded; the source expression is never
reevaluated. The sequence uses the existing D0/D1/A0/A1 scratch set.

Temporary homes are four-byte reservations with each value stored at its
actual width at the start of its home. Edge transfers stage all sources in a
separate frame area before writing destinations. Conditional edges use separate
machine blocks, so only the chosen edge's parallel transfer executes.

Native calls use an outgoing area at the bottom of the caller's A6 frame.
Arguments and indirect targets are captured before this area is written. After
JSR and LINK, incoming arguments begin at A6+8. Slots are even-sized; a BYTE
occupies the first byte of its slot. Mutated or address-taken parameters are
copied to invocation-local homes. Scalars return in D0 and pointers in A0.
Only D0/D1/A0/A1 are scratch; D2–D7/A2–A5 are untouched and A6/A7 are restored.
Final frame reservations are checked for overlap and signed-16 displacement
limits, including incoming arguments and fixed-size copy staging.

Memory addressing preserves full-width indexes. Constant stride scaling uses
32-bit shifts/adds, including non-power-of-two record strides. Known aligned
word/long accesses use native instructions; unknown or odd alignment uses
big-endian byte accesses. Volatile reads and writes do not duplicate or widen
observable byte accesses. Fixed-size copies stage the complete source before
writing the destination, preserving overlapping value semantics. Automatic
array descriptors, backing, and initialization remain invocation-local.

Qualified symbol names are attached as display metadata by the compiler API;
no machine decision depends on source/linker spelling. Parameters and automatic
objects have frame-relative symbol locations, including the actual source
parameter names. Routine symbols report their emitted code extents.
