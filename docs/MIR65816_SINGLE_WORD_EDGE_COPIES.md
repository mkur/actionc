# Native 65816 direct single-word edge copies

Completed on 2026-09-21. Baseline semantic probes and typed edge-site evidence
are committed as `34af812`; selection and focused execution as `91b8e4e`.
The [plan](MIR65816_SINGLE_WORD_EDGE_COPIES_PLAN.md) retains the forecasts; the
[qualification record](abi/action65816-single-word-edges-qualification.json)
binds the results to source, tools and saved machine-code artifacts.

## Selection and contracts

An eligible exact-word edge with one assignment now loads the complete source
into A16 and stores directly to its successor's home. This removes the staging
STA/LDA pair. Immediate and stack sources use the existing checked word operand
rules, including authoritative mutable parameter homes. Self-copies still load
and store. All preflight checks, including validation of unused staging storage,
run before any code or mode change.

Known A16 needs no mode instruction; A8 or unknown knowledge retains REP #$20.
The typed JML remains. A stack-source transfer takes eight bytes / 14 cycles;
an immediate transfer takes nine bytes / 12 cycles. REP adds two bytes and
three cycles. Independent ca65 probes verify encodings, overlapping source and
destination words, full A/N/Z results, preserved C/V/X/Y/S/D/DBR/I and exact bus
traffic. Interrupt probes preserve A while it holds the captured word.

Physical ABI v1, image v3, the o65 profile, stack guards, frame allocation and
storage maps are unchanged. Four-byte staging slots remain reserved but all
four bytes stay untouched by direct transfers. Multi-value word edges still
capture all sources before assigning destinations; mixed/unsupported edges and
empty edges retain their previous paths. No DP allocation, new helper, push,
source-memory reordering or cross-call register lifetime is introduced.

## Measurements

The immutable baseline is the [empty-edge snapshot](benchmarks/65816-empty-edges/after/tables.md),
emitted at `3dd5cb2` and qualified at `192b6d8`. Its 56 builds, 224 file hashes,
264 matching host records and saved snapshot hashes were rechecked before work.
See the [new snapshot](benchmarks/65816-single-word-edges/after/tables.md) and
[complete delta](benchmarks/65816-single-word-edges/delta.md).

| Optimized kernel / input | Bytes before / after | Cycles before / after | Stack reads before / after | Stack writes before / after | Unchanged stack peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| sum_loop(13) | 154 / 146 | 1,735 / 1,595 | 273 / 245 | 216 / 188 | 16 |
| byte_sum($12FFFC,16) | 252 / 244 | 4,646 / 4,476 | 624 / 590 | 523 / 489 | 22 |
| loop_rotation(13) | 186 / 186 | 1,314 / 1,314 | 201 / 201 | 178 / 178 | 26 |

All forecasts match exactly. Four static sites in two optimized builds select
direct copies. All ten independently declared vector counts match: 92 executed
copies per incoming I state, each removing two instructions, ten cycles, two
stack-byte reads and two writes. All raw and other optimized Action artifacts
are byte-identical. Frame maps, guard costs, stack peaks, DP traffic, fusion counts
and total word-edge/word counts remain unchanged. LF/CRLF corpus builds match.

The [instruction-stream checker](../tools/compare65816/check_single_word_edges.py)
requires that only those staging pairs disappear, with relocated JSL/JML targets.
The dedicated delta option accepts only the predicted traffic reduction; default
and earlier accounting modes stay strict. Mutation tests reject wrong counts,
operands, targets, unrelated changes and invalid exceptions.

Both external comparison host commands save all 264 records and retain the known
optimized vbcc unlink vector-0 failure in both I states. All vbcc records and
generated code remain identical. Its raw assembler listing differs only in the
source-header build path; the checker verifies every other byte. The existing
failure is reported without an exemption.

## Execution and qualification

Direct-edge decoding uses verified MIR and typed machine fixups to identify
eligible sites, then checks the actual emitted bytes at reached boundaries.
Ordinary load/store/jump sequences and staged-copy suffixes are not treated as
new direct edges. This test-only evidence is separate from the independent
result/count oracles and never participates in CPU execution. Corpus evidence
requires an exact match between the saved image and a recompilation with its
recorded source, mode and layout. No public format changes were needed.

Focused execution covers immediate, temporary, immutable and mutable parameter
sources, Goto, countdown backedges, ordinary/fused branch arms with a common
target, live-ins and word boundaries. Exact traces verify direct destination
writes and untouched staging canaries; existing staged cycles, rotations,
repeated/unused parameters and mixed-width fallbacks remain qualified. Volatile
and aliased bank-crossing captures still cross checked edges and clobbering calls
with unchanged external traces.

The new IRQ probe covers **34 task/PC sites per mode**, including direct-copy
instruction boundaries and successor entry, both branch outcomes and both task
domains. Each restoration is checked against an uninterrupted reference CPU
step, including full registers and invocation frame contents. Existing targeted
coverage retains 300 sites, all 222 cyclic word-copy sites and all 24 post-CMP
truth/task combinations. General coverage remains 2,245 raw / 2,087 optimized
enabled addresses. Both seeded IRQ/NMI schedules pass.

The new o65 probe executes direct immediate/stack edges and a countdown backedge,
with ordinary/fused branch arms, after serialization and relocation at $100000
and $600000. All five static sites are reached; 48 machine executions cover both
compiler modes and I states, checking exact copy cycles, full-word results,
staging canaries and relocated targets. Existing multi-word relocation probes
remain green.

Full native qualification passes **78 tests in each host build**, with **270
identical artifacts**. Compiler checks pass 36 native library tests and 58
ABI/emission/o65/CLI integration tests. Python comparison tests pass 11 cases;
disassembler tests pass five. No NIR, semantic or allocator contract changed, so
the full root suite and NIR sweep were outside this slice. Evidence is corrected
pinned VM execution; board and Exec816 loader qualification remain separate.
