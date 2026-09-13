# MIR68K execution boundary

MIR68K consumes verified NIR, after shared aggregate ABI expansion. It never
consults SemIR or recovers executable meaning from names. NIR remains the owner
of Action! evaluation order, scalar widths, casts, control flow and effects.
MIR68K owns the native ABI, physical frames, access strategy and big-endian data
projection. Emission will own instruction encoding and final addresses.

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
