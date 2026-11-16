# MIR6502 guaranteed machine definitions implementation plan

Status: all three implementation slices complete and qualified, 2026-11-17.

Implements issue **5** from the [fact repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md)
and the [guaranteed register/flag design](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#guaranteed-register-and-flag-definitions).
Commit each slice independently. Keep frontend/NIR contracts, emission strategy,
callee eligibility and assembly coverage unchanged.

## Consumer map

| Consumer | Required meaning |
| --- | --- |
| Machine liveness; native entry transfer; CFG redefinition scans | Only guaranteed writes kill an incoming value; reads happen first |
| Argument forwarding's restored N/Z check | Guaranteed N/Z definitions |
| Carry/overflow overwrite compatibility queries | Projections of the same guaranteed flag writes |
| Register/home value availability and parameter availability | All possible changes invalidate knowledge |
| Address/loop motion, register reuse, forwarding and rewrite exit-state declarations | All possibly changed flags must be covered by the safety proof |
| Rewrite effect comparison | Preserve possible effects; selected-result checks still require actual definitions |
| Legacy all-flags entry query | Projection of the shared precise entry analysis with the same eligibility |
| Bounded assembly decoder | Actual decoded instruction facts; annotations remain possible effects |

## Slice 1 — Separate possible-effect consumers

- Add central possible-register/flag queries over the existing effect fields.
- Migrate invalidation and rewrite safety consumers that currently use a definite
  field as a possible-effect mask. Preserve conservative unknown effects.
- Add focused controls showing that finite clobbers remain visible to value
  invalidation and rewrite checks. Record the definition defects for slice 2.
- Do not narrow the existing definition producers in this slice. Run affected
  effect, value-availability and rewrite unit tests, then commit.

Slice 1 validation: 69 focused effect, machine-value, loop-motion, address-rewrite
and register-reuse unit tests passed. Possible clobbers still invalidate cached
register/flag values without claiming definitions.

## Slice 2 — Share guaranteed definitions

- Audit selected operation recipes against emission. Set the existing
  `register_writes` and `flag_writes` only for guaranteed boundary definitions;
  retain merely possible changes in clobber fields. Do not add a parallel
  post-classification whitelist.
- Cover unconditional byte loads/transfers/ALU, direct immediate comparisons,
  supported memory comparisons and updates. Keep folded, identity, zero-work,
  conditional and unsupported forms conservative. Preserve reads of incoming C.
- Switch ordinary liveness, native entry liveness and loop redefinition scans
  together. Remove the separate entry definition whitelist.
- Derive carry/overflow overwrite queries from guaranteed sets; remove duplicate
  fields. Make the legacy all-flags answer a projection of the precise entry
  result, retaining existing call and eligibility policies.
- Test individual flag masks, byte/expanded-word comparisons, zero CMP reuse,
  self-moves, zero shifts, conditional paths, finite may-clobbers, read-before-write,
  call policies and freshness. Include changed rewrite/loop consumers.
- Commit only after the focused consumers pass together. Broaden within MIR6502
  when a failure exposes another affected consumer.

Slice 2 validation: 373 focused MIR6502 unit tests passed (one diagnostic test
ignored). Coverage includes effect producers, all entry queries, machine values,
call facts, forwarding, rewrite transactions, CFG and local cleanup. Older local
flag-redefinition helpers now replay the shared transfer; rewrite effect
comparison accepts definition refinement while retaining possible effects and
explicit selected-result requirements.

## Slice 3 — Qualify the repaired contract

- Run a focused emitted/VM control varying initial flags and data around compare
  boundaries, including an emitted CMP and reused zero flags with later carry
  use. Cover both runtimes and LF/CRLF through the real source-loading path.
- Run relevant flag-forwarding/rewrite integration or VM controls for the changed
  consumers. Avoid full repository suites by default.
- Re-run the retained independent fact controls and all selected TN routines.
  Report the actual `ArenaCursor`/caller flags, result facts retained from issue 4,
  remaining placement blockers and image size/hash. Do not require size savings.
- Save qualification evidence, update the contract and priorities, check links
  and diffs, and commit the qualification slice.

Qualification: three callee-fact VM controls, the C/V observer VM matrix
(both runtimes, LF/CRLF), and seven loop integration tests passed. The existing
A→X and precise-entry size-delta assertions fail identically on pre-repair
`2ce87047` (2 versus 4 bytes; 0 versus 2 bytes). Their expectations are unchanged.

The retained audit passed for nine independent controls and all 95 TN routines.
`ArenaCursor` and `Alloc` have no acyclic incoming flags; issue-4 result facts
remain intact. TN is 9,811 bytes, byte-identical to issue 4. See the
[qualification receipt](benchmarks/6502-call-facts-audit/machine-definitions-qualification.json).
No full suite was run.

## Follow-up — Design issue 6

After issue 5 is qualified, inspect the actual fixed-address resolver, known
memory-write preservation queries, source backing/aliases, and placement-aware
consumers. Extend the shared design to specify authoritative address evidence at
each phase, range/alias handling and conservative Unknown behavior. Include
source-global RAM versus ZP controls and freshness rules. Do not implement issue 6.

Follow-up complete: the [issue-6 design](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#physical-overlap-from-backing-and-placement)
is saved. Issue 6 is not implemented by this plan.
