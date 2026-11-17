# MIR6502 home admission using existing facts: implementation plan

Prepared 2026-11-17. Status: in progress; slices 1–3 complete.

Implement in five slices, committing each after its focused checks. This is the
next qualified widening of the [home-selection design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#further-widening-follows-measured-blockers),
using the completed [helper contracts](MIR6502_RUNTIME_HELPER_CONTRACT_DESIGN.md)
and [wide routine ABI](MIR6502_WIDE_ROUTINE_ABI_DESIGN.md). No new IR boundary,
contract catalog, allocator or general analysis framework is needed.

## Outcome and baseline

Let source-home discovery consume the contracts that expanded verification
already understands. Admit a qualified operation because its implementation
and physical effects are supported; retain explicit exclusions for expansion
paths that have not been qualified. Admission proposes bindings. The resumed
candidate must establish its own proofs before selection.

Start from `6167df8b` and the [wide ABI qualification](benchmarks/6502-wide-routine-abi/README.md):

| Workload | Packaged bytes | Named-ZP routines | Entry certificates |
| --- | ---: | ---: | ---: |
| TN, `$3000`, optimized ActionCart | 9,913 | 10 | 30 |
| Huffman portable, `$2000` | 3,851 | 0 | 9 |
| Huffman driver, `$8000` | 3,781 | 0 | 8 |
| Huffman portable without leaf inlining | 3,851 | 0 | 9 |

The current `unsupported caller body or signature` diagnostic conflates:

| Routine | Actual discovery boundary |
| --- | --- |
| `ReadBit`, `Main` | No parameters or source locals to place. |
| `ReadBits` | Explicit Lsh32/Rsh32 helper operations are absent from the operation allowlist despite available contracts. `n` and `i` are narrow candidates; `value` is four bytes. |
| `ReadHeader` | Record-index multiplication, word shifts and variable word arithmetic in a routine with pointer reads. |
| Portable `Decode` | Global-address formation for `codes` and `heap` is excluded. |
| Driver `Decode` | Passes body admission; live locals are blocked across `WriteByte` by unknown returning writes. |

## Invariants and scope

- Stay in MIR6502. Retain verified NIR, logical storage identities and existing
  `ObjectAddressControl`, `LogicalHomes`, `ResidenceCalls`, invocation contracts
  and physical-overlap checks. No SemIR lookups or sample-name special cases.
- Keep byte/word source-object eligibility, the four incoming-byte placement
  limit, current ZP pools, whole-object bindings and source exposure rules.
  Four-byte ABI support does not promote LONG source objects into ZP.
- Reuse existing configuration gates. Newly admitted helper boundaries use the
  cross-call/lifetime path; the conservative configuration stays unchanged.
  Preserve the single-call/multiple-call distinction, counting helper sites too.
- Keep one combined cross-call proposal and one continuation. Preserve selected
  bindings, callee-before-caller coordination, descendant exclusions, strict
  whole-image improvement, final layout, preservation replay and exact rollback.
  No subset retries or per-routine cost veto.
- Keep full observations distinct from normal-return writes. A helper's
  successful-return contract grants no purity, flag provenance, read elimination
  or permission to weaken an unknown pointee-write footprint.
- Defer variable pointer arithmetic/fused pointer staging, general multiply or
  division pseudo-operations, variable shifts, arbitrary machine/indirect calls,
  own-frame SArgs placement, recursive residence and general local-only leaf
  discovery. Existing supported calls and operations retain their behavior.

## Slice 1 — Report the actual admission decision

Primary code: [eligibility](../src/mir6502/parameter_homes.rs),
[cross-call discovery](../src/mir6502/parameter_homes/cross_call.rs) and its tests.

1. Replace the boolean-only admission result with a small internal reason type
   and optional retained `MirSite`. Separate structural prerequisites from
   operation checks without changing their accepted set.
2. Distinguish no source objects, incompatible ABI/already-bound frame, incoming
   capture extent, unsupported operation, pointer/arithmetic expansion and
   unsupported effects. Retain later per-object exposure, initialization,
   invocation-preservation and cost failures as separate decisions.
3. Emit reasons through existing trace events. Include the blocking operation
   kind/width/site where relevant. Use one deterministic first blocker; no
   diagnostic-only second analysis, new fact cache or broad tracing subsystem.
4. Record `ReadBit`/`Main` as no candidate objects. Keep absent placement distinct
   from an unavailable callee entry certificate.

Checks: one grouped admission/trace test for distinct boundaries, existing
parameter-home tests affected by the refactor, and a trace-off/on image equality
control. This slice must not change generated code or candidate membership.

## Slice 2 — Consume certified explicit helper invocations

Primary code: cross-call discovery, [residence queries](../src/mir6502/parameter_homes/calls.rs),
[helper invocation](../src/mir6502/runtime_contracts/invocation.rs),
[lifetime verification](../src/mir6502/parameter_homes/lifetimes.rs) and
[observations](../src/mir6502/parameter_homes/observations.rs).

1. Admit an explicit `MirOp::RuntimeHelper` through the existing
   `helper_invocation` contract, checking its exact arguments, results, effects
   and selected implementation. Do not add a second supported-helper list.
   Unsupported, deferred or stale bindings retain an explicit rejection.
2. Use the selected executable's immutable implementation evidence to assess
   the retained operation as a planning request. A missing selected binding is
   unavailable evidence, including when optimization removed the helper.
   Never match retained and expanded operations by block/operation index or
   copy selected evidence into the candidate. Resolve and validate the resumed
   candidate independently, including any helpers that reappear or change.
3. Include helper sites in the existing invocation census and per-object
   lifetime exclusions. Cover routines containing helpers but no ordinary
   calls, through the cross-call path rather than the register-only leaf path.
   Treat helper implementations through their contracts, not as new native
   source-home allocation subtrees. Retain native descendant coordination.
4. For each surviving object, exclude normal-return scratch/result writes.
   Retain setup reads/writes and result-read protection when the object dies
   at the helper. Use the helper's actual result locations, including private
   `$C4–$C7`; do not assume every invocation has public Action return slots.
   Reuse existing ABI observations and scratch reservations.
5. Keep final expanded verification authoritative for all resident routines,
   including unchanged callers. A newly interfering helper or stale contract
   rejects the trial and restores the selected artifact.

Checks: a grouped contract/discovery control for live, dead and defined-after
objects; repeated helper sites; a helper-only routine; both lanes of a word;
unsupported/changed binding; and a forced candidate-time conflict with rollback.
Use a small VM control with a live narrow home across wide shifts in both
runtimes, signed/high-bit values and guards. Reuse existing division/fault
controls if those supported helper kinds become newly reachable. Run affected
residence/runtime-contract tests, not the full repository suite.

Exit: `ReadBits` is no longer rejected merely for explicit Lsh32/Rsh32. Report
its next actual decision even if cost or interference prevents new homes.

## Slice 3 — Admit ordinary global and static addresses

Primary code: shared admission, existing address materialization and ownership/
physical-observation consumers. Make the corresponding small clarification in
the home-selection design when this slice lands.

1. Admit supported word `LeaAddr` forms targeting `Global` or `Static`, using
   the existing structured relocation/storage identity. Retain restrictions
   on exposed local/parameter objects and unsupported address spaces/forms.
2. Qualify address materialization with resident source homes, including
   argument setup, pointer consumers and any fused use. Taking a global's
   address does not by itself expose a different local object.
3. Preserve fixed/numeric observations, final physical alias checks and
   unknown-write rejection. This change grants no preservation to the called
   routine and does not infer a pointee bound from address formation alone.

Checks: one paired symbolic-global/fixed-address control, one static-address
control, and an explicit overlapping observation/exposed-local negative case.
Execute the successful caller-home case in both runtimes and check emitted
addresses at two origins. Portable `Decode` should reach preservation analysis
instead of failing solely on its global `LeaAddr` operations; unknown returning
writes may still prevent its homes.

## Slice 4 — Qualify bounded constant word shifts

Primary code: admission and existing word-shift materialization. Extend the
relevant expansion-scope paragraph in the object-storage design with the exact
qualified forms, retaining its other exclusions.

1. Start with constant word Lsh/Rsh counts 0–4, using existing bounded native
   recipes. Reuse/factor the materializer's selection predicate where needed;
   do not duplicate helper-selection rules in the admission layer or change
   which shift recipe is emitted.
2. Account for both bytes, carries, A/X/Y and flags, overlapping source/result
   homes, reservations and any helper boundary actually emitted. If a path
   cannot establish these obligations, retain a specific rejection.
3. Keep variable shifts, unqualified larger counts, multiplication and the
   pointer-plus-variable-word-arithmetic guard excluded. This slice does not
   claim to unblock all of `ReadHeader` or `BuildTree`.

Checks: one grouped test for both directions at zero/one/four, byte carries and
high bits; count five/variable-count exclusion; live siblings and an overlapping
home. Reuse existing shift execution controls and add a focused source-home VM
case. Run affected expansion and residence tests. Keep zero-shift flag behavior
unchanged and reject an expanded clobber through the existing verifier.

## Slice 5 — Qualify useful placements and remaining blockers

1. Reuse the TN/Huffman probes at matching origins/configurations. Save a new
   compact receipt alongside the wide ABI baseline, leaving old receipts intact.
   Record exact rejection sites, candidate homes, accepted/rolled-back trials,
   packaged and per-routine byte deltas, ZP occupancy and fallback events.
2. Demonstrate automatic placement and correct execution in independent helper,
   global-address and word-shift controls. Distinguish better admission from
   profitable selection. Do not require or promise a Huffman/TN saving.
3. Require supported helper sites and complete `ReadBits`/`ReadHeader`/`Decode`
   entry facts to remain available. Keep `ReadBits.value` inline, and explain
   any remaining `n`/`i`, `ReadHeader` or `Decode` blockers precisely.
4. Run Huffman cartridge/standalone vectors and existing LF/CRLF loading checks.
   Exercise affected TN VM controls if its image changes. Check trace equality,
   parent-gate behavior, whole-image cost acceptance and exact rollback.
5. Broaden once to the affected MIR6502 parameter-home, runtime-contract and
   materialization tests plus relevant ABI integration/VM targets. Reuse passing
   checks; investigate new failures. The six known MIR6502 unit failures are
   recorded in the baseline receipt and must not be silently relaxed.
6. Remove temporary probes, update design scope/plan status, check links and
   commit the qualification. General pointer-arithmetic admission and unknown
   returning-write precision remain separately prioritized follow-ups.

## Validation discipline

Use focused checks after each slice and one combined final qualification. A
documentation-only plan needs content/link checks, not compiler builds. During
implementation use the existing shared target, disabled incremental/debug
output and two build jobs to limit disk growth. Preserve unrelated work.

Normalize host text before newline-sensitive fixture processing, and verify
actual LF/CRLF paths if such processing changes. Preserve binary/ATASCII bytes.
No shared frontend/NIR changes are planned; if required, first revise the scope
and follow the repository's mandatory shared-contract checks.

## Implementation receipts

- Slice 1: 68 existing parameter-home tests passed; the previously recorded
  expanded-entry handoff failure was explicitly skipped. The new grouped
  admission/trace test passed after correcting a reserved name in its sample.
  Trace-off/on proposals and emitted output agree. Production admission is
  unchanged; rejection events now retain a reason and the blocking site/op.
- Slice 2: 71 parameter-home tests and ten runtime-contract tests passed, with
  the known expanded-entry failure still explicitly skipped. The helper-only
  VM control executes 50 cases across both runtimes, keeps `keep` in working
  ZP outside the reserved ABI window and saves eight payload bytes (163 → 155)
  in each runtime. Stale/malformed helpers, repeated-site gating and both-lane
  result conflicts are covered; conflicting trials restore the complete
  selected artifact. Invocation certificates now consume qualified helper
  facts while helper operations retain their physical observation boundary.
  The existing wide divide-by-zero VM control also passed.
- Slice 3: the grouped global/fixed/static-address unit control covers private
  siblings, an exposed parameter and a reserved numeric ZP byte. Static
  `LeaAddr` uses a real static ID; source string arguments ordinarily use
  `StaticAddr` directly. The VM address control passed 64 invocations across
  both runtimes and two origins, preserving the numeric guard and relocated
  addresses. Both table variants save 18 payload bytes (95 → 77 with inline
  table data; 89 → 71 with a fixed table address).
