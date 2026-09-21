# Native 65816 word parallel edge copies

Completed on 2026-09-21. Nonempty edges with entirely word-sized arguments and
parameters now use A16 LDA/STA in both raw and optimized compilation. Sources are
captured into the existing staging slots before any successor parameter is
assigned. The [plan](MIR65816_WORD_EDGE_COPIES_PLAN.md),
[emission contract](MIR65816_EMISSION_CONTRACT.md) and
[qualification record](abi/action65816-word-edges-qualification.json) describe
the boundaries and bind the measurements to compiler, fixture and tool hashes.

## Selection contract

Whole-edge preflight checks arity, exact physical widths, source and destination
homes, the target label and accessed stack extents including transient S movement.
U8 is not implicitly widened for a word parameter. Mutable parameters use their
current frame home. Unsupported entries fall back only after later entries have
also been checked; preflight cannot emit a partial prefix or change mode knowledge.

The selected path uses stack words, U16 immediates and physical word parameters.
Every source is staged before any destination write, including repeated sources,
self-copies and unused parameters. Four-byte staging allocations remain unchanged;
only the low word is accessed. Empty, mixed BYTE/CARD/24-bit/32-bit and unsupported
edges retain bytewise emission. Labels still invalidate local mode knowledge,
so true-edge trampolines restore A16 when necessary. Typed JML fixups are unchanged.

Physical ABI v1, image v3, o65, storage maps, stack guards and depth are unchanged.
No helper, push, DP access, cross-block register lifetime or wider external memory
access is introduced. Original volatile/aliased loads and call barriers remain
separate. Stack byte reads and writes are identical in count and addresses;
within each word, both reads now precede the corresponding store.

## Measured results

The immutable baseline is the [fusion snapshot](benchmarks/65816-compare-branch/after/tables.md),
emitted by `4687170` and qualified at `01ea393`. All 224 saved file hashes and
264 matching debug/release records were rechecked before implementation.
Baseline semantic probes were committed as `988f574`; selection and focused
execution checks were committed as `b5b4cd4`.

The [new snapshot](benchmarks/65816-word-edges/after/tables.md) and
[delta](benchmarks/65816-word-edges/delta.md) cover the unchanged 14-pair,
66-vector corpus, both compiler modes and incoming I states. Values include
guards and RTL. These three optimized kernels contain all selected corpus edges:

| Kernel / input | Code bytes before → after | VM cycles before → after | Stack depth | Stack reads / writes |
| --- | ---: | ---: | ---: | ---: |
| sum loop(13) | 183 → 160 | 2,030 → 1,780 | 16 | 273 / 216 |
| loop rotation(13) | 247 → 192 | 1,720 → 1,344 | 26 | 201 / 178 |
| byte sum($12FFFC,16) | 281 → 258 | 5,004 → 4,700 | 22 | 624 / 523 |

Size forecasts match exactly. The plan's cycle forecasts understated selected
copy cost by two cycles per word. In the qualified VM, A16 stack-relative LDA and
STA each take five cycles: four instructions cost 20 cycles per staged stack
word, or 18 with an immediate source. Including the removed SEP/REP pair, the
one-word initial edge saves 16 cycles and a stack backedge saves 18. Rotation
saves 40 cycles on entry and 42 on each of eight backedges. All predeclared
regression ceilings pass; no CPU or timing-model change was made.

All 16 [predeclared vector counts](benchmarks/65816-word-edges/expected-edges.json)
match complete sequences decoded at reached machine instruction boundaries:
146 edges / 254 copied words per incoming I state and host build. All raw cases
and other optimized kernels select zero edges and retain byte-identical emitted
files and measurements. Strict default delta checks accept all 264 records,
without stack-read exceptions. The additional
[coverage check](../tools/compare65816/check_word_edges.py) proves unchanged DP
reads/writes/touched offsets, fusion counts and complete routine storage contracts.
Listings retain raw/optimized sum loop, rotation and byte sum.

Both external comparison commands retain the existing optimized vbcc `unlink`
vector-0 failure, in both I states. Complete debug/release results agree, and
all vbcc records are identical to baseline. There is no failure exemption.

## Execution, interruption and relocation

Three word-edge tests execute constructed verified MIR in both frontend modes:
three-value rotations, repeated sources, unused parameters, live-ins, self/backedges,
ordinary and fused branches, mixed widths and authoritative mutable parameters.
They use full word boundaries, both I states, independent ca65 encodings and
exact ordered bus traces. Every selected edge must finish in A16/X16, preserve
S/D/DBR/X/Y/I, touch no DP and leave upper staging canaries unchanged. Existing
same-target swap tests remain, and volatile/aliased bank-crossing captures now
cross explicit word edges before direct/indirect calls that clobber all scratch.
External byte traces remain exact. Decoder tests reject malformed/truncated
copies, wrong modes and invalid targets.

The targeted interrupt probe rotates three full words through overlapping homes
in both task domains. It checks 330 task/PC sites per mode, including all 222
word-copy sites, while retaining six fused comparison windows and all 24
post-CMP truth/task combinations. At each injection, a separate uninterrupted
CPU step supplies the expected state after instruction completion. All 354
restorations preserve every register and the invocation-owned stack contents,
including partially captured or assigned words. Both seeded IRQ/NMI schedules
pass. General preemption retains 2,275 raw / 2,117 optimized enabled instruction
sites; materialized comparisons retain 96 sites and 16 truth/domain combinations.

The new o65 probe executes serialized nonempty Goto/backedges and both conditional
arms at `$100000` and `$600000`, after dropping compiler objects. It checks word
results, decoded relocated JML targets, reached counts and guards in both modes
and I states. Bytes, placements, measurements and decoded sites are saved.

Full native qualification passes **71 tests in debug and release**, with **224
identical artifacts**. Compiler checks pass 32 native library tests and 58
integration/CLI tests; existing decoder/delta tooling passes 5/6 Python tests.
LF/CRLF corpus builds agree. No NIR contract or allocator policy changed; the
full root suite and repository-wide NIR sweep were outside this emission slice.

Staging-slot shrinking, native mixed-edge selection, self-copy removal and
copy scheduling without staging remain separate allocation/traffic changes.
Qualification uses emitted machine code on the corrected pinned VM; board and
Exec816 loader acceptance remain separate integration work.
