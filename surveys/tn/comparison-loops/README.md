# MIR6502 comparison loop qualification

The [implementation plan](../comparison-loop-pointer-reuse-plan.md) separates
pointer lifetime proofs, retained bases, comparison fusion and Y induction.

## Stage 0 baseline

[stage0.json](stage0.json) pins the compiler executable, source revision,
four current TN/TNDBG images, generic fixture hash, direct comparison vectors
and exclusive routine measurements. Large images, listings and typed/selected
MIR dumps are retained under `target/tn-comparison-loops/stage0`.

Both MIR roots contain a 246-byte `MyDosAfter` with zero spills, four parameter
bytes and three local bytes. Both reversed 64-entry sorts take 534,512 cycles
including the wrapper. The existing sort fixture contains `FILEnnnn` names
with common prefixes and an independently ordered host oracle.

The generic fixture and VM target cover first/middle/last and every other
mismatch position, equal and identical pointers, unsigned byte boundaries,
page crossings, a displacement-239 field, exact pointee read order, guards and
observable character/index values. They run MIR6502 cartridge and standalone
with LF and CRLF input. Classic is an independent source/ABI control.

Typed MIR has ordinary local induction and capture cells. The comparison body
loads each pointer, adds its embedded-field displacement, loads the common
index, then stores each character into a local before testing inequality.
The mismatch successor reloads those same characters for ordering. The
existing contiguous dual-indirect matcher cannot eliminate these capture
stores or their successor uses. The current counted-loop contract also does
not describe this body with its early-return exit.

Candidate discovery belongs in the pre-home canonicalization group after CFG
cleanup and block-argument lowering, before comparison and address expansion.
This group already has provisional physical layout and reserved pointer pairs.
Final ownership checks must be repeated after allocation/coloring and emission
layout. A companion comparison-loop fact will preserve the existing loop
selectors' narrower contracts.
