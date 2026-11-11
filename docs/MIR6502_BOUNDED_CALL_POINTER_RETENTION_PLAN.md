# MIR6502 bounded call pointer retention

Implement a destination-disjointness proof for pointer retention across calls.
The proof must establish both that a captured pointer pair survives and that
the ordinary pointer homes it represents remain unchanged. Preserve unknown
destinations and the existing parameter-home alias counterexamples.

## Scope and limits

The first slice handles inspectable leaf machine routines with bounded control
flow and ordinary RAM accesses. Interpret their typed machine bytes with the
actual materialized register and zero-page values at each call. Indirect Y
accesses conservatively cover every possible byte index when the index is
unknown. Every possible indirect destination must lie in an owned ordinary
allocation, away from zero page, stack, hardware, code and wrapping boundaries.
Record every direct scratch write as well as the indirect destination range.

Use these context-specific footprints in existing resident-pointer proofs;
never cache a footprint by callee ID alone. Unknown inputs, unsupported machine
instructions, malformed control flow, analysis budget exhaustion or physical
placement uncertainty retain the previous code. No source/callee names and no
printed assembly are optimization inputs. Existing structured machine decoding
supplies typed bytes; unresolved string operands are rejected by this extension.

This is MIR6502 analysis and selection. SemIR/NIR semantics, entry contracts,
parameter-home observability and the allocator contract remain unchanged.
Calls and source memory accesses retain their order. Earlier call/loop proofs
remain available under their original controls.

TN's `Strcat` takes unknown incoming pointers. Its callers obtain destinations
from mutable globals and `Alloc`, whose cursor has no checked upper bound.
Those facts cannot establish arena containment or a closed set of safe entries.
Do not manufacture such a contract from the three direct calls. This slice
must report TN's remaining unknown destination honestly; it must not remove
the existing post-copy parameter reloads on that basis.

## Implementation slices

1. Add an inert bounded machine-memory analyzer and focused proof tests. Cover
   backward loops, zero/256 counts, unknown destinations, ownership, wrapping,
   hardware, pointer scratch, unsupported instructions and malformed branches.
   Commit the fact model and this contract before enabling a consumer.
2. Add a separate default-off control and context-dependent footprint query to
   existing pointer retention. Retain all writes and remove only setups already
   certified redundant by its physical/source-equality and live-state proofs.
   Reconstruct footprints from proposed callees and final physical placement
   during existing costed replay. Add a normally lowered source fixture with
   owned buffers and a real machine copy helper, plus unknown-destination and
   parameter-home overwrite controls. Commit the selected behavior.
3. Execute feature-off/on raw/optimized-NIR, runtime, origin and LF/CRLF matrices;
   compare buffers, ordered accesses, stack, scratch and parameter-home effects.
   Measure neutral and TN/TNDBG images and frames. Run the affected MIR6502
   backend checks, qualify defaults, record results and commit.

No shared NIR contract change is planned. If one becomes necessary, document
and run its mandatory shared checks before consuming it.

## Implementation status

Slice 1 is committed as `dbeac306`. Three focused fact tests cover typed
machine decoding, loop footprints, count zero, unknown inputs, physical
ownership, hardware/wrap exclusions and malformed/unsupported instructions.
LF/CRLF Action sources lower to identical machine facts.

Slice 2 is committed as `60d456f8`. It adds the separately controlled context query to the existing costed
transaction. A normally lowered owned-buffer copy removes ten bytes and two
setup stores; its sample call saves fourteen cycles. The same helper with an
unknown destination retains the old setup. Unit cases reject an explicit
parameter-home destination, unknown source, hardware/stack effects, changed
callee and missing physical map. The parent call-retention switch also gates
the new consumer when CFG retention remains enabled.

The focused integration matrix passes raw/optimized NIR, both runtimes and
both origins. VM execution passes 2,952 calls across those modes and actual
LF/CRLF files, including every byte count, pointer aliases, a pointer into its
own home, and a copy that overwrites the pointer home and changes the following
dereference.

Slice 3 completes qualification and enables the optimized default. Two more
fact tests reject incoming-flag observation across all eight branch conditions
and incomplete relocation projections. Nonzero relocation addends, high-byte
relocations and byte-address operands naming larger physical addresses retain
the previous barrier; the existing flat decoder cannot certify those cases.

Final validation passes 1,242 MIR6502 unit tests, 108 integration tests in 21
targets, all 169 emission/load fixtures, and 38 tests in 13 affected VM targets.
Eight MADS LF/CRLF assemblies reproduce the optimized default. Four fresh
TN/TNDBG cartridge/standalone images match the preceding qualified images;
their string/parameter-alias and dispatch/panel VM checks also pass. The
[scorecard](benchmarks/6502-bounded-call-retention/README.md) records exact
hashes, unchanged storage, sizes and sample cycles. TN's `Strcat` remains 113
bytes because its destination bounds and entry provenance are still unknown.
