# Native 65816 selected actions and CFG

Slice 2 of the [analysis implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
is complete. The compiler retains a typed selected-action stream and CFG in
ordinary builds, independently of optional test traces. Current selection,
ABI v1, stack guards, allocation, image v3 and o65 profile v1 are unchanged.

## Recorded contract

The tracked emitter records each instruction exactly once through its existing
dispatcher. It also retains label allocation/binding, mode requests including
omitted REP/SEP, body anchors, home registration, barriers, forwarding
capture/consume attempts, MIR-entry obligations, X requests and source boundaries.
Nested request/end markers identify which instructions belong to a compound
facade request. They retain inputs; they do not supply saved proof permissions
or a replay implementation.

`SelectedRoutine` owns the allocation snapshot, records and graph. A site has
an owner token, RoutineId, allocation generation, selection generation and
ordinal. Identical routine IDs and byte offsets in separate compilations do not
make sites interchangeable. An immutable clone retains its snapshot identity.
Layout may remap encoded ranges but cannot change sites or edges.

MIR operation spans are separate from selected sites. Empty source spans keep
their events, and fused comparisons explicitly name their terminator. Compiler
setup and block bindings may have no MIR operation attribution. No semantics
are reconstructed from display strings or disassembled bytes.

The action-level CFG implements the existing shared `DataflowGraph` interface.
It retains both successors of a compound conditional, internal compare/staging
paths, zero-byte fallthrough, loops, return exits and stack-overflow exits. Calls
are summaries with return continuations. An indirect RTL must use its PER's
exact label, even if another label has the same byte offset.

Construction checks labels, nested requests, mode requirements, stack changes,
body anchors, reachable environment joins, and MIR predecessor multiplicities
and reachability. It preserves conservative joins and distinguishes unreachable
compiler metadata after terminal transfers from executable paths. Invalid
boundaries reject construction. Current forward-state observations remain
observations; backward home/register liveness and checked rewrites are deferred.

Before and after branch relaxation, records reconcile with the entire code,
symbolic and PER fixups, labels, MIR source spans and transfers, conditional
dispatch metadata and optional effect observations. Historical state snapshots
remain on valid selected boundaries. Proof-feature queries return immutable
observations and validate site ownership and generation.

## Qualification

The [qualification record](abi/action65816-selected-actions-qualification.json)
identifies the tested worktree by its parent revision and exact source hashes.
It retains the following new observations separately from historical evidence:

- [Selected-action inventory](benchmarks/65816-analysis-rewrite/selected-actions-summary.json):
  7,137 sites, including 2,399 instruction records, across all 28 raw/optimized
  Action builds. It covers 20 request kinds and 12 fused source spans; 6,939
  sites are reachable. Traced and untraced builds have matching structure and
  equal executable images.
- [VM path checks](benchmarks/65816-analysis-rewrite/selected-cfg-execution.json):
  24 executions covering raw/optimized sum loops, inputs 0/1/13, I clear/set and
  stack-guard success/failure. All 1,904 observed selected transitions and
  terminal exits match the CFG.
- [Exact equality](benchmarks/65816-analysis-rewrite/slice2-equality.json):
  all 56 Action/vbcc builds, 224 artifact files and 264 full comparison records
  match the frozen INX baseline. Each host profile executes 528 cases. The
  rebuild also verifies 28 CRLF Action builds.

Checks passed: 124 native library/emitter tests, 61 affected root integration
tests, and 123 native tests in each debug/release host profile. Nine new unit
tests cover graph structure, shared-solver compatibility, request nesting,
omitted mode requests, duplicate predecessor obligations, stale/foreign sites,
layout identity, malformed records and PER continuation identity. Three new
native tests check production recording, indirect continuations and VM paths.
The full native suites cover relocation, calls, stack guards and IRQ/NMI
preemption. All 650 artifacts from slice 1 remain identical; the two added
artifacts are the inventory and VM checks above.

Two preflight regressions now compare code and execution state explicitly,
because failed edge preflight still records the existing barrier request. Their
machine-state guarantees are unchanged. No golden output was refreshed.

The ignored comparison target retains the known optimized vbcc `unlink`
vector-0 failure. The equality gate requires exactly that external failure;
every Action result passes. Representative optimized output remains rotation
126 bytes / 735 cycles / eight stack bytes and sum-loop(13) 120 bytes / 1,092
cycles / six stack bytes.

Full root tests/NIR sweeps and an isolated CRLF fixture rebuild were outside
this slice: no NIR, semantic, shared-runtime or newline-sensitive fixture
handling changed. The isolated rebuild remains required for replay/pilot
qualification. Production recording adds compiler work and memory; measuring
that overhead remains part of final foundation qualification.
