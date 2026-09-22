# Native 65816 analysis foundation: baseline gate and physical effects

Implementation slices 0 and 1 are complete. All 28 raw/optimized Action builds
and all 264 comparison records match the frozen INX baseline. The public ABI,
stack guards, homes, allocation, image v3 and o65 profile v1 are unchanged.

## Delivered scope

Slice 0 (`51a5499`) adds the strict
[equality gate](../tools/compare65816/check_analysis_rewrite.py). It authenticates
the actual comparison inputs against the frozen baseline, compares complete
artifact and observer records, and requires exactly the known external failure.
Negative controls reject changed bytes, fixups/maps, frames/guards, control
targets/predicates, traffic/cycles, forwarding counters, hashes and failures.

Slice 1 moves the admitted instruction forms to
[selected.rs](../src/mir65816/emit/selected.rs). The tracked emitter dispatches
every form through [physical effects](../src/mir65816/emit/effects.rs) and the
existing encoding/state update. Effects distinguish physical register lanes,
independent flags, execution environment, ordered memory accesses and control.
M/X width determines data access; encoded operand size is a separate property.
Unknown forward values do not erase concrete reads/writes from the effect model.

Verified native call plans supply stack argument reads and declared result lanes,
including discarded results and zero extension. Other register/flag values and
the 64-byte scratch region are call-clobbered. Callee and indirect-memory possible
writes cannot establish initialization or kill a prior stored definition.
The indirect PHK/PER/PHA/RTL protocol retains its protected physical phases;
its RTL is classified as a call, separately from a routine return.

The optional proof observer records effects and encoded ranges separately from
historical state snapshots. Branch relaxation remaps those ranges. It covers
2,399 typed forms across the 28 Action builds, including guard paths, with equal
traced/untraced images. A compound conditional dispatch occupies one record.
Ordinary builds do not retain this observer.

This slice supplies the effect vocabulary. Selected actions/compiler events,
stable sites, CFG construction, home and register/flag liveness, replay and
checked rewrites remain later slices of the
[implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md).
No optimization consumes these effects yet. Host compilation overhead was not
measured here; the final foundation qualification requires that measurement.

## Validation

The [qualification record](abi/action65816-analysis-effects-qualification.json)
records source hashes and the parent revision of the tested worktree. It is
separate from all historical evidence. Checks passed:

- 82 comparison-tool tests, including the new equality negative controls.
- 115 native library/emitter tests; the eight effect tests also passed after
  strengthening the independent result-lane expectations.
- 61 affected root integration tests covering emission snapshots, ABI, flat
  images, experimental o65, state boundaries and both CLIs.
- 120 native tests in each host profile, including calls, relocation, guards,
  IRQ/NMI and preemption. All 649 historical native artifacts remain identical;
  the sole added artifact is the separate instruction-effect coverage summary.
- New effect checks cover every admitted family, legal widths, hidden A,
  carry/RMW dependencies, DP/stack ranges, transfers and call/return contracts.
  Independent VM probes execute 312 width/flag/I combinations, compare actual
  bus access addresses and check preservation outside declared write/clobber
  masks. Raw/optimized production tests also check all scalar result classes
  through direct, indirect and discarded-result calls.
- The corpus rebuild verifies 28 Action CRLF builds. Debug and release produce
  equal complete results: 264 paired records, 528 executions per host profile.
  The strict gate accepts all 56 Action/vbcc builds and 224 artifact files.

The ignored comparison target exits with its known optimized vbcc `unlink`
vector-0 failure. Every Action result passes; the gate requires this precise
external failure and rejects additional failures.

| Optimized case | Code bytes | Cycles | Additional stack bytes |
| --- | ---: | ---: | ---: |
| Rotation | 126 | 735 | 8 |
| Sum loop (13) | 120 | 1,092 | 6 |

See the [exact equality result](benchmarks/65816-analysis-rewrite/slice1-equality.json)
and [coverage summary](benchmarks/65816-analysis-rewrite/instruction-effects-summary.json).
No golden output was refreshed. Full root tests/NIR sweeps and an isolated CRLF
fixture rebuild were outside this slice: there are no NIR, semantic, shared
runtime or newline-sensitive fixture-handling changes. The isolated rebuild
remains required for replay/pilot qualification.
