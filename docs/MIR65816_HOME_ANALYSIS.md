# Native 65816 physical home analysis

Slice 3 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
adds canonical physical bytes and backward home liveness. These are read-only
analyses; selection, allocation, stores, ABI v1 and stack guards are unchanged.

## Identity and ownership

`HomeByte::Stack(offset)` is relative to invocation-entry S. An instruction's
stack displacement becomes `displacement - depth_before`; allocated body homes
become `offset - allocated_extent`. Outgoing arguments and transfer pushes use
the same coordinates. DP bytes are relative to the current execution domain.
Multiple temp, staging and frame-object names can identify one physical byte.

The selected snapshot captures ownership from verified MIR frame plans and the
checked allocation. Temp and staging bytes, non-addressable frame objects and
ABI scratch are private. Incoming arguments, return addresses, addressable
objects, domain metadata and unowned stack bytes remain protected. A protected
owner wins if ownership overlaps. Incoming logical arguments, return addresses
and domain metadata are defined by the entry contract; padding is not.

These coordinates rely on ABI v1's disjoint, nonwrapping bank-zero stack and DP
reservations, checked stack peaks and page-aligned D. Preemption preserves the
domain's stack and DP contents. The analysis does not add an asynchronous
scratch clobber. Ranges outside those contracts, wrapping ranges or accesses
without a current domain remain unresolved. Absolute addresses, symbolic
addresses and indirect targets conservatively alias every tracked byte;
numeric addresses do not establish private ownership or disjointness.

## Ordered accesses and liveness

The shared backward solver follows every reachable selected-CFG edge. Reads
add bytes, definite writes kill those bytes and may-writes never kill. Sub-effects
are composed in reverse order: a read before a write needs the old value, while
a call's internal read of its newly written return frame does not. Pointer
address bytes remain explicit reads before the unresolved target access.

Normal exits observe protected homes; results themselves use the native
register contract. The terminal raw fault adapter has no narrower observation
contract, so its exit conservatively reads the universe. Unknown accesses expand
to that universe and carry uncertainty; an unresolved write becomes a may-write.
This policy deliberately overestimates liveness around calls and pointer access.

`AnalysisSnapshot` borrows an immutable selection. Construction resolves the
fallible home-access model; private per-snapshot cells compute home liveness,
stored definitions and machine liveness only when queried, once each. Site
validation precedes solver demand. An uncomputed result never means dead, empty
or safe, and a new selection generation begins with empty cells. No result is
transferred across an edit.
`home_live_before/after` and the proof-feature observations validate snapshot
ownership, generation, bounds and reachability. No optimizer consumes them yet.
Synthetic selected probes without a verified storage contract cannot claim home
facts. Byte/fixup layout remains separate from home and selected-site identity.

## Qualification

Nine new unit tests use independently authored graphs/access sets and checked
range examples, covering diamonds, loops, unreachable sites, partial writes,
RMW, ordered calls, reused names, S movement and DP/wrap boundaries. The native
home probe checks all 28 raw/optimized corpus selections, foreign/unreachable
queries, entry provenance, reused temp bytes, machine-encoded indirect pointer
reads and call ordering. Existing selected-CFG VM probes cover actual loop and
guard paths.

The [slice-3 qualification](abi/action65816-home-liveness-qualification.json)
records scoped debug/release checks and
[full output equality](benchmarks/65816-analysis-rewrite/slice3-equality.json).
No NIR/semantic, runtime or fixture-text handling changed; full root/NIR and
unrelated native suites are outside this slice. Analysis overhead has not been
benchmarked. [Stored-definition queries](MIR65816_HOME_DEFINITIONS.md) were added
in slice 4; rewrite permission remains subsequent work and does not follow from
whole-home liveness alone.
