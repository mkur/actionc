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

## Slice 3: stride-one BYTE indexes

Code shrinks **348,995 → 347,866 B**, saving **1,129 B** across 52 indexed
accesses in 22 routines. No routine grows; frames, guards, ABI and data remain
unchanged. The estimated loaded size is **286,221 B**.

[Summary](03-byte/summary.json), [routines](03-byte/routines.csv),
[changed spans](03-byte/spans.csv).

Checks cover all 256 BYTE indexes, offset 65535 and one-byte-overflow fallback,
full 24-bit carry/wrap, exact ordered reads/writes, neighboring canaries,
raw/optimized and LF/CRLF paths, flat/two-placement o65 execution and relocated
symbol bases. Existing CARD-index tests, pointer forwarding, memory tests and
reentrant indexed IRQ/NMI checks also pass. Three address-selector unit tests,
five address integration tests and the unchanged boundary snapshot pass.
Both new BYTE-index tests pass in release. Full qualification remains deferred.

## Slice 4: power-of-two strides and wider payloads

Code shrinks **347,866 → 346,044 B**, saving **1,822 B** in 22 routines; none
grow. The loaded-size estimate becomes **284,399 B**. Frames, ABI, data and
all guard ranges/amounts remain unchanged.

[Summary](04-scaled/summary.json), [routines](04-scaled/routines.csv),
[changed spans](04-scaled/spans.csv).

The selector retains the existing nonvolatile transfer policy: full words plus
an exact odd byte, preserving ascending external traffic. All 49 modeled sites
are admitted, including narrow zero and NULL constants; the 1,816-byte model
is exceeded by six bytes of mode/layout interactions. Native ASL A and INY have independent ca65/VM checks at both
accumulator/index widths, carry/sign boundaries and interrupt-mask states;
physical-effect tests verify every unwritten register/flag bit and memory access.

Validation: 266 emitter unit tests pass (one existing ignored), as do five
address integration tests, 22 emission tests and the unchanged boundary snapshot.
Five indexed runtime tests cover exact 1/2/3/4-byte traffic, exhaustive BYTE
indexes at a representative stride, fit/overflow boundaries, bank wrap,
flat/two-placement o65 execution and exhaustive reached-window IRQ/NMI reentry.
Three effect, ten state, two pointer-preemption, eight memory and two replay
tests pass.
The replay request inventory now explicitly includes the shared-return join.
All five indexed runtime tests also pass in release; the new instruction state
and physical-effect targets pass in release as well. Full qualification remains
deferred.

## Slice 5: other bounded constant strides

Code shrinks **346,044 → 344,710 B**, saving **1,334 B**, against the 1,331-byte
model. All three indexing slices save **4,285 B**, 19 bytes beyond their combined
4,266-byte model. Their offset, home and payload contracts remain bounded;
frames, peaks, guards, ABI and data are unchanged. The loaded-size estimate is
**283,065 B**, leaving **20,921 B** to the release cap.

[Summary](05-constant/summary.json), [routines](05-constant/routines.csv),
[changed spans](05-constant/spans.csv).

Five address integration tests and the unchanged boundary snapshot pass. Five
indexed runtime tests and two replay tests pass in debug. The scaled tests now
exercise all 256 indexes at strides 3 and 44, as well as 5, 255, 257, complete
payload bounds and overflow fallbacks. IRQ/NMI reentry covers the shift/add
windows for strides 3 and 44 in both domains and I states. High-stride test
addresses avoid the harness's own direct-page/stack mappings; separate cases
retain 24-bit wrap coverage.
Both expanded scaling/preemption tests also pass in release. Full qualification
remains deferred.

## Slice 6: native unsigned integer casts

Code shrinks **344,710 → 343,783 B**, saving **927 B** in 120 routines against
the 924-byte model. No routine grows. Cast selection counts entry and exit mode
requests, keeps the previous A8 exit contract and requires a strict byte saving.
It checks complete private extents and refuses partial overlap; same-start
transfers retain only necessary extension stores.

[Summary](06-casts/summary.json), [routines](06-casts/routines.csv),
[changed spans](06-casts/spans.csv).

The native cast matrix covers unsigned conversions and signed fallbacks,
truncation/extension, preserved source values, poison neighbors, sign-bit/all-ones
patterns, raw/optimized and LF/CRLF input, flat/two-placement o65 execution and
exact external source reads. A separate test injects IRQ/NMI at each reached
conversion-window instruction, with reentry in both task domains and I states.
Pointer-value, pointer-coalescing and replay targets also pass.
Both cast runtime tests pass in release. All 22 emission and 11 o65 integration
tests and the unchanged boundary snapshot pass. Full qualification remains deferred.

## Completed result and remaining footprint

| Slice | Measured saving |
|---|---:|
| Shared void epilogues | 208 B |
| Shared native value epilogues | 5,741 B |
| Stride-one BYTE indexes | 1,129 B |
| Power-of-two strides and wider payloads | 1,822 B |
| Other bounded constant strides | 1,334 B |
| Unsigned integer casts | 927 B |
| **Total** | **11,161 B (10.9 KiB)** |

The measured result is 517 bytes below the 11,678-byte model: explicit shared
join repairs account for most of the difference, partially offset by layout
and mode interactions. Compiler code is **343,783 B**, or **271,531 B** after
subtracting the unchanged guards. Adding the carried-forward 8,300 B of package
assembly and 2,307 B of initialized data gives **282,138 B (275.5 KiB)**.
The remaining gap is **19,994 B (19.5 KiB)**. This is still guard-range subtraction,
not an independently built guard-disabled release.

The [completed summary](completed-summary.json) is generated by
[summarize.py](summarize.py). A fresh ranking of disjoint operation families in
the final frozen image, excluding guard ranges, is:

| Remaining operation family | Nonempty sites | Current footprint |
|---|---:|---:|
| Unindexed indirect scalar loads/stores | 3,667 | 75,441 B |
| Call sequences | 2,045 | 60,948 B |
| Private 24-bit captures | 2,229 | 18,712 B |
| BYTE equality/inequality comparisons | 1,032 | 14,130 B |
| Explicit address formation | 339 | 7,519 B |
| Integer casts after native selection | 368 | 3,988 B |

These footprints include required payload transfers and already optimized
instructions. They are an order for further inspection, not expected savings.
The retained pointer captures still need distinct ownership/lifetime evidence;
this plan has not expanded their forwarding windows. Any next implementation
should identify removable sequences and checked admission rules within these
families before assigning a whole-image saving.

For reproduction, each slice uses the existing `exec-size-inventory` probe
against `target/exec-current-audit/frozen-exec/`, followed by:

```sh
python3 -B docs/benchmarks/65816-epilogue-index-casts/measure.py BEFORE AFTER SLICE
python3 -B docs/benchmarks/65816-epilogue-index-casts/summarize.py
```

The frozen workload, probe outputs and VM execution artifacts remain ignored
working files. Summaries include their identities/hashes; the CSVs retain
routine and changed-source-span evidence. No hosted Exec or full/final backend
qualification was performed for this series.
The complete [frozen input identity manifest](frozen-inputs.json) is retained
alongside the summaries. The focused cast preflight test also passes, covering
five raw cast forms, both entry widths, odd/last-legal homes, same-start copies,
partial-overlap refusal, signed fallback and exact typed memory extents.
