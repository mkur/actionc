# Native-width call arguments and result capture

Starting from `860a02f2`, replace bytewise call payload copies and result capture
with native word transfers where the complete source/destination extent is
known. Preserve ABI v1, outgoing layouts and zero padding, declaration order,
all stack guards, cleanup, allocation and direct/indirect transfer contracts.
No new stack, DP or bank-zero reservation is permitted.

Preflight numeric constants and captured temp/parameter homes before emitting
the call guard. Check full extents with the prospective outgoing stack delta,
including the last byte. Exact-width words use A16; four-byte values use two
word transfers; three-byte values use a word and one byte, with no overlapping
stores or fourth-byte access. Unsupported symbolic or mixed-width operands
retain bytewise marshalling and its existing extension behavior. Source-memory
loads remain separate, preserving volatile accesses and evaluation order.

Select widths over each argument sequence using only two states, A8 and A16.
Include the next transfer/target-preparation width in the encoded-byte cost and
prefer bytewise copies on ties. This avoids mode-switch overhead making a mixed
BYTE/word/BYTE call larger; it adds no cross-operation value analysis.

After the existing result-preserving caller cleanup, capture the declared A/X
lanes directly into checked private homes. BYTE stores only A's low byte; word
stores A16; three-byte stores A16 and X's low byte; four-byte stores A16/X16.
Discarded results need no stores. Keep call barriers and introduce no persistent
register/flag relation across calls. Reuse typed instructions and existing replay,
effects and relocation machinery.

Qualification covers exact argument bytes and one write per outgoing byte,
all result widths and canaries, full call clobbers, mutable/incoming parameters,
direct/indirect calls, boundary displacements, zero/maximum constants, source
effects, relocation and IRQ/NMI during construction/cleanup/capture. Check LF
and CRLF through actual source/instrumentation paths. Run the affected native
unit/integration tests and full native debug/release runtime suites. Refresh any
intentional emission snapshots with an explained change; no NIR contract changes.

Compare frozen Exec, the small corpus and Dijkstra with the qualified baseline.
Separate call-site changes from secondary layout/mode effects, verify unchanged
frames/guards/ABI metadata, and record compiler and artifact hashes. Exec's live
sources, compiler pin and hosted qualification are separate from this slice.
