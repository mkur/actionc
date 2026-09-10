# Selection return codegen

Status: complete and validated on 2026-09-10. Baseline: `e6d42cc`.

## Scope

Forward unconditional edges through empty scalar value-return blocks in
verified NIR. Substitute a returned block parameter with the corresponding
incoming argument. Keep computations, calls, memory operations and faults at
their existing execution points. Leave conditional edges, nonempty return
blocks and aggregate returns unchanged. Remove newly unreachable blocks and
rebuild temporary definitions through the existing optimizer fixed point.

The existing selection audit shows scalar CASE paying three extra cycles for
the shared return path. IF's shared-return shape also prevents existing MIR
leaf inlining. This slice measures whether normalizing returns exposes useful
codegen without introducing source-specific optimizer rules. Target inlining,
register placement and byte/cycle decisions remain MIR6502 responsibilities.

## Acceptance

- Cover nested joins, multiple typed parameters, conditional predecessors,
  nonempty/effectful return blocks, fault paths, and optimizer idempotence.
- Preserve exact scalar widths and verifier checks; retain existing expression
  consumer/effect VM oracles, including wide values and returning Error handlers.
- Compare all four statement/expression audit pairs against the recorded
  baseline, report both code size and cycles, and retain known-SOME folding.
- Review any optimized snapshot changes as intentional CFG simplification;
  raw NIR and source semantics are unchanged.
- Run focused regressions, NIR snapshots and sweep, the full compiler suite,
  all-targets check, and relevant VM suites before committing this slice.

## Result and validation

The general NIR return-forwarding cleanup is implemented. In the four audited
pairs, MIR expression size and cycles now match the corresponding statement
form. Scalar CASE saves three cycles at unchanged size; dynamic variant CASE
saves one to three cycles at unchanged size. IF saves 12 cycles through existing
leaf inlining and grows by 11 bytes. Known-SOME and classic measurements are
unchanged. See the [full measurements and rationale](IF_CASE_EXPRESSIONS_CODEGEN_AUDIT.md#return-forwarding-follow-up)
and [follow-up CSV](SELECTION_RETURN_CODEGEN_AUDIT.csv).

All 3,073 compiler tests pass, with 22 pre-existing ignored. All 260 tests in
the full locked VM suite pass, with none ignored. The seven focused NIR tests
cover exact-width returns across four target layouts and optimizer idempotence,
including malformed-edge rejection and preserved storage/fault boundaries.
The 384-execution codegen audit, 20 IF/CASE VM tests and all 29 Oscar64 tests
pass. NIR snapshots, the 49-fixture sweep, the broader 342-source corpus with
eight expected exclusions, the sample build matrix and `cargo check --all-targets`
pass. Three optimized snapshots intentionally replace obsolete return joins;
raw snapshots are unchanged.
