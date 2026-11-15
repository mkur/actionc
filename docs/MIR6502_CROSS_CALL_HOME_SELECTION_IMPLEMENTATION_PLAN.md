# MIR6502 ZP homes across ordinary calls: implementation plan

Prepared 2026-11-15. Status: slices 0–5 complete; slice 6 pending.
Starting repository revision: `5de3dda4`; compiler behavior: `a77210d8`.

Implements the [ordinary-call and local-home design extension](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-zp-homes-across-ordinary-calls).
The existing leaf, terminal-handoff, callee-entry-capture and late-register-reuse
optimizations supply the baseline. Execute the slices in order and commit each
after its checks pass. Keep unrelated working-tree changes outside those commits.

## Objective and fixed scope

Let an ordinary Action routine keep eligible parameters, then source locals,
in ZP while executing a known call inside a closed CFG. Use the existing binding,
reservation, lifetime and final-cost machinery. The motivating cases are
[Ord and Items](../samples/tn/modern/LIB.ACT): eliminate repeated pointer setup
and shorten memory accesses without changing their source or the public ABI.

The first production scope has one direct ordinary Action call site per caller;
it may execute repeatedly or conditionally in a loop. Start with ordinary leaf
callees whose expanded native operations have complete entry and write contracts.
Retain the four-byte incoming-parameter limit and the existing supported native
operation set outside the call. Parameters may be mutable. Returned BYTE/WORD
values, including pointer values, use the existing explicit result assignment.
Eligible objects have unexposed scalar BYTE/WORD storage and coherent pointer
pairs. Locals additionally require proof of assignment before every read on each
invocation.

Multiple call sites, nested/recursive callees, indirect calls, SArgs, machine
bodies, unsupported helpers, opaque or indirect-write effects, exposed/fixed
storage and persistent local values remain outside this initial scope. These
restrictions apply to the new extension; preserve existing terminal handoff's
independent admission rules. Keep `$AC/$AD` as copy-and-rebase scratch and retain
the current candidate slot ranges and source-ZP exclusions.

Use a preserved working pair and an explicit result copy initially, even when
the old pointer is dead across the call. Return-home coalescing into `$A0/$A1`
is a separate follow-up, as are register residence across calls, split lifetimes,
save/restore insertion and wider call-graph allocation. No SemIR/NIR contract
change, new state tracker or program-specific optimization is planned.

## Code boundaries and current gaps

| Responsibility | Existing code to extend |
| --- | --- |
| Selection, reservations and logical lifetime projection | [parameter_homes.rs](../src/mir6502/parameter_homes.rs), [zero_page.rs](../src/mir6502/parameter_homes/zero_page.rs), [lifetimes.rs](../src/mir6502/parameter_homes/lifetimes.rs) |
| Byte preservation and entry contracts | [known_callees.rs](../src/mir6502/analysis/known_callees.rs), [entry_inputs.rs](../src/mir6502/analysis/known_callees/entry_inputs.rs) |
| CFG and storage facts | [home_liveness.rs](../src/mir6502/analysis/home_liveness.rs), [home_definitions.rs](../src/mir6502/analysis/home_definitions.rs), [effects.rs](../src/mir6502/analysis/effects.rs) |
| Typed bindings and phase verification | [ir.rs](../src/mir6502/ir.rs), [verify.rs](../src/mir6502/verify.rs) |
| Capture, calls and pointer consumers | [abi/param_capture.rs](../src/mir6502/abi/param_capture.rs), [materialize/abi.rs](../src/mir6502/materialize/abi.rs), [materialize/pointers.rs](../src/mir6502/materialize/pointers.rs), [call_plan.rs](../src/mir6502/call_plan.rs) |
| Selected input, bounded continuation and configuration | [preparation.rs](../src/mir6502/preparation.rs), [parameter_homes.rs](../src/mir6502/parameter_homes.rs), [passes.rs](../src/mir6502/passes.rs) |
| Planning/final storage and cost | [materialize/layout.rs](../src/mir6502/materialize/layout.rs), [layout/ordinary.rs](../src/mir6502/materialize/layout/ordinary.rs), [emit.rs](../src/mir6502/emit.rs) |
| Downstream facts and preservation | [rewrite/driver.rs](../src/mir6502/rewrite/driver.rs), [late_cleanup/preservation.rs](../src/mir6502/late_cleanup/preservation.rs), [analysis/record_bases.rs](../src/mir6502/analysis/record_bases.rs) |
| Diagnostics and measurements | [trace/homes.rs](../src/mir6502/trace/homes.rs), [printer.rs](../src/mir6502/printer.rs), [parameter_homes.py](../tools/compare6502/parameter_homes.py), [measurement probe](../tools/compare6502/parameter_homes.rs) |

Three integration gaps need explicit work:

- Lifetime verification currently takes only a routine and rejects every
  nonterminal call. It needs program-level callee facts from the representation
  being verified, while preserving logical definitions separately from clobbers.
- Known-callee write normalization currently resolves allocated single-byte
  virtual ZP slots. A residence proof must also account for bound parameter
  bytes, and later local bytes, including accesses still written with logical
  identities. An unresolved logical home cannot silently imply preservation.
- Planning and emission already honor parameter bindings, but local allocation
  and `routine_costs` still count ordinary locals unconditionally. Local support
  must reach all those consumers before selecting any local for ZP.

## Slice 0 — freeze the baseline and add independent controls

Add `fixtures/runtime/cross_call_homes.act`, a focused root integration target
`mir6502_cross_call_homes`, and VM target `cross_call_homes`. Use ordinary Action
caller/callee examples independent of TN, with distinct low/high bytes and an
observable result. Preserve a real call by disabling inlining in isolated cases;
also retain cases using the normal optimized configuration.

Cover a pointer replaced by a returned pointer, a second value live across the
call, and an initialized counter. Include zero-trip and repeated loops, early
exit, conditional call paths and a local read before initialization. Begin with
passing characterization tests for current inline homes and current behavior;
update expectations only when the relevant slice implements the optimization.
Use seeded or repeated-invocation controls for entry values; do not base a VM
oracle on unspecified initial contents of an uninitialized local.

Extend the existing parameter-home measurement harness with a cross-call suite
and, when introduced, an explicit extension toggle. Do not turn off existing
leaf/handoff/reuse optimizations to construct the feature-off control. Record
compiler revision/diff/library hash, normalized source hashes, configuration,
images, maps, relocations, routine code/ordinary/ZP sizes and compiler work.
Store compact receipts under `docs/benchmarks/6502-cross-call-homes/` and large
artifacts under ignored `build/`; reuse the existing shared Cargo target directory.

Recheck the documented starting values rather than treating them as fresh results:

| Subject | Expected starting measurement |
| --- | ---: |
| TN cart / standalone, packaged bytes | 9,847 / 9,954 |
| Ord code + ordinary parameter/local bytes | 80 + 5 |
| Items code + ordinary parameter/local bytes | 56 + 3 |
| Next / Key code | 30 / 34 |
| Strcpy / Fnamecmp code + ordinary parameter bytes | 20 + 0 / 105 + 0 |

The TN image and late-reuse baseline is recorded in the
[qualification receipts](benchmarks/6502-late-register-reuse/README.md).
Measure both runtimes at `$3000` and `$41C7`; identify any divergence and its
cause before using a new baseline. Preserve the source unchanged for comparison.

**Checks and exit:** new baseline integration/VM controls pass against independent
argument, result, memory-access and stack oracles. LF and CRLF exercise the actual
fixture-loading path. Commit the controls and baseline receipts; compiler behavior
is unchanged.

## Slice 1 — make residence verification aware of callee contracts

Add a byte/pair preservation query using the shared known-callee write coverage.
Normalize bound parameter lanes, fixed ZP, absolute aliases and allocated virtual
slots through the routine's authoritative storage mapping. Missing allocations,
unresolved backing and unknown writes remain conservative. Include capture,
callee homes, scratch and result writes; do not reuse the private-inline-RAM
preservation shortcut for shared ZP. Keep transitive/recursive unknown behavior
even though the new selector initially admits leaf callees only.

Pass current program/callee facts into lifetime verification. Build summaries
once per relevant program verification boundary, rather than once per caller
or candidate byte, after structural binding/CFG checks succeed. Keep explicit
phase provenance: pre-materialization verifies
binding shape, ownership and planned entry capture; PostHome,
PostMaterialization and PreEmission check actual expanded effects. A pre-capture
discovery summary cannot certify the expanded candidate.

Extend role-specific transfers in the existing CFG liveness analysis:

- Only writes to the logical parameter define its next value. Physical scratch,
  result and invocation writes remain clobbers.
- Read outgoing arguments before invocation effects. Verify setup and result
  copies in their actual expanded order; retaining a ZP value does not preserve
  any register copy or N/Z fact through a call.
- Require every byte live across an ordinary call to be preserved. Keep terminal
  consumption on its existing path, where values die at the call.
- Check all loop/backedge and join paths, including partial and conditional
  definitions. Preserve entry requirements for noncanonical fixed/register/flag
  inputs using the existing capture-aware analysis.

Use manually bound MIR controls to test the new proof before automatic placement:
a preserving callee, one-lane clobber, clobbered setup, overwritten result source,
and a mutation followed by a backedge. Change a callee binding/allocation after
forming a fact and demonstrate that re-verification rejects stale assumptions.
Do not attach semantic validity to trace IDs or carry operation indexes across
expansion without an existing validated mapping.

**Checks and exit:** focused known-callee, lifetime, verifier and handoff tests
pass, including unknown-call rejection. Existing source controls retain complete
output equality. Commit the proof support with automatic new placement disabled.

## Slice 2 — select parameter homes in one gated batch

Introduce `enable_cross_call_home_selection`, initially false in conservative
and optimized configurations, subordinate to parameter placement and peepholes.
Add one extension stage after the currently selected leaf/handoff result. Use
its `SelectedPlacement` input/executable and existing `try_proposal` continuation;
give the stage its own trace event and counter instead of reporting it as handoff.

Admit the fixed scope above. Use selected materialized callee summaries only to
filter candidates. Retain existing accepted bindings; for the new routine,
choose whole-parameter ZP homes in deterministic declaration order through the
shared reservation code. Exclude all invocation writes from candidate bytes,
including for a parameter immediately redefined after the call. Reserve coherent
pairs against caller scratch, outgoing setup, source-owned bytes, other objects
and the uncaptured fourth incoming byte. Unsupported objects stay inline.

Use the existing ABI capture and resident-pointer consumers. Preserve explicit
result assignment: `menu` receives the new result after `Next`; `item` keeps its
old value. If an existing consumer cannot honor a binding, reject it through
the ordinary proof/fallback path rather than adding a special lowering for TN.

Resume core materialization at most once for the complete batch. Recompute
callee contracts from the expanded candidate, validate resident callers, and
run normal downstream preservation replay and final emission. A discovery
contract invalidated by changed capture, allocation or cleanup must reject the
trial. Keep frames/bindings fixed inside terminal cleanup.

Retain `Executable::improves`: strictly smaller packaged image and no routine
growth in code plus ordinary homes. Equality, unknown cost, invalid proof or
growth restores the complete current optimized artifact, including maps, facts,
reservations and trace selection. No per-object retries, subset search or extra
upstream optimization search. An empty candidate set adds no continuation.

**Checks and exit:** independent source controls select preserved byte/word/pointer
homes and save final bytes. Integration/VM checks cover result assignment, entry
capture, argument setup, loop behavior, exact rollback, trace agreement and the
one-additional-materialization bound. Commit the gated parameter implementation.

## Slice 3 — qualify the parameter scope before adding locals

Measure TN with the new gate off/on and inspect `Ord`/`Items` listings and maps.
Report parameter homes, omitted inline bytes, pointer setup, remaining result
copies and final image savings. Locals still use their current homes in this
slice. A handwritten size estimate or selected binding alone is not evidence
of an emitted saving.

Broaden independent controls to cover conditional calls/definitions, zero and
multiple iterations, exits before and after the call, partial word clobbers,
source ZP conflicts, exposure, exhaustion and absolute/fixed-ZP aliasing. Include
unsupported multiple/nested/recursive calls and helper/machine boundaries as
fallback controls. Exercise optimized callers that relied on a callee's old
contract, including a candidate whose changed clobbers force rollback.

Verify public reads/writes and their order, canonical arguments before capture,
result ABI, stack balance, page crossings and applicable pointer wraparound.
Use raw and optimized NIR controls to check safe fallback where lowering differs.
Retain existing leaf, handoff, pointer and late-reuse controls.

**Checks and exit:** focused affected integration/VM and assembly checks pass,
both runtimes/origins have measured receipts, and parameter behavior has an
independent oracle. Explain rejection or unchanged output for any TN target;
do not widen the scope just to obtain a headline saving. Commit qualification
and any focused fixes. Keep the production default off until final qualification.

## Slice 4 — share storage binding queries with locals

Add typed local bindings keyed by `LocalId` and common object/lane resolution
for parameters and locals. Retain parameter ABI roles and existing public MIR
identities; local homes initially need only inline and complete ZP storage.
Do not model a source local as a spill or fabricate parameter capture for it.
Keep all automatically selected locals inline in this slice.

Make the common binding authoritative for:

- Storage reservation and overlap verification, logical effects and lifetime
  projection, entry/exit callee facts, and physical alias queries.
- Planning allocation, final emission, maps/relocations, pointer-pair queries,
  ordinary-memory bounds and routine cost. A bound ZP local allocates no inline
  bytes and is not counted as ordinary data.
- Post-home value/rewrite consumers and preservation replay. No local access
  may bypass the binding or assume that `MirMem::Local` always means inline RAM.
- Printed MIR and home traces, retaining readable source names as metadata.

Extend the existing exposure proof to typed local relocations, address uses and
aliases. Reject missing/duplicate bindings, invalid widths, overlapping owners,
incoherent pairs and conflicting legacy metadata. Unsupported fixed/aliased/
initializer-backed locals retain their existing representation.

Verification of an explicit non-inline local binding must already enforce the
per-invocation definition rule: every logical byte is assigned before any read
from entry. Use the existing CFG transfer machinery; physical scratch or callee
writes cannot discharge the obligation. Slice 5 reuses this proof for discovery.

**Checks and exit:** default output, maps and costs remain identical. Manually
bound local controls establish correct ZP operands, no shadow inline allocation,
physical effects and verifier rejection of invalid bindings. Run affected
layout/emission, callee-summary, pointer, rewrite and preservation tests. Commit
the shared storage support before automatic local selection.

## Slice 5 — admit definitely initialized source locals

Add local candidates to the same cross-call batch, after parameter candidates,
using stable declaration order and the same reservations. A local may remain
inline when no safe home exists. Do not launch a separate local trial or change
the result-home policy. This slice covers eligible locals in the admitted
ordinary-call routines; general local placement elsewhere remains a follow-up.

Prove that every relocated local byte is assigned before any read on every path
from routine entry. Reuse the CFG dataflow and logical transfer machinery: an
entry-value dependence indicates failure. Only actual logical definitions
discharge it; physical writes by calls or scratch do not initialize a local.
Account for read/modify/write operations, partial stores, conditional definitions
and a loop whose body does not execute. `c = 0` before the loop should qualify;
a counter initialized only on a previous invocation must not.

Preserve ordinary source initialization and memory ordering. Exclude exposed,
fixed-address, aliased, initializer-backed and persistent objects. Apply the same
call-preservation proof to locals as to parameters, including physical writes to
newly selected local homes when summarizing the routine for its own callers.

**Checks and exit:** independent byte/word/pointer locals qualify or fall back
for the right reason; entry-read, partial-definition and persistent-value controls
pass over repeated invocations. Measure `c` in `Ord` and `Items`, retained pointer
setup/result copies, inline-data removal and full-image savings. Retain the
parameter-stage receipts to distinguish the incremental local effect, recording
any other compiler changes. Trace counts still allow only one added continuation
for the combined batch. Commit local selection and focused qualification.

## Slice 6 — complete backend qualification and choose the default

Run the full affected MIR6502 unit, integration and VM/runtime qualification,
including ABI and MADS checks. Include cart/standalone, shifted origins, actual
LF/CRLF fixture/loading paths, raw/optimized NIR controls and traced/untraced
output equality. If fixture helpers use newline-sensitive operations, normalize
host text before them and verify both line-ending paths; preserve binary and
ATASCII bytes exactly.

Measure TN and independent controls with only the new gate toggled. Publish
accepted/rejected candidates, final code/ordinary/ZP/image deltas, commands,
hashes, work counts and existing skips. Record compile time and memory as
observations, without claiming a speed improvement from incidental timings.
Use the same shared build directory; avoid duplicate large build trees.

Enable the gate in `Mir6502Config::optimized()` only after correctness and actual
size benefit are established; conservative mode stays off. Update older isolated
optimization tests to explicitly select their baseline where necessary, while
retaining combined-feature tests. A changed marginal saving is not a reason to
weaken semantic or whole-image assertions. If the scope remains unproductive,
keep it gated and document the blocking shape rather than expanding this plan.

**Checks and exit:** qualification receipts describe the actual final default,
fallback remains exact, existing gains are preserved, and all admitted bindings
survive final layout and preservation checks. Update the design note's status
and this plan's completion record. Commit qualification and the justified
default policy.

## Check scope and completion criteria

During development, use filtered MIR6502 library tests and the new targets plus
their affected consumers. Once the new targets exist, representative commands are:

```sh
cargo test --lib mir6502::parameter_homes
cargo test --lib mir6502::analysis::known_callees
cargo test --test mir6502_cross_call_homes --test mir6502_parameter_homes --test mir6502_parameter_argument_handoff --test mir6502_call_abi
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes --test parameter_homes --test parameter_argument_handoff --test word_result_placement
```

Final unit qualification uses `cargo test --lib mir6502::`. Enumerate the full
affected integration/VM/assembly targets in the receipt; the
[previous backend qualification](benchmarks/6502-late-register-reuse/qualification.json)
is a starting inventory, not a substitute for checking new consumers. Batch cases
by distinct behavior and avoid multiplying every negative control across every
configuration. Broaden or repeat passing suites only for new changes or failures.

No shared NIR/semantic/printer contract change is intended. If implementation
requires one, the AGENTS.md shared-contract checks apply in full; do not waive
them under backend scoping.

The work is complete when the admitted parameter and local cases use one
authoritative binding path, expanded call/CFG proofs and final preservation hold,
strict cost acceptance and exact rollback are qualified, actual savings or
remaining exclusions are reported, and each completed slice has its own commit.
Return-home coalescing remains a separately planned implementation.

## Slice 0 completion

The independent fixture, integration/VM targets and measurement suite are in
place. [Baseline receipts](benchmarks/6502-cross-call-homes/README.md) confirm
the documented TN sizes, inline Ord/Items homes, LF/CRLF behavior and independent
execution oracles. Compiler behavior is unchanged.

## Slice 1 completion

Residence verification now consumes fresh expanded callee entry/write facts,
after structural verification. Bound parameter writes resolve to physical ZP;
unknown allocations remain conservative. Invocation writes never define caller
objects. Tests cover per-byte clobbers, absolute aliases, changed bindings and
allocations, setup interference, extra inputs and unknown calls.

Checks passed: 32 parameter-home, 26 known-callee and 44 verifier unit tests;
23 affected integration tests (one MADS test remains explicitly ignored here).
The independent source images exactly match slice 0 at both qualified VM lanes.
`cargo check --lib --tests` also passes. Automatic cross-call selection remains
disabled. [Check receipts](benchmarks/6502-cross-call-homes/slice1-checks.json)
record the focused executions.

## Slice 2 completion

The cross-call gate now adds one deterministic parameter batch after selected
leaf/handoff placement. Existing callee writes exclude candidate bytes; actual
expanded proofs, preservation replay and strict final-cost acceptance decide the
result. Unsupported or rejected trials return the selected complete artifact.
Already-elided parameter storage is kept on its existing path.

Two shared facts needed refinement: native indexed arithmetic now certifies its
explicit carry initialization, and the native two-byte pointer-offset recipe has
an exact destination write summary while retaining unknown pointee reads. Tests
cover both boundaries; no callee-local selector heuristic replaces those facts.

Independent parameter controls save final bytes with one extra materialization;
LF/CRLF VM checks exercise both feature settings. TN qualification, including a
trial rejected for downstream caller growth, is recorded in slice 3. The gate
remains off in both default configurations.

## Slice 3 completion

The parameter scope is qualified against independent execution controls and TN.
[Receipts](benchmarks/6502-cross-call-homes/README.md#parameter-qualification-slices-23)
record 64/61-byte independent savings and exact TN rollback when four other
routines grow. Raw NIR, recursive/nested/helper/machine fallbacks and the existing
byte-level clobber controls pass. Local selection is still disabled; final MADS
and full backend qualification remain in slice 6.

## Slice 4 completion

Typed local bindings and common physical-byte/pair queries now drive layout,
emission, maps, costs, callee facts and residence verification. Bound locals
allocate no shadow inline bytes and cannot use ordinary-memory range proofs.
The verifier enforces every-path/every-byte initialization even for manual MIR
bindings, plus exposure, alias, ownership and allocation rules. Automatic local
selection is still disabled. [Checks](benchmarks/6502-cross-call-homes/slice4-checks.json)
cover manual byte/word/pointer homes and preserve independent output/map/relocation
hashes with both parameter gate settings.

## Slice 5 completion

Definitely initialized source locals now join parameter placement in the same
batch, using the same reservation mask and whole-object slot choice. Local-only
candidates do not recreate elided parameter captures. New controls cover words,
pointers, zero-parameter callers, exposed addresses and seeded entry values.
[Measurements](benchmarks/6502-cross-call-homes/README.md#combined-parameter-and-local-qualification-slice-5)
record eight additional bytes saved in the original control and exact TN rollback.
Six integration and two VM controls pass; the optimized default remains off until
slice 6 qualification.
