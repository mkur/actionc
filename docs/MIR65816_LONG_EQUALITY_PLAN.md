# Native 32-bit equality

Implement LONGCARD/LONGINT Eq/Ne and zero tests in MIR65816 selection, starting
from `26146332`. Preserve ABI v1, every guard, allocation, source-memory ordering,
call barriers and per-domain scratch ownership. Signedness does not change bit
equality; signed and unsigned ordering retain the existing fallback.

## Selection boundary

Extend the existing condition classifier with exact four-byte stack temps,
authoritative parameter homes and U32 constants. Check both word subranges,
including the final byte and transient stack delta, and the one-byte Boolean
destination before emitting any instruction, label or mode change. Invalid
homes are errors; legal unsupported operands fall back without partial emission.
Do not add DP allocation, source-load folding or implicit widening/truncation.

Compare the captured low words in A16, then the high words only when needed.
Eq requires both matches; Ne accepts either mismatch. Normalize an immediate
zero on either side to the right and consume each loaded word's Z flag without
CMP-zero. Use existing typed instructions, branches, selected CFG, replay and
layout finalization. Preserve the operation barrier and introduce no persistent
value/flag relation for a half of a long value.

Materialize one canonical BYTE 0/1 when a value is needed. For an immediately
adjacent Compare/Branch, reuse the existing routine-wide sole-use proof and
edge-copy machinery, retaining the Boolean home and all allocation metadata.
Same-target edges with different arguments and loop backedges remain meaningful.
Earlier volatile and aliased loads still read all four source bytes in order;
only private captured-home reads may short-circuit.

## Delivery and checks

1. Freeze the qualified BYTE-return compiler and Exec baseline. Add raw and
   optimized execution probes before changing selection; record sizes, cycles
   and stack/DP traffic. Cover both long types, unequal low/high halves, zero on
   either side, sign boundaries, constants, mutable parameters and Boolean uses.
2. Add selection and preflight tests, exact final-byte/ca65 checks and private
   traffic assertions. Check malformed/unsupported inputs without partial output,
   signed/unsigned ordering fallback, shared homes, displacements and fused uses.
3. Qualify source effects, call clobbers, nonempty edges, relocated fixed/o65
   images and full IRQ/NMI restoration at reached decisions in both task domains.
   Run scoped native unit/integration checks and full native debug/release suites
   with stable inputs. Exercise actual LF/CRLF parsing/instrumentation paths.
4. Rebuild frozen Exec, the small corpus and Dijkstra in both modes. Report
   per-routine and whole-image reductions, separate branch-layout effects, retain
   guard/frame/ABI invariants and publish hashes. Update the emission contract,
   backlog and quality plan. Added bank-zero reservation must remain zero.

Exec's compiler pin and hosted qualification remain separate. Do not combine
this slice with broader branch relaxation, call copies, arithmetic selection or
source/NIR changes. The audit's 24,568 bytes in 228 Eq/Ne sites are baseline cost,
not promised savings; refresh counts after the completed BYTE-return slice.
