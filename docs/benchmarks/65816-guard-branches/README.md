# Native 65816 short guard branches

Completed on 2026-09-23. The four conditional transfers in each native stack
guard now use the existing typed branch-layout finalizer. Guards shrink from
**45 to 29 bytes**. Every guard, its reservation amount, both unconditional
JMLs, fault behavior and stack protection remain.

The [plan](../../MIR65816_GUARD_BRANCHES_PLAN.md) is complete: baseline and
accounting `74d7c660`, selection and focused validation `dc8106be`, followed by
this qualification/report slice. Production changes are four `branch` →
`dispatch` calls and an API comment. No new pass, instruction form, branch
registry, allocation policy or public ABI was introduced.

## Measured code size

The baseline is the completed call-padding implementation `835f0cb0`, qualified
at `183c7a5d`; planning/baseline commits do not change production emission.
The workload remains frozen Exec `c3500c8`, eight tasks, console, MyDOS and
stack checks enabled. Retained baseline hashes were checked before reuse.

| Measurement | Raw before → after | Optimized before → after |
| --- | ---: | ---: |
| Compiler routine bytes | 520,969 → 483,721 | 489,697 → 452,577 |
| All executable bytes | 528,534 → 491,286 | 497,262 → 460,142 |
| Executable bytes saved | **37,248** | **37,120 (7.46%)** |
| XEX file bytes | 545,790 → 507,860 | 513,926 → 476,142 |
| Guard bytes | 104,760 → 67,512 | 104,400 → 67,280 |
| Guard count, unchanged | 2,328 | 2,320 |

The direct forecast is exact, with **zero secondary branch-layout savings**.
All 551 routine contracts, frames, homes, guard amounts/order and platform/ABI
inputs match in each mode. Typed labels, retained fixups and MIR spans follow
the guard-deletion position mapping. Existing non-guard dispatch choices match;
only the four eligible conditionals per guard are added to layout metadata.
An independent instruction walk verifies PER continuation rebasing. All other
instructions match after relocation/position rebasing. Proof-enabled probe
images equal the ordinary CLI's compiler segments.

See [totals and hashes](exec-results.json), [per-routine sizes](exec-routines.csv)
and [all 4,648 guard sites](exec-guards.csv). Added stack, DP and bank-zero
reservation: **0 bytes**. Runtime bank-zero budgets remain 24,672 bytes excluding
OS and 61,536 including OS. Code-bank savings do not enlarge task stack capacity.

The audit retains the existing adapter that asserts and removes only
`stack_checks:true`, which main does not accept as a layout field; main always
emits mandatory guards. The first raw host build reached the build tool's
120-second timeout during concurrent checks. A retry with a 600-second
audit-only command timeout completed. This changes no target inputs or guard
policy. Live Exec sources and its compiler pin remain untouched; hosted boot
qualification and pin integration remain separate.

## Corpus and Dijkstra execution

All 28 Action corpus images shrink by exactly 16 bytes per guard. Examples:

| Optimized routine/module | Code bytes before → after | Cycles before → after |
| --- | ---: | ---: |
| Identity | 51 → 35 | 49 → 42 |
| Direct calls | 303 → 239 | 420 → 385 |
| Sum loop | 120 → 104 | Input-dependent; seven cycles saved per invocation |
| Dijkstra, original benchmark | 5,202 → 4,850 | 1,689,456,541 → 1,688,826,982 |

Dijkstra saves 352 bytes in each mode, retaining all 22 guards. Raw code is
5,790 → 5,438 bytes and original benchmark cycles are
1,816,542,330 → 1,815,912,771. Both modes save **629,559 cycles** on that benchmark.
All cycle reductions in the corpus and Dijkstra are attributed to guard execution.
The common successful path drops from 32 to 25 guard cycles and 11 to nine
instructions. Other success/fault paths are independently measured in the probes.

Stack/DP data traffic, metadata reads, stack peaks, results and non-guard counters
stay equal. Dijkstra peaks remain 96 raw / 86 optimized. All vbcc outputs and
counters are unchanged. See [corpus sizes](corpus-sizes.json),
[routine sizes](corpus-routines.csv), [execution deltas](corpus-execution.csv),
[Dijkstra sizes](dijkstra-sizes.json), [routine sizes](dijkstra-routines.csv)
and [execution deltas](dijkstra-execution.csv).

All 132 Action comparison records pass in debug and release; all 264 records
agree between profiles. Both comparison commands retain exit 101 because the
existing optimized vbcc `unlink` vector 0 fails at `$12ffff` (expected `$00`,
observed `$fc`, store PC `$010030`). See the [comparison status](corpus-execution.json).
Dijkstra passes all 33 cases: 132 records / 264 executions including both
incoming I states, with [execution provenance](dijkstra-execution.json).

## Qualification

The [qualification manifest](../../abi/action65816-guard-branches-qualification.json)
binds the committed compiler, fixture inputs, pinned VM/patch and artifacts.
Full runs used an isolated checkout of `dc8106be` so unrelated local edits could
not change their inputs. Incremental compilation and dev/test debug info were
disabled. Local testing stayed within the 65816 backend.

- Native unit tests: **196 passed**, one ignored. Seven root integration,
  CLI and o65 targets: **63 passed**.
- Full native runtime suites: **158 passed, four ignored** in both debug and
  release. All **484 input hashes and 804 artifact hashes** agree.
- Guard accounting decoder: three focused tests cover both encodings, moved
  origins/faults, every single-byte mutation, truncation, overlapping candidate
  bytes, bank wrap and missing entry/call guards. Static overhead is the sum of
  verified range lengths, and guard counts are checked against routine metadata.
- Independent ca65 references and actual emitted guards match at success/fault
  exits for **3,072 boundary cases** (9,216 executions including both references),
  covering all legal input N/V/Z/C/I combinations. Full A/X/Y/S/P/D/DBR and ordered
  ceiling/floor reads match, with no guard writes. See the
  [baseline](baseline.json) and [selected observations](selection.json).
- **1,440 interrupt probes** cover every reached entry/direct/indirect guard
  boundary in both task domains, modes and I states: 1,080 delivered IRQ/NMI
  events and 360 masked-IRQ controls. Ceiling/floor equality and a safe fault
  path preserve full state, task DP, caller memory and the final guard decision.
  Existing two-task switching/preemption tests also pass.
- Serialized o65 calls execute at both placements with moved fault adapters;
  exact-floor success and failure stop at the correct guard exit. Existing
  mixed calls, indirect PER continuations, ABI results and asynchronous
  relocation tests remain green.
- An isolated CRLF checkout converts 25 text fixtures, including assembly and
  the emission snapshot. **35 native tests and one root snapshot test pass**;
  all 460 emitted artifacts match LF. Corpus and Dijkstra generators also pass
  actual LF/CRLF build comparisons.

The intentional state-boundary snapshot change covers 24 routine records:
exactly 32 guard sequences shrink, with unchanged frames and otherwise identical
machine instructions. Labels/fixups/spans rebase accordingly. No NIR/printer
contract changes, generic branch expansion or test-budget waivers were required.

Retained local evidence is under `target/guard-branches/`, including copied
qualification manifests and the full debug artifact set. Frozen Exec outputs
are under `exec816/build/code-size-detail-20260923/exec/build/` in
`shell-guard-branches-raw` and `shell-guard-branches-opt`. Temporary qualification
and CRLF source checkouts are removed after retaining the evidence.
