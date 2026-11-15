# MIR6502 private-storage boundary: implementation plan

Prepared 2026-11-15. Status: slices 1–2 complete; qualification pending.

Allow existing cleanup to remove private temporaries without requiring complete
read footprints for every call or assembly routine. Reuse storage ownership,
observation checks and the terminal transactions. Commit after each slice.

This is the next priority after slices 1–2 of the
[callee-facts repair plan](MIR6502_CALLEE_FACTS_REPAIR_IMPLEMENTATION_PLAN.md).
Keep those correctness fixes. Leave the unfinished, uncommitted slice 3 parked;
do not include it in this work or its measurements.

## Boundary to implement

- Compiler-created, nonescaping temporaries are private in RAM and ZP. Ordinary
  calls and assembly cannot inspect them unless an operand, address exposure or
  ABI contract makes them accessible. Depending on an undeclared compiler spill
  address is outside the supported contract.
- Unknown memory access still covers accessible program memory: globals,
  address-exposed objects and ABI storage. It does not by itself expose every
  compiler allocation. Keep the existing conservative treatment of source
  parameters and locals; a compiler-assigned home alone does not make them private.
- Typed machine references, explicit numeric aliases, declared physical regions
  and ABI accesses remain observations. Preserve exposures in retained historical
  programs as well as the current program. Source-owned ZP remains source storage.
- Register, flag, stack and scratch effects remain independent requirements.
  Assembly must describe additional effects not covered by decoding or its ABI;
  retain conservative defaults where these facts are missing. Detailed read lists
  are optional precision, not a prerequisite for deleting unexposed temporaries.

This changes observation eligibility, not general aliasing or call ordering.
Unknown writes retain their existing value-invalidation behavior. Existing
fixed-scratch lifetime and clobber checks remain necessary across calls.

Slice 1 validation: all 16 late-cleanup ownership tests passed, including the new
unknown-assembly/exposure groups and existing execution, source-ZP and ABI controls.

## Slice 1 — make explicit exposure independent of footprint completeness

Primary code: [storage_ownership.rs](../src/mir6502/analysis/storage_ownership.rs),
[known_callees.rs](../src/mir6502/analysis/known_callees.rs) and
[late_cleanup/mod.rs](../src/mir6502/late_cleanup/mod.rs).

1. Add the boundary above to the ownership and allocation-pruning sections of
   the [late-cleanup design](MIR6502_LATE_CLEANUP_DESIGN.md). Replace the blanket
   requirement for complete assembly footprints with the private-storage rule.
2. Reuse `StorageOwnership` to identify eligible compiler homes. Preserve source
   backing, address-taking, typed assembly references and physical source overlap
   as exclusions. Do not introduce a second ownership classifier.
3. Preserve explicit machine observations even when another instruction makes
   the routine-wide read or write summary `Unknown`. Reuse the existing machine
   decoder and structured effects to collect those references for observation
   checks; an incomplete inventory must never be advertised as `Exact`.
4. Keep unresolved explicit references conservative for the affected candidates.
   Unknown indirect access alone is not an explicit reference to a private home.
   No new index-range inference or instruction-state analysis is required.

Focused checks: one grouped regression covering an unknown assembly read with
an unexposed temporary, the same case with explicit typed/numeric exposure, and
a known direct access followed by an unknown read. Exercise read and write
observations where allocation safety depends on both. Run the existing ownership
controls for source-owned ZP and typed machine exposure.

Commit the contract clarification, observation changes and focused tests together.

## Slice 2 — remove the blanket assembly veto from private cleanup

Primary code: [late_cleanup/pruning.rs](../src/mir6502/late_cleanup/pruning.rs)
and the shared observation consumers in
[late_cleanup/mod.rs](../src/mir6502/late_cleanup/mod.rs).

1. Replace `unobserved_bytes`' whole-program requirement that every machine
   routine have exact reads and writes. Check the candidate's private ownership
   and explicit observations instead, using slice 1's inventory.
2. Apply the same rule to removed allocations and the existing narrowly permitted
   private-byte movement, in baseline/candidate layouts and retained witnesses.
   Keep unresolved machine-name restrictions where exposure cannot be established.
3. Audit other unknown-read vetoes encountered by these candidates. Change only
   checks that already have private-ownership evidence. In particular, do not
   globally change `may_read_caller_private_byte(Unknown)` to false: its current
   carrier consumer operates on a `Param`, which needs a separate proof.
4. Retain dead-definition/register liveness, observable-load protection,
   register/flag/callee preservation replay, physical layout checks, strict
   whole-image cost acceptance and exact rollback. Keep existing feature gates
   and movement limits; add no optimizer pass.

Focused checks: extend the existing pruning tests to accept an unused private
tail beside an unknown assembly reader, while retaining explicitly exposed,
source-backed and ABI storage. Include a historical exposure case and the
existing replay/rollback controls. Adapt the existing execution control to
exercise the changed assembly boundary and verify its result and observed memory;
reuse its runtime/origin coverage rather than creating another matrix.

Commit after those checks pass. Do not fold in the parked directional-write work.

Slice 2 validation: 28 pruning tests passed across the initial run and one focused
recheck after updating the expected earlier rejection reason. Two MADS-only tests
remain ignored. Execution controls cover every byte input, both runtimes and both
origins; the unknown assembly reader permits one-byte tail removal and two-byte
compaction. No full suite was run.

## Slice 3 — measure and record the result

1. Compare against the committed correctness baseline: **9,862 bytes**, recorded
   in the [callee-facts evidence](benchmarks/6502-callee-facts-repair/README.md).
   Exclude unrelated working-tree changes from both builds.
2. Use the existing TN measurement tooling once after implementation. Report the
   normal optimized image separately from the opt-in spill-pruning experiment;
   compare identical settings and retain their configuration receipts.
3. Record whole-image bytes, removed stores/spill bytes and changed routine
   sizes. Inspect Putchar, FormatDirectoryRow and SetWin, which grew after the
   correctness repairs. Do not assume all 71 bytes will return: source parameter
   homes and remaining clobber/layout restrictions may still block rewrites.
4. Record the focused execution results and any remaining blocker. Zero TN savings
   is a valid measured result; do not expand the implementation to chase a number.

Commit the qualification evidence. Reuse the existing shared build target; run
no full test suite or other backend suite for this MIR6502-only change. If fixture
instrumentation becomes newline-sensitive, verify its actual LF and CRLF paths.

## Deferred

Complete TN assembly annotations, new annotation syntax/plumbing, automatic
indexed-read bounds, recursive call analysis and broader parameter-home proofs
are separate work. This plan changes no SemIR/NIR contract and adds no general
memory-analysis framework.
