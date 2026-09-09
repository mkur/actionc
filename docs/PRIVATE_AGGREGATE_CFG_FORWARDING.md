# Cross-CFG private aggregate forwarding — slice 4

This extends slice 3's read-forwarding proof; it does not redirect producers.
The common CFG/dataflow solver now computes must-availability of the snapshot
image. Initialization establishes availability, intervening source/destination
writes or unknown effects kill it, and joins intersect predecessor facts. A
consumer must have availability on every incoming path. Loop reinitialization
can reestablish the image; a mutation on a backedge invalidates a later read.

The reference census still requires a sole complete initializing copy into an
unexposed ordinary capture. Fixed nested source fields are admitted by nominal
type and byte extent, not field identity: overlapping union views invalidate
reuse, while disjoint sibling writes do not. Mutable pointer relays/dynamic
source addresses and whole call/return values remain staged. Calls after the
last read, or on a different terminal path, no longer reject harmless reuse.

For `LET item=MaybeByte.SOME(42)` followed by CASE, construction now goes directly
to item and CASE validates/reads item without its former two-byte snapshot.
Required invalid-tag faults remain. The fresh construction region is still
10 instruction bytes / 12 cycles. Optimized MIR XEX size falls from 103 to 94
bytes with the cartridge runtime, and 386 to 377 standalone; cycles to PrintBE
entry fall from 56 to 46. Classic emission is unchanged. These figures exclude
printing execution and must not be compared with whole-program cycle totals.

The optimized `generic_types`, `nested_patterns` and `variant_match` NIR
snapshots intentionally lose redundant CASE captures and read the original
stable homes instead. Raw snapshots and the typed ABI contract are unchanged.
Native fresh-call variant probes can now have zero logical CopyBytes while
still requiring physical ABI transfers; baseline assertions distinguish these.

Four compiler tests exercise all four layouts, diamonds, backedges, repeated
initialization, byte overlap, nested aggregate binders and mutating guards.
Two VM tests check both branch outcomes, full union tails and the original
captured value after a guard mutates its selector. Existing bounded/fresh,
guard/nested-pattern and 288-execution corpus VM audits also pass. Historical
slice 1–3 CSV files remain unchanged; final costs are recorded in slice 6.
