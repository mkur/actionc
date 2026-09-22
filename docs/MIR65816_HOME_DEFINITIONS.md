# Native 65816 stored-definition analysis

Slice 4 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
adds reaching stored definitions, read attribution and possibly undefined
private reads. It adapts MIR6502's home-definition analysis to the native
selected CFG and [canonical physical bytes](MIR65816_HOME_ANALYSIS.md), using
the shared forward dataflow solver. Emission and allocation are unchanged.

## Definition and entry contracts

A stored definition is `(HomeByte, selected write site)`. Different temp names
for a reused byte do not create separate homes; different stores to that byte
remain separate definitions. At joins, possible reaching definitions are united.
A definite write replaces only the bytes it writes. A may-write preserves old
candidates and adds uncertainty, without proving initialization. A later definite
write clears that uncertainty for its own bytes.

Entry state seeds ABI-defined inputs from the verified home contract, separately
from compiler stores. Private bytes without an entry value start possibly
undefined. That possibility survives a join if any incoming path leaves them
undefined. ABI entry values are not invented store sites.

Ordered sub-effects attribute each read before applying later writes. RMW reads
the previous stored value, then defines its result. Call arguments are read
before clobbers; a return-frame write is visible to its subsequent internal read.
Unknown target reads expand to the full potentially aliased home universe.

## Checked read-only queries

The immutable analysis snapshot exposes:

- `uses_of_definition(home, store)`: read sites and sub-effect indices, including
  uncertainty from unresolved aliases or intervening may-writes.
- `definition_dead_outside_window(home, store, end)`: whether a particular private
  stored definition has no reads outside an inclusive selected-site window.
- `undefined_private_reads()`: exact or uncertain reads with a possibly
  uninitialized incoming private byte.

Queries validate snapshot ownership, generation, reachability and bounds. A
store query requires one exact write to that byte at the selected site. No
write, a may-write alone, or several internal writes with the same coarse site
identity blocks the query rather than returning an empty successful proof.

Outside-window proofs currently require a reachable, single-entry straight-line
window and a private home. Protected storage, uncertain reads/effects and
nonlinear windows block that proof. A read before its own static write site can
reach the same definition through a loop; it consumes a previous iteration and
cannot be excused as a local read of the proposed window's new value.

An outside-window result is **not permission to remove a store**. A replacement
still owes window-local reads their values and must preserve definedness,
observable effects, registers, flags and the execution environment. The future
checked-rewrite driver must analyze and validate the replacement. These queries
do not change accepted programs or make conservative undefined-read observations
into compiler diagnostics.

## Qualification and limits

Fourteen new independent graph tests cover distinct stores, intervening reads,
conditional initialization, partial aliases, may-write uncertainty, unknown
reads, ordered calls and RMW, ABI inputs, loop iterations, invalid/protected
windows, multiple internal writes and newly introduced replacement reads.

The [raw/optimized inventory](benchmarks/65816-analysis-rewrite/home-definitions-summary.json)
covers all 28 Action corpus builds: 702 stored byte definitions and 992 attributed
reads. There are zero exact possibly undefined private reads. The 5,146 uncertain
observations include conservative fault, call and unresolved-pointer aliases;
they do not establish uninitialized program behavior. The inventory's singleton
window counts are analysis observations, not store-removal or code-size forecasts.

The [qualification record](abi/action65816-home-definitions-qualification.json)
identifies the tested source hashes, scoped native debug/release checks and
[unchanged executable output](benchmarks/65816-analysis-rewrite/slice4-equality.json).
The prior liveness inventory and selected-CFG VM observations remain identical.
No semantic/NIR, ABI, runtime, guard or fixture-text handling changed. Full
root/NIR and unrelated native suites were not repeated. Analysis runtime and
memory costs remain unmeasured; register/flag liveness and checked rewrites are
the remaining foundation work.
