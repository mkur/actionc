# Checked adjacent temporary forwarding

Slice 8 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
migrates the temporary-home choice in `Builder::load_checked_word` to the
[checked transaction driver](MIR65816_CHECKED_REWRITES.md). Shadow qualification
was committed separately as `4130d858`; checked transactions now control production
load removal. The historical predicate remains only on the proof-feature direct
reference path and in the unchanged frame/incoming mechanisms.

## Candidate and proof

Before asking to omit a load, the planning facade retains a typed candidate:
the consume-request site, logical temporary, allocated home and actual LDA
instruction that would otherwise execute. This includes failed attempts and
immediate loads. Frame and incoming-parameter forwarding remain separate.

The rule requires the exact private two-byte temporary ownership, A16 mode,
unmoved stack, retained producer/home generation, unchanged instruction/label
cursor and full A/N/Z identity. It executes the actual LDA from a freshly
replayed prefix and checks equal machine/home/environment/X facts. Independent
flag deadness cannot relax the existing full N/Z requirement. The consume event
remains in both windows, preserving successful and failed single-use semantics.

For every successful projection, the original typed LDA is materialized in a
scratch selected routine before the checked rule runs. The sealed rule removes
exactly that load; its declared A/N/Z effects are recomputed and justified by
equivalence. The driver replays and finalizes the replacement; qualification
compares every existing output field and trace with the historical path.
Neither a saved success token nor an already-omitted stream proves read removal.

No stores, homes, preflight, X fallback priority, stack guards, call/alias rules,
ABI or preemption contract change. The additional observations are separate
from historical image, trace and corpus records.

## Shadow qualification

The [shadow qualification](abi/action65816-adjacent-shadow-qualification.json)
records 173 native library tests and four scoped native tests. Two new driver
tests cover the actual load transaction and forged partial-home, stale-capture,
foreign-site and undeclared-effect plans. The corpus inventory covers 102
requests in 28 raw/optimized builds: 75 accepted and 27 blocked, with no decision
mismatch. All direct/replay bytes, metadata and traces remain identical.
Additional raw/optimized forwarding, frame, parameter, call/alias and preemption
fixtures retain exact output; the existing deterministic replay suite passes.

## Authoritative planning and publication

The planner first retains each candidate, then tests local A16/home/NZ
equivalence independently of the old temporary predicate. It consumes the
single-use witness even on failure. A proved-equivalent projection may guide
subsequent selection, including the existing X fallback priority, but its
provisional bytes cannot be published.

Before final emission, the complete original planned stream restores every
projected-away LDA in one traversal. One ordinal map reindexes request/end/parent
links, and one constructor verifies the resulting CFG. Candidates must match the exact original consume inputs,
occur once in order and retain their actual addressing forms. Fresh replay
validates the original continuation. Each load then goes through the sealed
adjacent rule and atomic driver. The adapter does not repeat the selector's
local proof. Failed-selection diagnostics are retained solely for observations.
The driver builds fresh required facts; rediscovery validates the exact consume
inputs and actual load before minting a site in the current generation after
every accepted edit. The number of candidates
bounds application; each accepted edit reduces finalized bytes.

A blocked final ownership/equivalence proof retains the actual load and its
already-verified continuation. No old success token authorizes omission. The
fallback is tested by withholding temporary allocation identity after planning;
the emitted load is restored and the original routine's bytes/metadata match.
Other controls reject altered load operands, duplicated candidates and corrupted
Code. Modes, captures and X obligations are freshly recomputed during replay.

The [authoritative qualification](abi/action65816-adjacent-checked-qualification.json)
records full native execution, corpus equality and an isolated CRLF rebuild.
The [final foundation qualification](MIR65816_ANALYSIS_REWRITE_QUALIFICATION.md)
adds mutation controls and measured host compile-time/memory overhead.

The [simplification record](MIR65816_EMISSION_SIMPLIFICATION.md) documents the
single-pass reconstruction and removal of duplicate adapter proofs. The old
incremental reconstruction exists only as a test oracle. Full per-edit replay,
layout and definition postconditions remain in the production driver.
