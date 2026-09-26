# Epilogue, BYTE-index and cast size measurements

Baseline: compiler `fc729cf0`, frozen Exec `622b139-dirty`, 631 routines and
120 checked input hashes. All measurements subtract guard ranges from the
same guarded workload; these are not separately linked guard-disabled builds.
Full/final backend and hosted Exec qualification remain deferred.

The [implementation plan](../../MIR65816_EPILOGUE_INDEX_CAST_SIZE_PLAN.md)
defines six commits. [measure.py](measure.py) compares frozen inventory outputs,
checks ABI/frame/peak/data/guard invariants and publishes routine/span deltas.

## Slice 1: shared void epilogues

Code shrinks **354,944 → 354,736 B**, saving **208 B** in 43 routines. There
are 71 redirected returns and no larger routines. The explicit REP at each
shared internal join costs two bytes; redirected BRA/BRL adds 3/4 cycles,
respectively, and every shared return also executes the 3-cycle REP. Frames,
local peaks, ABI, initialized data and all 2,676 guards (72,252 B) are unchanged.
The estimated loaded size excluding guards becomes **293,091 B**.

[Summary](01-void/summary.json), [routines](01-void/routines.csv),
[changed spans](01-void/spans.csv).

Validation: 264 emitter unit tests and two new join-refusal tests pass; one
existing unit test is ignored. All 22 emission and 11 o65 integration tests and
the unchanged boundary snapshot pass. Focused native call-return, guard,
replay and state tests pass. The new shared-tail fixture passes in debug and
release, raw/optimized and LF/CRLF forms, flat/two-placement o65 execution, and
IRQ/NMI injection at each reached instruction in both task domains and I states.

## Slice 2: shared native value epilogues

Code shrinks **354,736 → 348,995 B**, saving **5,741 B** in 340 more routines,
with 1,034 redirected value returns. No routine grows. Both epilogue slices
save **5,949 B** against the 6,488-byte model: 766 bytes of explicit join repairs
are partly offset by 227 bytes of branch/layout effects. Result preparation
and immediate call-result forwarding remain at their source returns. ABI,
frames, guards and data are unchanged. The loaded-size estimate is **287,350 B**,
leaving **25,206 B** to the cap.

[Summary](02-value/summary.json), [routines](02-value/routines.csv),
[changed spans](02-value/spans.csv).

Focused native checks cover BYTE constants, captured BYTEs, words, wide values,
forwarded calls and shared tails (17 tests). The new test exercises every result
class and interrupt reentry with actual result capture. The existing constant
BYTE test now locates the retained tail through its MIR source span while
preserving its ca65, stack-read and register assertions.
All three shared-epilogue tests also pass in release. The 22 emission and 11 o65
integration tests pass. The reviewed boundary snapshot changes only the
multiple-return accumulator and recursive-sum routines: one REP/internal label,
one replacement BRA, and corresponding span/fixup positions; the updated check
passes. No full qualification was run.
The adjacent-word test probe deliberately excludes typed shared-return joins:
its old single-instruction endpoint cannot describe the transferred A result.
The dedicated shared-tail tests verify the result lanes through those joins.
