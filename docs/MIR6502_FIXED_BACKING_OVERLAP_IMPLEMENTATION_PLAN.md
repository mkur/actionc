# MIR6502 fixed backing overlap implementation plan

Status: ready to implement, 2026-11-17.

Implements issue **6** from the [fact repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md)
and the [physical overlap design](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#physical-overlap-from-backing-and-placement).
Keep logical footprints, input classification, eligibility, emission and final
cost/rollback contracts intact. Commit each slice separately.

## Slice 1 — Shared fixed backing evidence

- Build a short-lived address projection from the current program using
  `fixed_storage_address`. Keep it separate from logical footprints.
- Compare bytes by checked fixed addresses. Missing backing, invalid aliases,
  overflow and ownerless private references remain possible overlaps.
- Test RAM versus ZP, overlapping names, nested aliases, offsets, unresolved
  backing, cycles, overflow and rebuilding evidence after rebinding.
- Run focused query unit tests and commit.

## Slice 2 — Migrate preservation consumers

- Bind the projection to the existing known-callee analysis; use it explicitly
  for fixed byte/pair preservation, cached-value invalidation and caller-private
  read/write checks. Reject a pair beginning at `$FF` instead of saturating.
- Migrate residence discovery, spill placement, lifetime verification, carrier
  selection and contract replay together. Compare old and proposed summaries
  using their respective backing evidence.
- A caller memory map may resolve the queried caller byte, never ownerless
  callee-private identities. Keep unresolved private placement conservative.
- Add consumer controls for BYTE/CARD writes, named backing collisions,
  Unknown footprints, candidate freshness and cached-value retention/deletion.
- Run focused MIR6502 consumer tests; investigate regressions in that scope and
  commit the coherent migration.

## Slice 3 — Qualify and record results

- Exercise caller state across fixed RAM and ZP global writes in the VM, both
  runtimes and LF/CRLF through the actual source loader. Check source reservations.
- Re-run the independent fact controls and TN audit. Record physical facts,
  placement changes, remaining blockers and actual image size/hash; savings are
  not an acceptance condition.
- Save a qualification receipt, update the design and priority status, verify
  links/diffs and commit. Run no full suites unless a failure requires them.

No frontend/NIR contract changes or general layout/alias framework are planned.
Private-layout precision without retained ownership remains follow-up work.
