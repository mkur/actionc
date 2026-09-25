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

## Slice 9: bounded local-frame sources

Against `658b8cd2`, **663 local captures** disappear. Code shrinks
**360,259 → 354,944 B**, saving **5,315 B**: 5,308 instruction bytes and seven
branch bytes. Guard-subtracted compiler code is **282,692 B**. There are 210
smaller routines and none larger; frames, peaks, guards, initialized data and
all 120 frozen hashes remain unchanged.
[Size and hashes](local/exec-summary.json), [routines](local/exec-routines.csv),
[spans](local/exec-spans.csv).

The 672-site / 5,376-byte model included nine parameter-owned frame copies in
`TaskPolicy.SameName`, `IOSettledCall` and `ConsoleTakeArrival`. Those are
intentionally excluded by the local-ownership rule. The 663 admitted local
sites save eight transfer bytes each, plus four bytes of mode interactions.
No ownership assumptions were widened to admit the remaining nine sites.

Validation: seven focused binding tests cover local ownership, escapes, partial
homes, overlap, bounds, source writes, calls, Copy and forged parameter metadata.
The 22 emission tests, 11 o65 tests and unchanged boundary snapshot pass.
Thirty-one focused native debug tests cover frame/pointer forwarding, pointer
values/coalescing/edges, home analysis/definitions, state tracking and replay.
Four focused tests pass in release, including the multi-use/preemption fixture
for both parameter and local sources. These retain raw/optimized and LF/CRLF
coverage, exact bank-crossing reads, canaries, flat/two-placement o65 execution,
and IRQ/NMI injection with reentry. Full/final qualification was not run.

## Completed phases and release estimate

| Work | Measured saving |
|---|---:|
| Phase 1: call cleanup/results (previous commits) | 4,011 B |
| Phase 2: direct argument pushes | 15,687 B |
| Phase 3: bounded pointer bindings | 15,083 B |
| **Phases 2–3 implemented here** | **30,770 B** |
| **All three phases since `9bff177b`** | **34,781 B** |

The complete plan's 34,819-byte model is within 38 bytes of the measured result.
The differences are the explicitly retained width-mismatch calls, one pointer
edge use, nine parameter-owned frame captures, and mode/branch interactions.
Phase 3 removes 1,880 captures in total and retains the original reservations.

The release estimate is now **293,299 bytes (286.4 KiB)**: 282,692 bytes of
compiler code after subtracting guards, plus the carried-forward 8,300 bytes of
package assembly and 2,307 bytes of initialized data. The compiler's 951 data
bytes are already included in that data total. The gap to 256 KiB is
**31,155 bytes (30.4 KiB)**. This remains guard-range subtraction from the frozen
guarded build, not a separately linked guard-disabled release.

The current image still has 2,229 private pointer captures occupying 18,712
bytes, outside these admitted windows. Further removal needs separate lifetime
or ownership proofs. The next bounded size opportunities, recounted against the
new image, are:

| Next work | Current footprint | Modeled saving |
|---|---:|---:|
| Shared epilogues: 1,488 tails in 383 routines | 13,164 B | 6,488 B |
| Bounded BYTE-index access using Y: 124 sites | 7,937 B | 4,266 B |
| Unsigned integer casts: 219 sites | 3,338 B | 924 B |

Shared epilogues add a branch on redirected returns. BYTE-index selection
retains the checked 16-bit offset bound and exact source accesses. These remain
models; they are not implemented or credited to the release estimate.
[Completed-plan summary](completed-plan-summary.json) records these totals.
