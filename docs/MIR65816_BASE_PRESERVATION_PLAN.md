# Preserve prepared pointer bases across ordinary stores

`NewList` currently stages the same incoming pointer three times. Each staging
costs eight bytes. The target is one staging, without changing public memory
accesses, scratch allocation or the general physical-memory alias analysis.

## Ownership and limits

The [native ABI](MIR65816_PHYSICAL_ABI_V2.md) already owns compiler scratch and
unexposed invocation storage. The pointer-leaf allocator relies on this same
ownership when keeping live pointer temporaries in DP across ordinary stores.
Ordinary program objects cannot occupy live compiler scratch or unexposed
incoming/temporary homes. This is an ownership contract, not memory protection.
Address-taken local objects remain normal aliasable program storage.

Preserve a prepared base only for a checked, nonvolatile, unindexed indirect
scalar store with a bounded field displacement. Prove that its authoritative
source is an owned stack temporary, immutable incoming argument, or unexposed
local home. Match the exact source identity, home, scratch triplet, current
stack depth and instruction's write extent. Absolute/symbolic destinations,
volatile accesses, dynamic indexing and unproved sources retain invalidation.
Pointee loads must still observe intervening writes and calls.

## Implementation slice

1. Derive a sealed store contract from verified MIR and final allocation after
   borrowed pointer bindings are resolved. Record it as a typed request scoped
   to that one MIR operation. Replay checks the site and reconstructs decisions.
2. Let the existing pointer tracker keep its resident fact for matching writes
   within the checked store. All real instruction effects remain unchanged;
   unknown writes still alias all homes in physical liveness/definition analysis.
   Never restore a discarded fact after a barrier. Writes to source/scratch,
   calls, control transfers and domain/stack transitions still invalidate it.
3. Test one staging across scalar and pointer field stores, exact stores and
   observable re-reads, source mutation/escape, volatile/raw access fallback,
   scratch clobbers and mismatched contracts. Cover raw/optimized emission,
   replay, relocation and interrupt/task reentry with the affected native tests.
4. Measure NewList and the same frozen Exec inputs, validate frame maps, record
   the result, and commit this slice. The estimated NewList saving is 16 bytes;
   measure the final code rather than assuming the estimate.

No compiler pin or play-image refresh is part of this slice. Reserved bank-zero
delta: **0 fixed bytes and 0 bytes per task**, including guards, alignment and
unused reserved capacity.

## Result

Implemented with typed operation-scoped store contracts and unchanged generic
alias effects. NewList is **63 bytes**, down from 79. The same Exec inputs save
**6,178 bytes across 242 routines**, with no growth or frame-map changes. The
[measurement](benchmarks/65816-base-preservation/README.md) records the listing,
provenance and focused backend/native validation.
