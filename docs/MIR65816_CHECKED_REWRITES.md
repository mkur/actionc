# Native 65816 checked rewrites

Slice 7 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
adds a private transaction driver adapted from MIR6502's immutable proof contexts,
generation-bound plans and effect declarations. It introduces no production
optimization. Adjacent temporary-load forwarding is the next consumer.

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
declared change never grants permission. The initial production rule set is
empty. Identity replay and a test-only NOP deletion exercise transactions; they
do not add an optimizer switch or alter ordinary output.

Windows contain contiguous top-level instructions in one selected block.
Compiler events, nested request instructions, calls, control transfers, barriers,
stack/mode changes and reservation changes are protected. Closed rule checks
must justify replacement reads and live-state equivalence. Even complete
declarations or dead destinations cannot authorize an arbitrary replacement.

Application validates the original Code, plan and analyses before constructing
scratch actions. Symbolic request links are reindexed and the selection
generation advances. All analyses are rebuilt, fresh typed replay regenerates
bytes and metadata, and layout/reconciliation run before the single publication
point. A blocker leaves the original Code, actions, sites and observations
unchanged. No panic-catching implements rollback. Existing assertions remain
inside the tracked facade, behind the closed admitted rules.

Each successful edit invalidates every previous site/plan, including disjoint
windows. A caller must rediscover against the current generation. Transactions
have an explicit application limit; non-identity edits must reduce finalized
byte length. Identity is one-shot and is never iterated to a fixed point.
Attempted/applied counts and deterministic blocker reasons are separate from
historical executable measurements.

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
