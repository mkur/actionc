# Core layout dependency census — 2026-11-13

This slice adds opt-in diagnostics to identify which core rewrite rounds consult
planning addresses. It changes no optimization decision and does not permit
frame compaction. The existing late-pruning experiment remains disabled.
The [design contract](../../MIR6502_LATE_CLEANUP_DESIGN.md#core-allocation-dependency-census)
defines the diagnostic boundary and its limitations.

## Interpretation

`core-layout-round` records a routine and rewrite generation, the actual applied
sites, and a deduplicated set of address queries/predicate observations. The
association is a **conservative discovery-round envelope**, not a list of
necessary predicates for each applied rewrite. Shared analysis and rejected
candidates can contribute queries. A round that applies no rewrite is recorded
separately. Applied rewrites inside speculative materializations are local
results; this census does not infer that they survive parent selection or later
cleanup.

Raw queries report the storage ID, byte offset and optional planning address.
Annotated predicates report their boolean result and operand ranges: address
equality, range membership, disjointness and above-zero-page/no-wrap. Unknown
addresses remain explicit. Query source locations are diagnostic metadata, not
executable identities or replay keys.

The observer covers the three planning layouts created by core materialization:
initial, post-home and post-ZP. Queries outside the shared rewrite drivers are
reported as `core-layout-unattributed`, with no inferred acceptance. Separate
constructor, captured-comparison and scalar-capture transactions own additional
layouts; they remain outside this dynamic scope. Unannotated query sites and
unattributed observations are coverage gaps, never evidence of independence.

## Findings

All 16 complete API comparisons and 54 frozen CLI artifacts are unchanged.
TN remains **9,970 bytes cart / 10,077 standalone**. Its three materializations,
43 emissions, 258 inner emission probes and 24,926 rewrite-analysis builds are
identical with the census off/on. All recorded scopes complete without truncation.

The following counts repeat in both runtimes and at `$3000`/`$41C7`. They count
local discovery rounds across every core materialization, including speculative
ones; they are not counts of final surviving optimizations.

| Full source case | Rounds with address observations | Those applying a rewrite | Those also querying planning storage |
| --- | ---: | ---: | ---: |
| TN | 1,113 | 138 | 123 |
| Word-load-sharing control | 21 | 1 | 1 |
| Argument-staging control | 34 | 0 | 0 |
| Invariant-pointer-loop control | 5 | 0 | 0 |

For TN, the largest applied group querying planning storage is
`known-callee-word-result-placement`: **42 rounds** across the three core
materializations, including `Alloc`. Direct pointer-temp rematerialization
accounts for 27 rounds. Examples in the receipt show ordinary locals/params
compared with `$A0/$A1` return slots and `$AC/$AD` pointer scratch. Other recorded
relations include ordinary-to-ordinary homes, so a blanket ordinary-versus-ZP
rule cannot cover the whole pipeline.

The 123 rounds contain 1,283 resolved and 306 unresolved planning-address query
evaluations. Unknown results must remain separate from resolved addresses.
Annotated predicate observations in these round envelopes comprise 627
disjointness, 342 equality and three range-membership evaluations. Their counts
include repeated analysis and rejected candidates; none is an exact count of
obligations that must survive compaction.

The word control has one applied `known-pointer-local` round with raw queries
but no annotated predicate. The staging and loop controls perform address
queries without applying a rewrite in those same rounds; their other core/late
optimizations are outside this count. An additional independent
[word-return control](../../../fixtures/runtime/core_layout_census.act) positively
exercises the equality observations in known-callee result placement. Its test
disables leaf inlining in both runs to keep that call boundary visible, covers
both runtimes/origins and LF/CRLF, and compares full same-source artifacts and
cross-line-ending executable bytes.

Unattributed queries remain significant, especially legacy SSA-lite processing
and final machine-value reload folding. Separate constructor/capture layouts
also remain uncovered. Their absence from accepted-round counts grants no
permission to move storage.

The next narrow investigation should start with **known-callee word-result
placement**. First determine which fixed-return-slot checks can follow from
typed storage identity, complete object bounds and ordinary storage being above
ZP. Retain explicit predicates for the remaining address relations. This is a
candidate for simplifying dependencies before expanding replay machinery; the
census does not establish that this alone would unlock TN compaction.

## Static dependency inventory

These distinctions follow from the current query consumers, independently of
whether a particular corpus run exercises them:

| Consumer | Address fact used | Census detail |
| --- | --- | --- |
| SSA-lite memory/address invalidation | Equality with separately named physical bytes; source separation from pointer scratch | Queries and equality/disjointness observations |
| Staged word/indirect-store and known-result placement | Source/destination separation from fixed scratch, return slots and other homes | Queries plus selected equality, range and disjointness observations |
| Widened byte-shift store selection | One source byte disjoint from two destination bytes | Disjointness observations |
| Predicate argument capture/staging | Ordinary source above ZP and complete byte/word range without wrap | Bounds observations |
| Parameter-address forwarding and preserved-register carriers | Non-overlap with intervening writes and exact callee read/write footprints | Queries; complete relational/callee obligations remain unrecorded |
| Machine-value memory maps and register-reload folding | Physical byte identity/alias relations across the map | Map-input queries; legacy reload folding is unattributed |
| Indexed recovery and counted-loop carriers | Full indexed range avoids scratch/induction storage and does not wrap | Queries; range predicates remain unrecorded |
| Pointer-pair selection | Low-byte address fits a complete ZP pair | Queries; an exact selected address can matter |
| Unique word-load forwarding | Complete ordinary word above ZP without wrap | Queries outside shared drivers |
| Early LEA/address constant formation | Exact address, currently admitted only for `MirMem::Absolute` | Fixed-address queries; ordinary relocatable addresses remain symbolic |
| Source constructor/capture/comparison transactions | Private-cell alias exclusions, fixed destination ranges and historical storage | Separate layout owners; outside this initial dynamic census |
| Layout width/descriptor/storage-class queries | Typed shape rather than an ordinary numeric address | Not an address observation; unchanged shapes are a separate invariant |

The inventory explains why raw address equality is too coarse to become the
long-term compaction contract. It does not establish which predicates can be
discarded after later rewrites remove their consumers.

## Reproduction

For a focused CLI bundle:

```sh
actionc-emit --profile modern --backend mir6502 --emit-load \
  --trace-mir6502 build/tn-core-layout --trace-core-layout \
  --trace-routine FormatDirectoryRow --trace-routine SetWin \
  samples/tn/modern/TN.ACT > build/tn-core-layout.xex
```

The trace destination must be fresh. Inspect `manifest.json` and `events.jsonl`.
Both the overall trace status and `core_layout_census.status` must be complete
before interpreting the recorded scope. Collection caps are explicit and cannot
silently turn omitted facts into proof of independence.

The [qualification harness](../../../tools/compare6502/core_layout_census.py)
checks 54 frozen default CLI artifacts and invokes the
[API probe](../../../tools/compare6502/core_layout_census.rs) on TN plus three
independent controls, both runtimes and two origins. It compares complete output
and compiler work with the census disabled/enabled, then summarizes observations:

```sh
python3 tools/compare6502/core_layout_census.py \
  --compiler build/checked-arena/shared-target/debug/actionc-emit \
  --output build/mir6502-layout-census/qualification
```

The frozen baseline is `2c6b8edf`. Generated bundles stay under ignored `build/`;
the committed [receipt](qualification.json) contains summaries, examples,
source/compiler identities and validation commands.

Implementation: `d1ca2cb9`. Validation passed 1,317 MIR6502 unit tests, 134
distinct integration tests and 31 VM tests. Fourteen unit, eleven integration
and one VM optional checks remained ignored. The new integration target was
rerun after replacing its initial fixture, which had raw query observations but
no annotated predicate in an applied round; all three new tests then passed.
The other 131 integration tests already passed and were not repeated. Existing
byte/listing/map artifacts remain identical. No shared frontend/NIR or emission
contracts changed.
