# MIR6502 directional read facts implementation plan

Status: ready to implement, 2026-11-17.

Repairs issue **7** from the [fact repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md),
using the existing [directional-effects contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md#callee-exit-value-and-flag-provenance).
Commit the plan and each implementation slice separately. Keep frontend/NIR
contracts, emission, call eligibility and private-storage alias rules unchanged.

## Slice 1 — Restore the read projection

- Audit producers of aggregate uncertainty and their directional read/write
  fields. Read summaries must use read uncertainty; retain aggregate barriers
  where consumers intentionally constrain ordering or both effect directions.
- Remove write-only uncertainty from the native read-summary guard. Preserve
  genuine indirect reads, opaque effects and incomplete physical read evidence.
- Before publishing finite reads, retain both pointer-carrier bytes, direct
  operand extents and current home bindings. Logical temps, missing virtual
  allocations, invalid pair extents and overflowing ranges must not produce an
  incomplete Exact footprint. Reuse existing effect and allocation information.
- Cover plain indirect stores, direct-word copies and subtraction stores;
  contrast loads, indirect copies and read-modify-write operations. Check
  calls/composition, structured effects, barriers and unknown declarations.
- Compare representative finite footprints with emitted memory-read instructions,
  accounting for the existing balanced-stack contract. Check that writes stay
  Unknown, cached values are invalidated and observable reads remain protected.
- Run focused effect, callee-summary and affected consumer tests; document any
  confirmed baseline failures. Update the existing contract clarification and
  commit. No new analysis framework or design note is required.

## Slice 2 — Qualify and record

- Run a small VM matrix for store-only and read-modify-write callees with live
  caller state, boundary data and pointer aliasing. Cover default/optimized code,
  both runtimes, and LF/CRLF through the real source loader.
- Extend the retained audit with native store-only/composed and read-modify-write
  controls. Assert finite pointer reads separately from unknown pointee writes.
- Re-run the independent controls and all selected TN routines. Report changed
  footprints, actual image size/hash, placement and remaining blockers. No size
  saving is an acceptance condition.
- Save a qualification receipt, update issue status and plan, check links/diffs,
  remove the temporary audit hook, and commit. Do not run full suites by default.

## Acceptance

A store through an unknown pointer may have a complete finite read footprint
without gaining a finite write footprint. Reading the pointee still produces
Unknown reads. Neither refinement establishes definitions, permits ignoring
pointer inputs, nor grants permission to delete observable accesses.
