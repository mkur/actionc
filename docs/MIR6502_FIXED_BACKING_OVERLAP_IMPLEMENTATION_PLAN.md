# MIR6502 fixed backing overlap implementation plan

Status: all three fixed-global implementation slices complete and qualified, 2026-11-17.

Implements issue **6** from the [fact repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md)
and the [physical overlap design](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#physical-overlap-from-backing-and-placement).
Keep logical footprints, input classification, eligibility, emission and final
cost/rollback contracts intact. Commit each slice separately.

## Slice 1 — Shared fixed backing evidence

- Build a short-lived address projection from the current program using
  `fixed_storage_address`. Keep it separate from logical footprints.
- Compare bytes by checked fixed addresses. Missing backing, invalid aliases,
  overflow and ownerless private references remain unresolved by this query.
- Test RAM versus ZP, overlapping names, nested aliases, offsets, unresolved
  backing, cycles, overflow and rebuilding evidence after rebinding.
- Run focused query unit tests and commit.

Slice 1 validation: three new overlap controls and the existing fixed-storage
relocation control passed (four filtered unit tests).

## Slice 2 — Migrate preservation consumers

- Bind the projection to the existing known-callee analysis; use it explicitly
  for fixed byte/pair preservation, cached-value invalidation and caller-private
  read/write checks. Reject a pair beginning at `$FF` instead of saturating.
- Migrate residence discovery, spill placement, lifetime verification, carrier
  selection and contract replay together. Compare old and proposed summaries
  using their respective backing evidence.
- A caller memory map may resolve the queried caller byte, never ownerless
  callee-private identities. Preserve existing normalized-private-storage
  contracts; do not broaden this fixed-global slice into private alias repair.
- Add consumer controls for BYTE/CARD writes, named backing collisions,
  Unknown footprints, candidate freshness and cached-value retention/deletion.
- Run focused MIR6502 consumer tests; investigate regressions in that scope and
  commit the coherent migration.

Slice 2 validation: 241 focused MIR6502 unit tests passed. Two existing failures
(pointer-retention indexed-read acceptance and the expanded-entry carry
expectation) reproduce unchanged on a clean `239942f5` checkout; their assertions
remain unchanged. The audit passed for 95 TN routines and 11 independent controls.
TN remains byte-identical at 9,811 bytes. The final unit run reused the audit's
compiled test binary; four diagnostic tests and the separately executed audit
were ignored, and the two confirmed baseline failures were excluded.

## Slice 3 — Qualify and record results

- Exercise caller state across fixed RAM and ZP global writes in the VM, both
  runtimes and LF/CRLF through the actual source loader. Check source reservations.
- Re-run the independent fact controls and TN audit. Record physical facts,
  placement changes, remaining blockers and actual image size/hash; savings are
  not an acceptance condition.
- Save a qualification receipt, update the design and priority status, verify
  links/diffs and commit. Run no full suites unless a failure requires them.

Qualification: all three selected callee-fact VM controls passed, including the
new fixed-global alias-write matrix, both runtimes, default/optimized compilation,
LF/CRLF loading, live caller values and source reservations. Six TN routines now
correctly preserve `$E4`: `CalcAdr`, `Position`, `Relpos`, `_ImageAdvance`, `Instr`
and `FindNext`. Selected MIR and the 9,811-byte image are unchanged, including
16 ZP source homes across eight routines. `Copy` and `PopUp` retain their existing
unsupported-native-closure blockers. The independent RAM-write control remains
33 bytes; no image-size saving is claimed.

See the [qualification receipt](benchmarks/6502-call-facts-audit/fixed-backing-qualification.json).
The temporary audit include and isolated baseline checkout were removed. No full
suite was run, and the two confirmed baseline assertions were not changed.

No frontend/NIR contract changes or general layout/alias framework were introduced.
Private-layout precision without retained ownership remains follow-up work.

## Scope refinement during slice 2

Applying the generic unresolved-address fallback to all existing private-home
queries disabled acyclic placement and pointer retention controls. The fixed-global
repair therefore retains the existing private-storage exclusions at the consumer
boundary. New fixed-address decisions use the shared query; globals with unknown
backing never inherit those exclusions. Ownerless private references are not
resolved through caller maps. Replacing the legacy private exclusions requires
owner-aware footprints and applicable placement evidence as a separate slice.
