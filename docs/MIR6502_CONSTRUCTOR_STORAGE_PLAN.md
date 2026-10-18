# MIR6502 unused constructor storage

## Objective

After captured-copy forwarding, remove unobserved constructor stores and,
under a separate proof, their allocated homes. Keep payload evaluation,
validation and result-buffer writes at their existing execution points.

```text
Before: capture payload -> write constructor temporary -> write result buffer
After:  capture payload -> write result buffer
```

This is MIR6502 target strategy. Existing verified NIR supplies aggregate
capture provenance; no source-name checks, new language semantics, changed
aggregate ABI or general relaxation of pointer/call barriers is intended.

## Commit slices

1. **Provenance and verification.** Carry existing
   `NirLocalPurpose::AggregateCapture` into typed MIR storage-purpose metadata.
   Verify ordinary, uninitialized aggregate local storage. Provenance selects
   candidates; it does not establish privacy, freshness or nonreentry.
   Test raw/optimized lowering, malformed storage and independence from names.

2. **Store elimination.** Extend captured-copy selection with a bounded proof
   for complete constructors of one through eight bytes in one terminal block.
   Require complete in-bounds captured-byte coverage and no other observation.
   Preserve evaluations and the final transfer. Prove the transfer's destination
   disjoint using structured addresses and a bounded census of direct callers
   passing known ordinary buffers. Hidden result pointers alone are not no-alias
   facts. Unknown callers, escaped routine addresses, uncertain pointer changes,
   aliases, calls/faults/volatile effects in the region retain the original stores.
   Initially retain allocations. Compare fully emitted cost against today's
   forwarding-only output, independently of forwarding's existing savings.

3. **Allocation elimination and qualification.** Separately check executable
   references, aliases, initializer relocations, frame exposure and final physical
   layout before removing a now-unused capture home. Keep store-only savings
   when allocation removal cannot be proved. Preserve Atari static-frame and
   reentry behavior. Qualify the final backend change and measure TN.

Each slice updates the relevant MIR contract, runs its focused checks and is
committed before starting the next slice.

## Proof and cost requirements

- Use stable routine/local/parameter/temp identities, checked byte bounds and
  complete source and destination extents. Never infer facts from display names.
- Start with the existing tiny-fill and captured-value forwarding analyses;
  constants and uniquely defined virtual values are snapshots, live register or
  pointer-cell reads are not.
- Require every removed store unobservable. Stored values and address calculations
  must retain their original evaluation, ordering and failure behavior.
- Retain conservative handling of callbacks, recursion, machine blocks, unknown
  effects, hardware/absolute accesses, and exposed neighboring storage.
- Account for direct, indirect and data-image address exposure. Ordinary frame
  identities alone are insufficient evidence against physical aliases.
- Consider forwarding only, store removal, and store plus allocation removal.
  Accept only smaller emitted images, without affected-routine code/home growth
  or additional occupied zero-page bytes. Fall back independently when a stronger
  candidate fails. Use a separate configuration switch and rejection statistics.

## Validation

Positive cases cover populated/empty variants, nested payloads, complete zeroing,
repeated calls, one/eight-byte boundaries, and safe direct callers. Negative cases
cover both overlap directions, exposed source/neighboring storage, absolute and
local aliases, escaping destinations/routine addresses, callbacks/reentry, partial
or out-of-bounds writes, incomplete initialization, effects and larger copies.
Preserve existing copy-forwarding overlap tests.

Run affected MIR6502 unit/integration/emission-sweep and VM tests. Runtime tests
exercise raw/optimized NIR, cartridge/standalone runtime, feature toggles and LF/
CRLF through the actual parser. Final qualification broadens within MIR6502.
If implementation changes shared NIR/semantic/ABI contracts, also run the required
NIR snapshots, NIR sweep and full shared test suite; otherwise scope checks to the
backend per [AGENTS.md](../AGENTS.md).

## TN acceptance and measurements

Baseline at `f0993cec`: maintained TN 9,968 XEX bytes, TNDBG 13,073 bytes,
variant experiment 10,164 bytes. The concrete target is `FindItem`'s two three-byte
constructor captures and their initialization stores. The listing suggests tens
of bytes of opportunity; final emission determines the actual saving. Check
maintained TN/TNDBG behavior and size, emitted/deferred storage, buffer placement
and zero-page usage. Do not special-case TN or change its source for this pass.

Related contract: [MIR6502 copies](MIR6502_PSEUDO_MACHINE_CONTRACT.md#forwarding-captured-copy-values).
Existing input facts: [NIR aggregate boundaries](NIR_TARGET_SHAPE.md#aggregate-call-boundaries).

## Progress

- Slice 1 complete: `MirStoragePurpose` retains all existing scalar purposes and
  adds `AggregateCapture`. No executable NIR or ABI contract changes. The two NIR
  test assertions using the MIR API were updated mechanically. Validation:
  1,107 MIR6502 unit tests passed (two ignored), plus MIR fixture and call-ABI
  integration targets. Printed MIR snapshots are unchanged.
- Slice 2 implementation refinement: tiny-fill selection leaves the empty arm's
  stores in an unconditional predecessor. The proof therefore uses the existing
  forwarding bounds (four blocks / 64 operations) across unique unconditional
  edges, without CFG merging or admitting joins, loops or block arguments.
- Slice 2 complete: independent store-elision flag, closed caller/result-buffer
  proof, incoming-pointer protection using exact bounded callee writes, and
  final physical-alias and emission checks. Allocations remain intact. Four
  focused proof/cost tests and 14 VM tests passed across constructor storage,
  copy forwarding, direct aggregate calls and indirect aggregate calls. New VM
  cases cover raw/optimized NIR, both runtimes and LF/CRLF input.
- Slice 2 measurement: variant TN shrank from 10,164 to 10,134 XEX bytes
  (30 code bytes); its two three-byte captures remained allocated. Maintained
  TN and TNDBG stayed byte-identical to the baseline.
- Slice 3 complete: independent home-elision flag and transaction against the
  store-only output. Both three-byte `FindItem` captures disappear. A regression
  places an absolute alias in only the proposed layout and verifies rollback to
  the smaller store-only output. No shared NIR, semantic or ABI contract changed.

## Final qualification

Backend-scoped checks passed: 1,112 MIR6502 library tests (two ignored), all 96
tests in the 11 `mir6502_*` integration targets, and 169 fixtures through
`actionc-mir6502-sweep --emit-load fixtures/mir6502`.

All 48 tests passed in the VM targets `constructor_storage`, `copy_values`,
`scalar_captures`, `aggregate_calls`, `aggregate_indirect`, `tn_dispatch`,
`aggregate_fresh_initialization`, `variant_case_validation`,
`variant_initialization`, and `variants`. These include both runtime modes,
raw/optimized NIR, LF/CRLF parsing, invalid tags, overlapping copies, complete
payload initialization, callbacks and static-frame reentry. MIR snapshots did
not change. Formatting, patch whitespace and plan links were checked.

| Cartridge MIR6502 program | Before XEX | After XEX | Difference |
| --- | ---: | ---: | ---: |
| Maintained TN | 9,968 | 9,968 | byte-identical |
| Maintained TNDBG | 13,073 | 13,073 | byte-identical |
| Existing menu-variant TN experiment | 10,164 | 10,128 | -36 bytes |

The variant's `FindItem` code shrank from 148 to 118 bytes, and six emitted
capture-storage bytes disappeared. Its deferred storage remains 2,794 bytes;
the final buffer moved from `$5E92` to `$5E6E`. Final emitted size is 10,116
bytes, total reserved size 12,910 bytes. The final cost gates reject additional
zero-page demand. The remaining variant overhead versus maintained TN is 160
bytes. TN source and unrelated workspace changes were preserved.
