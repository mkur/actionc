# Ordinary word-result destination proof

This slice replaces the destination address checks in known-callee word-result
placement with a reusable typed-storage proof when the destination is a complete
inline ordinary scalar range. The proof uses declaration identity, range bounds,
an origin above zero page, and a conservative total-storage budget. It does not
compute individual planning addresses. Unsupported shapes retain explicit
address checks; unresolved fallback addresses now reject.

The [design contract](../../MIR6502_LATE_CLEANUP_DESIGN.md#ordinary-word-result-destination-proof)
records admitted roots, the no-wrap bound, fallback and supporting-analysis audit.
There is no new optimizer pass or strategy search. Frame compaction stays disabled;
terminal cost acceptance, rollback and surviving-address guards remain intact.

## Measurement

Against `9149ed66`, all **54 frozen CLI artifacts** and **20 full API outputs**
are unchanged. All 20 trace-off/on comparisons also preserve full output and
compiler work. TN remains **9,970 bytes cart / 10,077 standalone**, with **zero
additional image saving**. It still uses three materializations, 43 emissions,
258 inner emission probes and 24,926 rewrite-analysis builds.

These family-specific counts repeat in both runtimes and at both origins:

| Case | Applied rounds before/after | Numeric queries before → after | Equality checks before → after | Successful structural evaluations after |
| --- | ---: | ---: | ---: | ---: |
| TN, including `Alloc` | 42 / 42 | 324 → 0 | 270 → 0 | 54 |
| Independent word return | 1 / 1 | 6 → 0 | 5 → 0 | 1 |
| Word-load-sharing, staging, pointer-loop controls | 0 / 0 | 0 → 0 | 0 → 0 | 0 |

The last row concerns this family only: those controls still exercise other
layout consumers. The [receipt](qualification.json) records per-routine counts,
image/full-output hashes, compiler identity, work and validation.

Counts concern local applied discovery-round envelopes across all materializations,
including speculative ones. Queries can include rejected candidates; these are
not counts of surviving instructions, necessary per-rewrite obligations or saved
bytes. A structural observation with a null address is not an unresolved numeric
query.

## Scope and remaining dependencies

The fast path covers directly declared scalar globals, locals and parameters
with complete bounds. Aliases, descriptors, deferred arrays, spills, ABI-only
parameters, fixed/virtual ZP, absolute storage and unknown roots gain no new
structural proof. Existing store-reordering permission is still required.

The total-storage budget deliberately overcounts initialized objects and includes
deferred data. If it cannot establish no wrap, this family falls back to explicit
checks. The helper is a materialization-time fact, not a persistent proof witness;
allocation changes require rebuilding it. The allocation equality guard includes
the new facts, accounting for the exact one-byte spills it already permits removing.

The production driver and MIR callee exit analysis use empty physical-memory maps.
The post-call accumulator fact, register/flag liveness and structural-plan checks
have no direct planning-address dependency. Earlier transformations of callee
bodies and other rewrite families remain separate preservation obligations.
No general compaction certificate follows from eliminating this family's queries.

## Reproduction

The [API probe](../../../tools/compare6502/word_result_proof.rs) captures all
`CodegenOutput` fields as debug snapshots, executable images, complete traces and
compiler work. It runs TN, word-load-sharing, argument-staging, invariant-pointer-loop
and the independent word-return fixture in both runtimes at `$3000` and `$41C7`.
The word-return fixture disables leaf inlining identically in both versions to
retain the call boundary. Other cases use the ordinary optimized configuration.

Before rebuilding, compile the probe against the baseline library and capture it:

```sh
rustc --edition=2024 tools/compare6502/word_result_proof.rs \
  --extern actionc=/path/to/baseline/libactionc-HASH.rlib \
  -L dependency=/path/to/baseline/deps -o build/baseline-word-result-probe
build/baseline-word-result-probe build/word-result-baseline
```

After building the changed compiler, run the
[qualification harness](../../../tools/compare6502/word_result_proof.py):

```sh
python3 tools/compare6502/word_result_proof.py \
  --baseline-api build/word-result-baseline \
  --compiler build/checked-arena/shared-target/debug/actionc-emit
```

The harness checks 54 frozen CLI image/listing/map artifacts and 20 complete API
outputs against the baseline, plus 20 trace-off/on comparisons. The raw bundles
stay in ignored `build/`; the committed receipt contains hashes and measurements.


## Validation

Implementation: `51915136`. Validation passed **1,323 MIR6502 unit tests**,
**134 integration tests across 30 targets**, and **32 VM tests across 14 targets**.
Fourteen unit, eleven integration and one VM optional checks remain ignored.
The focused integration targets passed first; the other 26 targets ran separately
without repeating the passing cases. Initial new unit-fixture setup errors were
corrected before the passing full unit run; no production correction was needed.

The new word-return VM test executes 48 combinations of values, runtime, origin
and LF/CRLF input. It checks wrap/carry boundaries, final memory and the actual
high-lane-first write sequence. Existing TN allocator/dispatch/reproducer tests,
call ABI, pointer, staging, late-cleanup and pruning controls also pass.

CLI hashes use the frozen `2c6b8edf` corpus already qualified unchanged at
`9149ed66`; the full API baseline was captured directly at `9149ed66`. No emission
code changed and all frozen listings are identical, so optional MADS roundtrips
were not repeated. No shared frontend/NIR contracts changed.
