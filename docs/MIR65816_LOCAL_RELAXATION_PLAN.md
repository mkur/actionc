# Remaining local transfer relaxation

Status: implemented and qualified. See the
[measurements and checks](benchmarks/65816-local-relaxation/README.md).

Extend routine-local layout after typed selection. Every internal conditional
branch carries its predicate and label; dispatch provenance remains separate
from encoding choice. Every exact local JML reference carries its label and
encoding. Calls and external fault transfers retain their existing encodings.

The finalizer starts with inverse-branch/JML conditionals and JML jumps. It
jointly reduces conditionals to their two-byte predicate and local jumps to
BRA (two bytes) or BRL (three bytes). Each decision checks the candidate's own
layout, including its forward shrink; iteration reaches a monotone fixed point.
Signed-byte and signed-word displacement checks are exact. Transfers outside
those ranges retain their long forms. Conditional compounds that cannot use a
short predicate retain six bytes.

Labels, fixups, PER operands, MIR spans and transfers, selected instruction
ranges, effect records and trace PCs share one checked position mapping. The
selected actions, CFG, effects and allocation remain unchanged. Reconciliation
checks the actual final encodings and retained relocation ownership. Malformed
sites, overlaps, interior metadata and out-of-range targets are errors.

Relative operands are complete before either image writer collects relocations.
Placed instruction, next-PC and target must share a 24-bit code bank without
low-word wrap. Existing bank-contained routine packing and bank-aligned o65
text relocation preserve displacement bytes; no loader or ABI extension is
needed. The finalizer does not exploit wrapping relative displacements.

Qualification covers signed reach boundaries and cascades, independently
assembled encodings, flags/register preservation, image and o65 execution,
indirect PER continuations, arithmetic/helper loops, guard success/failure and
IRQ/NMI restoration. All guards, their amounts and order, stack frames, homes
and bank-zero reservations must remain unchanged. Measurements compare against
`7a627e58` in both raw and optimized modes, using the frozen Exec workload and
the existing corpus/Dijkstra inputs.
