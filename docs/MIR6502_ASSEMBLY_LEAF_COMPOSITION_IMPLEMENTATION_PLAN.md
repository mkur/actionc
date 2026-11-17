# MIR6502 certified assembly-leaf composition implementation plan

Status: slices 1–2 implemented; final qualification in progress, 2026-11-17.

Implements [issue 9](MIR6502_FACT_REPAIR_PRIORITIES.md) under the
[certified assembly-leaf design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-certified-assembly-leaves-in-acyclic-call-chains).
Commit the note/plan first and each implementation slice separately. Use the
existing shared certificates and ordinary-call transaction; no decoder, NIR,
source syntax, assembly result-definition or private-alias extension.

## Slice 1 — Compose certified entries

- Treat a machine routine as a terminal acyclic dependency only when the
  existing ActualEntry leaf producer certifies its complete body. Keep ABI
  admission, recursive/missing-dependency rejection and native composition.
- Retain the certificate's source, machine payloads, backing and stage through
  native ancestors. Cache within the immutable analysis context only. Report
  uncertified machine dependencies through existing diagnostics.
- Keep assembly `defined_results` empty. Check direct/acyclic agreement, nested
  and repeated calls, unsupported bodies, transitive freshness and bad later
  call sites. Run focused entry-analysis tests and commit.

## Slice 2 — Admit certified residence boundaries

- In acyclic residence contexts, replace the machine-body veto with mandatory
  leaf certification. Uncertified assembly cannot use the ordinary native
  missing-entry fallback, even at a dead boundary. Preserve leaf-only contexts.
- Keep canonical argument/result checks at every site. Separate certified input
  admission from finite write coverage: Unknown writes allow dead lifetimes but
  cannot preserve live values. Do not broaden incoming flags/scratch supply.
- Reuse backing-aware writes, per-object lifetimes, observation checks and the
  one combined proposal. Test disjoint/overlapping lanes, Unknown reads/writes,
  unsupplied inputs, scoped admission, and an expansion conflict with exact
  artifact rollback. Run affected residence/planning tests and commit.

## Slice 3 — Qualify execution, placement and TN

- Add one independent void-assembly/native-wrapper control with live caller
  byte/word values, observable sentinels and stack balance. Require a real home
  placement and packaged-byte saving. Run a focused VM group for both runtimes,
  default/optimized configurations and LF/CRLF through actual source loading.
- Retain a pre-extension image for the independent source and the existing
  13-control/95-routine audit. Compare the same source/configuration against the
  pre-extension compiler. The existing acyclic gate-off output is a scope control,
  not a switch isolating this extension from native-chain placement.
- Extend the retained audit assertions for certified assembly composition,
  unproved result definitions and remaining TN blockers. Run it once at final
  qualification with counts/timing; record actual images, facts, homes, routine
  deltas and trial counts. Issue 8's TN reference is 9,926 bytes.
- Reuse focused terminal/core-validity controls. No full suites by default.
  Broaden only for observed failures. Update design/status/plan, save a receipt,
  check documentation links and diffs, remove temporary hooks, and commit.

## Acceptance

Certified machine leaves can supply entry facts to native acyclic ancestors and
preservation facts to residence consumers without becoming movable source-home
candidates themselves. A live home requires complete compatible writes. Every
candidate retains its own facts/maps and the existing final-cost, physical
observation, compatibility and rollback guards. No clobber or declared return
type becomes a guaranteed result definition.

TN savings are measured, not required. `MovePage` may gain composed entry
admission while retaining Unknown writes; `CalcAdr`, `Internal` and `Ascii`
remain unsupported without a separate entry repair. Recovering issue 8's core
fallback cost is outside this plan.

## Progress

Slice 1 composes complete ActualEntry machine-leaf certificates without changing
result definitions or decoder coverage. Two grouped regressions cover a native
diamond, repeated queries/sites, opposite routine order, unproved results,
transitive freshness and unsupported/mixed bodies. All **17 focused entry-analysis
tests pass**. The independent pre-extension control is 88 bytes; the retained TN
baseline is 9,926 bytes.

Slice 2 admits machine callees only through the acyclic certificate path, checks
each site's canonical arguments, and keeps Unknown writes separate from entry
admission. Four grouped regressions cover both word lanes/absolute aliases,
live versus dead lifetimes, Unknown reads/writes, forbidden fallback, extra
inputs, scoped admission, real placement and exact candidate rollback. The
independent fixture is exercised with LF and CRLF. **162 focused tests pass**,
including existing entry, residence, planning, core-validity and terminal
transaction controls; three diagnostics are ignored (the audit runs separately).
