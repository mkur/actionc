# MIR6502 homes across multiple leaf calls: suspended implementation draft

Prepared 2026-11-15 against `8626beff`. Status: suspended; implementation has not
started. The [whole-image cost implementation plan](MIR6502_WHOLE_IMAGE_COST_IMPLEMENTATION_PLAN.md)
is now complete; [qualification](benchmarks/6502-whole-image-cost/README.md) leaves
normal TN unchanged at 9,791 bytes. Revisit batching before resuming this draft.

The earlier three-slice schedule assumed a separate multiple-call trial after
the existing single-call trial to protect its gains from the per-routine growth
veto. That sequencing requirement is withdrawn. This draft is not ready for
implementation until a fresh batching decision; cost-policy qualification is complete.
The agreed proof scope remains in the
[design extension](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-multiple-calls-to-leaf-routines).

## Retained scope

- Admit otherwise eligible callers with multiple direct sites only when every
  callee passes the existing ordinary Action leaf-call contract.
- Reserve the union of possible callee writes for both parameters and initialized
  locals. Validate every site's argument supply, including repeated targets.
- Reuse existing expanded per-call lifetime verification, invocation-home
  certificates, final-placement replay and exact transaction rollback.
- Retain the four-byte incoming-parameter limit, source-ZP exclusions, complete
  byte/word objects and explicit result delivery.
- Keep nested/recursive callees, indirect/unknown calls, unsupported helpers,
  machine bodies, register residence, save/restore and subset search outside scope.

## Work to reschedule after cost qualification

The selector in [cross_call.rs](../src/mir6502/parameter_homes/cross_call.rs)
currently accepts exactly one call and uses that call's writes. Generalization
must account for all sites; removing the count guard alone is insufficient.
Reuse [ResidenceCalls](../src/mir6502/parameter_homes/calls.rs) and the existing
placement helpers, without creating another allocator or liveness engine.

Decide whether new candidates join the current batch and identify its exact
selected fallback. Record the continuation bound and diagnostics for that choice.
Whole-image acceptance permits local growth but does not promise that a combined
candidate beats every unmeasured subset. An extra selection stage is not implied
by the cost-policy change.

Refresh measurements against the post-cost-policy compiler. FindItem's recorded
pre-change footprint is 74 code bytes plus three ordinary-home bytes; its calls
to Key and Next motivate the scope, without supplying executable special cases
or a promised saving. Reuse recorded TN source/configuration hashes until a
measurement is needed.

## Minimal testing to retain

Use one grouped proof test for two distinct callees, a repeated target, a second
callee's one-lane clobber, an invalid second call and exact rollback. Reuse existing
alias, exposure, ABI and local-initialization coverage.

Extend the existing `mir6502_cross_call_homes` integration and `cross_call_homes`
VM targets with a small independent two-call control. Replace the old `Twice`
case's blanket unsupported expectation while retaining nested-callee fallback.
Check final savings, arguments, results, stack balance, repeated/conditional
execution and real LF/CRLF loading. Use cart as the main case and one standalone
smoke case; avoid a configuration matrix.

Run one final TN cart comparison when this extension is implemented. Keep full
backend, raw-NIR, shifted-origin, MADS and TN runtime matrices deferred under the
user's minimal-testing direction. Reuse the shared build target. Produce a new
concrete slice/commit schedule before resuming this draft.
