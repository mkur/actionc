# Bounded pointer forwarding measurements

The frozen workload remains Exec `622b139-dirty`, optimized, 631 routines and
120 verified source hashes. Measurements subtract unchanged compiler guards;
they are not separately compiled guard-disabled release images. Existing
allocations, local stack peaks, initialized data, ABI and all 2,676 guards
(72,252 bytes) remain unchanged in these slices.

## Slice 7: adjacent immutable parameter

Against `6fd4bbe2`, **942 captures** disappear. Code shrinks
**370,027 → 362,469 B**, saving **7,558 B**: exactly eight bytes per capture
plus 22 local branch relaxations. Capture spans lose 7,836 bytes; 300 bytes of
necessary mode requests move to consumers, yielding the 7,536-byte direct
saving. There are 246 smaller routines and none larger. Guard-subtracted code
is **290,217 B**.

[Size and hashes](adjacent/exec-summary.json),
[routines](adjacent/exec-routines.csv), [spans](adjacent/exec-spans.csv).
The bounded binding contract is in
[MIR65816_POINTER_FORWARDING.md](../../MIR65816_POINTER_FORWARDING.md).

Validation: three focused binding tests cover positive selection, source
identity, retained homes, nine refusal cases and the last legal incoming byte
at displacement 255. The 22 emission tests and unchanged boundary snapshot pass.
Twenty-four native debug tests pass across pointer forwarding/coalescing/edges,
home analysis/definitions, state tracking and replay. Two new pointer tests pass
in release. They cover raw/optimized source, LF/CRLF parsing, flat images and two
o65 placements, exact bank-crossing reads and neighboring canaries. IRQ/NMI is
injected at every reached consumer-window instruction, in both task domains and
I states, with reentry of the same pointer-reading routine. Full/final backend
and hosted Exec qualification were not run.
