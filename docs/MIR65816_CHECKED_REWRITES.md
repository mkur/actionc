# Native 65816 checked rewrites

Slice 7 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
adds a private transaction driver adapted from MIR6502's immutable proof contexts,
generation-bound plans and effect declarations. Slice 8 migrates
[adjacent temporary-load forwarding](MIR65816_ADJACENT_CHECKED_FORWARDING.md)
as its sole production consumer, preserving existing eligibility and output.

## Transaction contract

`rewrite/context.rs` wraps the existing immutable analysis snapshot in
`Proof<T> = Proven(T) | Blocked(reason, site)`. Home, stored-definition,
register-lane and flag queries validate owner, allocation generation, selection
generation and reachability. A narrow local equivalence query replays the prefix
from native entry, checks the exact adjacent temporary/home/generation and
A16/N/Z identity, then executes the actual proposed LDA on a scratch tracker.
It checks equal A/X/Y/N/Z/C/V, home contents, environment and X obligations.
No unknown value is treated as proof of accumulator equality.

Plans are sealed within the rewrite module tree. They identify an exact original
window and its records, replacement instructions, removed physical definitions
and register/flag deltas. The driver recomputes effects and declarations; a
declared change never grants permission. The production rule removes one
original private temporary LDA after proving full A/N/Z equivalence. Identity
replay and test-only controls exercise transactions; they do not add an
optimizer switch or broaden eligibility.

Windows contain contiguous top-level instructions in one selected block.
Compiler events, nested request instructions, calls, control transfers, barriers,
stack/mode changes and reservation changes are protected. Closed rule checks
must justify replacement reads and live-state equivalence. Even complete
declarations or dead destinations cannot authorize an arbitrary replacement.

Application validates the original Code, plan and analyses before constructing
scratch actions. Symbolic request links are reindexed and the selection
generation advances. Fresh typed replay regenerates bytes and metadata, and
layout/reconciliation and rebuilt dataflow facts complete before the single
publication point. A blocker leaves the original Code, actions, sites and observations
unchanged. No panic-catching implements rollback. Existing assertions remain
inside the tracked facade, behind the closed admitted rules.

Each successful edit invalidates every previous site/plan, including disjoint
windows. A caller must rediscover against the current generation. Transactions
have an explicit application limit; non-identity edits must reduce finalized
byte length. Identity is one-shot and is never iterated to a fixed point.
Attempted/applied counts and deterministic blocker reasons are separate from
historical executable measurements.

Home analyses share immutable states across unchanged sites within one solver
run and copy on writes/joins that change facts. ABI-defined inputs begin outside
the possibly-undefined set; they never create synthetic store definitions. This
avoids duplicating unchanged sets at every compiler event. Each generation still
gets entirely rebuilt analyses; no fact survives an edit through a cache.
Structural candidate discovery leaves the driver responsible for the full proof,
avoiding a second identical analysis build merely to author the proposal.

## Slice 7 qualification

The [qualification record](abi/action65816-checked-driver-qualification.json)
records focused root/native checks and exact input hashes. Nine new unit tests
cover publication and generation invalidation, foreign owners, reversed windows,
changed originals, two disjoint stale plans, undeclared register/flag changes,
replacement reads, protected events/modes/calls, live stored definitions,
nondecreasing/exhausted transactions and complete rollback after replay failure.
The local proof also rejects mismatched identity, stale cursors and non-home
loads. Existing selected-site tests cover allocation/routine/bounds rejection.

The replay-failure control deletes a NOP which previously made a capture stale.
The later consume decision would change; fresh replay rejects it and the
original output remains intact. Existing native replay, CFG and machine-liveness
observations remain identical. Full execution/corpus/CRLF qualification follows
when the pilot becomes authoritative, as required by the implementation plan.
