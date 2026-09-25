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

## Slice 8: bounded same-block parameter uses

Against `75e3ad98`, **275 more captures** disappear and code shrinks
**362,469 → 360,259 B**, saving **2,210 B** (2,204 in instruction spans, six
branch bytes). Guard-subtracted code is **288,007 B**. Metadata, homes, peaks,
guards, initialized data and all frozen hashes remain unchanged; no routine grows.
[Size and hashes](bounded/exec-summary.json), [routines](bounded/exec-routines.csv),
[spans](bounded/exec-spans.csv).

The parameter cohort now removes **1,217 captures**, saving **9,768 B** including
28 branch bytes. The audit modeled 1,218 captures / 9,744 direct bytes. Its one
unadmitted site is `FSNames.Path`: the captured pointer is a Goto edge argument,
which correctly retains its original home pending a separate edge-copy proof.
Mode-request interactions add four direct bytes of saving relative to the
simple eight-byte model of the admitted sites.

Validation: 135 selector tests pass (one existing ignored test); five focused
binding tests additionally check multiple uses, hidden indexes, cross-block and
edge uses, barriers, casts and native pointer returns. Eleven o65 integration
tests and the unchanged boundary snapshot pass. Focused native debug checks
cover parameter forwarding, home definitions, pointer comparisons/values, the
new multi-use fixture and replay. The multi-use fixture is verified after adding
fresh typed definitions, and executes two exact external reads with a harmless
capture between them. Flat/two-placement o65 execution, LF/CRLF instrumentation,
canaries and exhaustive consumer-window IRQ/NMI injection remain covered.
Eight pointer/comparison tests pass in release. Full/final qualification was not run.
