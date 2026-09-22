# Checked adjacent temporary forwarding

Slice 8 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
migrates the temporary-home choice in `Builder::load_checked_word` to the
[checked transaction driver](MIR65816_CHECKED_REWRITES.md). This first commit
qualifies shadow decisions; the historical predicate still controls selection.

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

For each successful shadow candidate, the original typed LDA is materialized in
a scratch selected routine. Home/definition/machine analysis therefore sees the
actual read. The sealed rule removes exactly that load; its declared A/N/Z
effects are recomputed and justified by equivalence. The driver replays and
finalizes the replacement, and compares every existing output field and trace
with the historical path. Neither a saved success token nor an already-omitted
stream constitutes proof of read removal.

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

Authoritative integration, full VM execution, the frozen corpus gate and isolated
CRLF qualification remain required before slice 8 is complete.
