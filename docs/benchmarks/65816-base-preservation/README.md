# Prepared-base preservation across ordinary stores

Checked ordinary field stores now preserve their prepared DP base. A sealed
contract ties the store to its MIR operation, private source identity/home and
field extent. The tracker checks each write without restoring invalidated facts
or narrowing generic physical-memory effects. See the
[implementation plan](../../MIR65816_BASE_PRESERVATION_PLAN.md) and
[emission contract](../../MIR65816_EMISSION_CONTRACT.md#prepared-indirect-base-reuse).

Baseline: actionc `090f99b9`, Exec `134130b`, and the same 133 generated
source/layout inputs as the [component-store measurement](../65816-component-stores/README.md).

| Measurement | Before | After |
| --- | ---: | ---: |
| Exec routine code, 960 routines | 478,620 bytes | 472,442 bytes |
| NewList code | 79 bytes | 63 bytes |
| NewList frame / local stack peak | 0 / 0 bytes | 0 / 0 bytes |

**6,178 bytes saved across 242 routines; no routine grows.** NewList stages
`chain` once, then uses the same `$80..$82` base for all three field stores.
The native execution test verifies exactly one staging sequence and nine field
writes, including bank-crossing objects. The [full listing](newlist.asm),
[routine table](exec-routines.csv) and [provenance](provenance.json) record the
generated code, all size changes, and compiler/input/image hashes.

All 960 frame maps pass Exec's validator. Frames, spills, stack peaks, argument
homes, object/temporary homes and call maps are unchanged. Reserved bank-zero
delta: **0 fixed bytes and 0 bytes per task**, counting guards, alignment and
unused capacity. Counts exclude data, alignment and container costs. This is
a compile-only local compiler override, not hosted-system qualification; the
compiler pin and play image remain unchanged.

Validation: 336 MIR65816 unit tests pass (one existing ignored), 39 emission,
o65 and state-boundary integration tests pass, and 19 native tests pass across
`address_consumers`, `pointer_forwarding` and `pointer_values`. Coverage includes
raw/optimized NIR, request replay, wrong-source/extent/scratch rejection,
operation-scope expiry, volatile/raw fallbacks, mutable/address-taken/public
pointer sources, exact accesses, relocation and IRQ/NMI/task reentry.

Reproduce with the same module paths and compile command as the
[address-consumer measurement](../65816-address-consumers/README.md), using
`/tmp/exec816-base-preservation.a816.json` as the output. The existing decoder
reads an in-memory version-3 envelope for the listing; the version-4 arithmetic
fault metadata does not alter instruction encoding.
