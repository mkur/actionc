# MIR6502 argument setup A to X forwarding plan

Prepared 2026-11-11. Status: slices 1–3 implemented; optimized default remains off.
Compiler baseline: `1122b42c`.
Implement in four vertical slices, committing each after its checks pass.
This plan includes the bounded scheduling contract; no new executable MIR
operation or separate design note is required.

Retain a byte already in A through argument preparation by copying it to X
beside its existing store, then remove the later reload into X. Generalize
the existing argument-staging mechanism beyond arithmetic WORD producers,
private captures and the WORD/WORD/BYTE scalar-callee gate. Keep all stores,
homes, source evaluation and calls intact.

The [current measurements](benchmarks/6502-pointer-load-sharing/README.md)
put `Strcat` at 114 bytes, TN cartridge at 9,862 bytes and the sampled `Strcat`
call at 356 cycles. Its remaining `LDX $A1` can potentially become an earlier
`TAX`, saving one byte and one cycle. Broader applicability needs typed proofs
and final measurement rather than a `Strcat`-specific rule.

## Candidate evidence and scope

A preliminary scan of the current cartridge listings finds these windows in
both TN variants. The scan checks simple operation shapes; it is not a legality
proof and must not become an optimizer over printed MIR.

| Routine | Stored byte home | Intervening operations before X reload |
| --- | --- | ---: |
| `MoveMenuBar` | allocated `zp1` | 1 |
| `Strcat` | fixed `$A1` | 11 |
| `Draw` | allocated `zp1` | 3 |
| `Inv` | ordinary parameter byte | 1 |
| `TagAll` | allocated `zp1` | 1 |
| `Copy` in TN, `Dbg` in TNDBG | allocated `zp1` | 1 |

Five reloads use zero page and one uses an ordinary parameter home. Replacing
a two-byte zero-page `LDX` with one-byte `TAX` offers one byte/one cycle;
an absolute `LDX` offers two bytes/two cycles. The six windows suggest seven
local bytes per variant before proof and layout. Some may reject, particularly
when entry flags cannot be qualified. No whole-image saving is assumed.

Admit BYTE values in a retained direct store to H, a same-block setup gap and
a matching direct BYTE reload of H into X. H may be an allocated private
spill/zero-page byte, an ordinary parameter/local byte or an existing backend
fixed staging cell such as `$A1`. Establish ordinary RAM and placement through
existing frame/layout facts. Exclude absolute/hardware-backed, unresolved and
unsupported storage; fixed backend staging cells are not arbitrary absolute
source locations. H need not be private because its store remains.

The producer of A lies outside the rewrite window. It may be arithmetic, an
indirect load or an incoming parameter; no WORD producer pattern is required.
The subsequent direct call must consume X through its verified materialized
ABI. Initially admit canonical layouts of at most five argument bytes, with
BYTE lanes already explicit. Include ordinary Action and machine routines
without requiring a particular source signature or callee name. Indirect,
variadic/aggregate and runtime-helper calls retain existing paths.

SemIR and NIR keep their existing computation and source meaning. MIR6502
owns register scheduling, flags, byte identity and aliases. Emission writes
ordinary `Move`, `Load` and `Store` operations. No source parsing, ABI change,
capture/home deletion, read deferral or post-call pointer retention is planned.

## Schedule and proof window

The following is schematic typed MIR. G is the unchanged setup gap; T is up
to three unchanged simple A/Y setup operations before the call.

```text
Original                         Replacement
store.b H, A                     store.b H, A
G                                X = move.b A
X = load.b H                     G
T                                T
call R(... X ...)                call R(... X ...)
```

Rewrite only the region from the retained store through the X reload. T and
the call provide a checked use/flag witness but remain outside that mutation.
The operation count is unchanged: one move replaces one load. The original
store and every gap/tail operation retain their operands and relative order.

Bound the full original store-to-call region to 64 operations, including T
and the call. Stop at a block boundary or the first call/helper/machine block/
barrier. Use one transfer template and its existing fallback, without CFG
scheduling or competing register-allocation searches.

Initially allow gap operations needed by the frozen candidates: byte loads
and immediates into A/Y, explicit byte Add/Sub in A, direct byte stores from
A/Y and ordinary fixed-pair indirect byte loads into A. Audit their complete
shared effects. Reject an operation unless its source/address inputs, flags,
register changes and memory writes are fully supported. Indexed-X accesses,
X-based address construction, stack operations, opaque/unknown writes and
unmodeled effects reject. Expand this closed set only with separate tests.

## Required shared proofs

Expose a typed query through the post-home rewrite context, provisionally
`analysis/argument_x_forwarding.rs`. Its witness records routine, generation,
store/reload/call sites, H's byte identity and physical address, all gap/tail
sites, the retained store and the register/flag obligations. Reconstruct the
replacement from current typed operations and revalidate at application.

1. **Captured byte equality.** The retained store writes the exact incoming A
   byte to H. The removed reload names that byte, and no gap write can change
   it. Resolve every write's full physical range, including separately named
   aliases and ABI/pointer preparation. Distinct IDs do not prove disjointness.
   Missing placement or an unknown write rejects; no incoming H definition is
   needed because the retained store establishes it.
2. **X lifetime.** The original X value is dead after the store until the old
   reload. Every gap operation must neither read nor overwrite X, including
   implicit inputs and address modes. Dead incoming X alone cannot prove that
   the newly forwarded value survives. Supply the same X value at the old
   reload point and to all subsequent users.
3. **Inserted flags.** `TAX` preserves A/C/V/Y/SP but sets N/Z. Require N/Z dead
   after the retained store through the first original flag definition. Do
   not assume A's current byte already describes the live flags; a preceding
   compare can leave a different N/Z value.
4. **Removed flags.** Deleting `LDX H` also deletes its N/Z definition. Prove
   those flags unobservable after the old reload, accounting for T, successors
   and implicit call inputs. A preserved Y load/immediate in T can restore
   equal N/Z before a machine call. Without such restoration, require complete
   input-effect/ABI facts proving N/Z unused; unknown or opaque call effects
   cannot supply that permission. Keep such candidates on the baseline path
   when the existing analysis cannot establish it.
5. **Observations and exits.** The removed H read is ordinary RAM. All other
   reads remain once and in order, including indirect reads that may observe
   H, ABI cells or pointer scratch. Every store survives with equal value and
   order. A/Y/C/V/SP, call arguments and all surviving memory agree; changed
   N/Z must satisfy the proofs above. Preserve published callee A and X/Y
   preservation facts consumed by earlier selectors, plus tracked flag facts.

Use the shared liveness, home/value and physical-memory analyses. Do not
maintain a second alias scanner or infer safety from a producer's printed
shape. The structural dependency checker alone cannot establish these value
and machine-state properties.

Use the current driver support for supplied physical maps and typed witnesses.
If another witness variant is needed, replace the current single-purpose
optional witness with a small typed enum while retaining pointer-load-sharing
validation. Keep that representation change inside MIR6502 rewrite metadata.
The driver must reconstruct the new proof against its current snapshot/map;
an asserted Boolean or rule-name match cannot authorize early X clobbering.

## Integration and qualification gate

Keep the implementation in the existing argument-staging family. A small
`materialize/argument_staging/x_forwarding.rs` submodule is appropriate if it
keeps the original restricted selector readable. Reuse register-copy effects,
physical-reference emission, structural plans and routine/home costing.

| Code home | Planned work |
| --- | --- |
| [argument_staging.rs](../src/mir6502/materialize/argument_staging.rs) and optional submodule | General transfer discovery and final transaction |
| `analysis/argument_x_forwarding.rs`, [context.rs](../src/mir6502/rewrite/context.rs) | Shared typed byte, X-survival and flag proof |
| [register_copies.rs](../src/mir6502/materialize/register_copies.rs), [record_bases.rs](../src/mir6502/analysis/record_bases.rs) | Existing physical maps and ordinary-storage facts |
| [effects.rs](../src/mir6502/analysis/effects.rs), [machine_liveness.rs](../src/mir6502/analysis/machine_liveness.rs), [machine_values.rs](../src/mir6502/analysis/machine_values.rs) | Complete inputs/clobbers and shared state queries |
| [plan.rs](../src/mir6502/rewrite/plan.rs), [driver.rs](../src/mir6502/rewrite/driver.rs), [posthome.rs](../src/mir6502/rewrite/posthome.rs) | Typed witness validation and existing dependency checks |
| [mod.rs](../src/mir6502/mod.rs), [passes.rs](../src/mir6502/passes.rs) | Late entry point and independent feature control |
| [pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md) | Scheduling invariant, flag guarantees and rejected forms |

Add a late entry point such as `argument_staging::select_x_forwarding_costed`
after pointer load sharing. Preserve the existing earlier argument-staging
and register-copy passes and their policy. This phase can use the completed
setup without changing previous inlining, homes or cost trials. Reuse common
mechanisms without relocating the old passes or weakening their admission.

Introduce `enable_argument_x_forwarding` under the peephole master switch.
Leave it false in conservative/default and optimized configurations until
qualification completes; then enable the optimized default. Feature-off uses
the entire current pipeline, including all three preceding argument/pointer
optimizations, unchanged.

Apply one selected plan per rebuilt snapshot. Use the count of remaining
direct X reloads as the decreasing rank; operation count does not decrease.
The initial reload count plus one bounds iterations. Overlapping register
lifetimes can conflict even when mutation windows do not; test those cases
and avoid applying plans from a stale common snapshot. Do not add a general
cleanup round that removes stores or homes made dead by forwarding.

Require unchanged frames, storage capacities, reservations and occupied zero
page; no affected-routine growth; no whole-image growth; and a strict final
XEX saving. Recheck published exits and replay accepted transactions from the
baseline using the combined proposal's physical map. Restore rejected routines
before final replay. Unknown cost, lost proof, verification failure or a
combined-layout mismatch retains the complete accepted baseline.

Bound costing to two physical maps, three routine/home measurements and two
final XEX measurements per proposed batch, at most seven extra top-level
emissions and no extra whole-program materializations. Use a cheap shape scan
before maps/analyses, and record actual counts and compile timings. Avoid
whole-program trials per site.

Report `argument-x-forwarding-candidate`, `selected`, `applied` and blockers
for shape/budget, storage/placement, intervening alias write, live incoming X,
X read/clobber, inserted/removed N/Z, call inputs, stale proof, published exits
and final cost/replay. Count final applications only after acceptance, with
stable site accounting across snapshots. Reporting must leave images equal.

## Slice 1 Freeze the misses and establish proofs

1. Freeze compiler/source/dependency hashes, raw/materialized MIR, listings,
   images, frames and zero page at `1122b42c`. Preserve the six-window listing
   census as unqualified candidates, then classify it using typed facts.
   Store large artifacts under ignored `build/argument-x-forwarding/before/`
   and a compact manifest under `docs/benchmarks/6502-argument-x-forwarding/`.
2. Add `fixtures/runtime/argument_x_forwarding.act`, integration target
   `mir6502_argument_x_forwarding` and VM target `argument_x_forwarding`.
   Start with the confirmed mixed setup `Observe(p+p^+1,q+1,q^)` and an
   independent callee recording all ABI bytes. Add parameter-BYTE and private
   capture siblings with live callers; inspect normal optimized-NIR/inlining
   output before treating them as misses. An already-forwarded or inlined
   sibling is a control, not a reason to disable previous optimizations.
3. Add inert typed queries and rejection tests for physical aliases, hardware
   homes, incoming X uses, indexed-X inputs, X clobbers, both N/Z obligations,
   absent call facts, wrong owners and stale/mismatched witnesses. Include
   scalar and machine-callee contexts without coupling proof to source names.

Acceptance: at least one neutral optimized-NIR miss is confirmed, distinct
home/producer classes have focused proof coverage and all baseline images
remain unchanged. Freeze TN/TNDBG in both runtimes and neutral images at
`$3000`/`$41C7`; do not count earlier selectors' savings again.

Suggested commit: `Freeze argument X forwarding baselines and proofs`.

## Slice 2 Select retained-store forwarding with final costing

1. Add the late argument-staging entry point and the independently disabled
   feature. Replace only the matching reload with an earlier A-to-X move.
2. Apply through shared structural transactions and typed driver validation.
   Preserve all gap/tail operations, stores, ABI homes and call effects.
3. Add bounded routine rollback, physical-map replay, exit-contract checks and
   exact final XEX qualification. Test failed/unknown cost and combined replay
   fallback without publishing a partial invalid program.
4. Test stable reporting, idempotence, reload-count convergence and overlapping
   X lifetimes. Include independent admitted/rejected routines and existing
   forwarding paths that remain better or already remove the reload.

Acceptance: a neutral case saves the measured one/two bytes with correct ABI
values and retained writes; rejected cases preserve their previous strategies.
Defaults remain off pending final qualification.

Suggested commit: `Select generalized argument X forwarding with costing`.

## Slice 3 Qualify flags aliases and call boundaries

| Area | Required cases |
| --- | --- |
| Byte/state oracle | All 256 forwarded bytes; boundary incoming A/X/Y values; C/V preservation; inserted and removed N/Z live/dead cases; stack balance |
| Home/alias | Private spill, allocated ZP, fixed staging and ordinary parameter/local homes; physical aliases under different IDs, partial WORD writes, unresolved/absolute/hardware homes |
| X survival | Explicit read, write, indexed-X access, address-builder input, opaque machine/helper, earlier argument use and neighboring/nested candidate windows |
| Flags/calls | N/Z use in the gap, N/Z consumed after reload, tail restoration, missing/implicit machine inputs, scalar/machine calls and every rejected call class |
| Observation | Indirect reads aliasing H/ABI/pointer scratch; unchanged source read order/count apart from the one ordinary H reload; identical retained store traces |
| Structure | 64-operation boundary and exhaustion, tail limit, extra block/edge uses, stale generations, wrong replacement and changed final physical map |
| Runtime | Raw/optimized NIR, both runtimes/origins, repeated runtime values, guards and actual LF/CRLF parsing |

Use an independent emitted-instruction oracle and callee-entry VM observations.
Compare full ABI bytes, A/X/Y, relevant flags, SP, source-visible memory and
ordered writes. For cases justified by dead flags, compare the full state at
the later restoration/observation point rather than inventing flag equality
immediately after the removed load. Isolate aliases with consistent entry
values instead of seeding over code or conflicting carrier bytes.

Execute exact measured TN/TNDBG images, including string/overlap boundaries
and both post-call parameter-home overwrite probes. The 20-byte post-call
reconstruction remains required. Exercise feature interactions with restricted
argument staging, pointer byte increment fusion and pointer load sharing;
keep each other control fixed within a measured pair.

Acceptance: admitted cases meet the complete observation contract and unsafe
register/flag/alias cases demonstrably compile through the baseline path.

Suggested commit: `Qualify argument X forwarding flags aliases and ABI`.

## Slice 4 Measure applicability and enable optimized selection

1. Export a corpus through normal entry-reachable linking and optimized NIR,
   toggling only the new feature. Include both neutral home families, all six
   candidate TN routines, existing argument/pointer fixtures, predicates,
   string loops, graphics and resident runtime calls. Distinguish absent
   templates from failed register/flag/alias proofs and final cost rejections.
2. Record source/image hashes, final routine/XEX sizes, code/reserved extents,
   frames, zero page, removed loads, retained writes, target cycles, timings
   and costing bounds. Report actual TN results; `Strcat` 114→113 bytes and
   356→355 sampled cycles are conditional local targets, not promised outcomes.
3. Pin the new feature off in historical exporters and baseline-sensitive
   helpers for all preceding measurements. Preserve their original manifests,
   images and role tables. Require fresh default neutral/TN execution to match
   the new measured files and keep feature-composition tests separate.
4. Enable optimized default only after focused qualification, then run the
   complete affected backend checks. Keep conservative/default off. Commit
   compact results under `docs/benchmarks/6502-argument-x-forwarding/` and
   update this plan, the pseudo-machine contract and documentation index.

Acceptance: a qualified neutral saving, reproducible baseline, honest broader
applicability and no storage/image regression. Record blockers without widening
the implementation merely to recover every preliminary candidate.

Suggested commit: `Measure argument X forwarding and enable qualified defaults`.

## Validation commands

Documentation preparation checks content, links and `git diff --check`.
Implementation introduces these focused targets:

```sh
cargo test --locked --lib argument_x_forwarding
cargo test --locked --lib mir6502::rewrite::
cargo test --locked --test mir6502_argument_x_forwarding
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test argument_x_forwarding
```

Final qualification follows [AGENTS.md](../AGENTS.md), scoped to MIR6502:

```sh
cargo test --locked --lib mir6502::
python3 - <<'PY'
from pathlib import Path
import subprocess
targets = sorted(p.stem for p in Path('tests').glob('mir6502*.rs'))
subprocess.run(['cargo', 'test', '--locked',
                *[f'--test={name}' for name in targets]], check=True)
PY
cargo run --locked --bin actionc-mir6502-sweep -- --emit-load fixtures/mir6502
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test argument_x_forwarding --test argument_staging \
  --test pointer_byte_plus_one --test pointer_load_sharing \
  --test register_copy_forwarding --test scalar_captures \
  --test call_predicates --test call_predicate_qualification \
  --test pointer_retention --test tn_codegen_reproducers --test tn_dispatch
```

Run the exact measured-image TN oracle and LF/CRLF listing/MADS relocation
checks for the new feature. Retain historical image oracles with explicit
controls and the frozen Strcat static/VM audits. Run scoped direct-word,
indexed-byte and indirect-field runtime assertions plus their applicable
selection assertions, avoiding unfiltered Cargo fixture wrappers. The existing
indexed-byte selection-count discrepancy remains separate from its execution
assertion. Use `CARGO_INCREMENTAL=0` when needed to bound debug artifacts.

No shared NIR or semantic change is planned. If those contracts change, also
run `cargo test nir_fixtures_match_snapshots`,
`cargo run --bin actionc-nir-sweep -- fixtures/nir` and `cargo test` as required.
Explain materialized-MIR changes as target scheduling; raw NIR stays unchanged.

## Completion record

| Slice | Status | Evidence |
| --- | --- | --- |
| 1 Baselines and proof | Complete, `8fe02793` | [Frozen controls](benchmarks/6502-argument-x-forwarding/baseline.json), three neutral home classes and four inert proof tests |
| 2 Selection and costing | Complete, `99fb571d` | Late selector, shared typed witnesses, physical replay and exact final costing |
| 3 Flags aliases and ABI | Complete, `d8b0ebdf` | Boundary oracle, alias observations, feature matrix and exact TN execution |
| 4 Corpus and defaults | Complete | Optimized default, historical pins and [measured qualification](benchmarks/6502-argument-x-forwarding/README.md) |

The neutral fixture confirms fixed staging, ordinary BYTE parameter and private
zero-page misses with normal optimized NIR/inlining. Its `Forward`,
`ForwardByte`, `ForwardPrivate` and already-forwarded `Plain` routines are
84/11/47/40 bytes; cartridge/standalone images are 360/442 bytes at both
origins. Integration and 768 independent VM calls cover raw/optimized NIR,
both runtimes/origins and actual LF/CRLF source parsing. Neutral images match
the frozen compiler exactly. TN controls reuse the preceding qualified images
from identical compiler code with checked source hashes; their matching
feature-off listings will be exported with corpus measurement.

The inert query distinguishes restored N/Z at machine-call entry from known
scalar entry contracts. The latter currently require a scalar signature,
non-opaque effects without flag inputs and no machine blocks in the callee.
Unknown machine entry flags remain a blocker. Shared context/summaries expose
this fact without changing executable MIR or existing selection behavior.

Slice 2 introduces the late selector with the feature explicitly enabled only
in tests. Typed witness variants preserve pointer-load-sharing validation and
also certify the older restricted argument-staging schedule. That path keeps
its admission policy; leaving its unchanged Y reload outside the mutation
produces the same final MIR and frozen image. The generalized selector proves
early X lifetime and both N/Z obligations, retains all stores and uses the
original X-reload count as its decreasing rank. Final replay/cost checks retain
the accepted baseline on failure.

Eight focused tests, integration and 1,536 feature-off/on VM calls pass.
The neutral routines shrink 84→83, 11→9 and 47→46 bytes; `Plain` stays 40.
Images shrink 360→356 / 442→438 bytes. Sample calls improve 342→341,
291→289 and 444→443 cycles; the control remains 429. Retained writes,
indirect reads and observed callee state agree. Both configuration defaults
remain off pending boundary and corpus qualification.

Slice 3 checks all 256 forwarded byte values in 24,576 emitted executions,
including both incoming carry states, word boundaries, source/capture/pointer
aliases and exact restored A/X/Y/C/N/Z/V/SP. Ten focused tests pass, including
64-operation and tail limits, implicit indexed-X inputs and wrapping pointer
consumers. Three VM tests cover the 1,536-call matrix, 104 alias calls and 64
calls across all eight preceding-feature combinations. Retained parameter,
private and ABI writes agree; indirect reads remain ordered and single.

The corpus exporter was added to supply exact images for boundary execution.
It exports source listings as well as MIR/maps; all eight frozen controls
match and their TN listings have been archived. Eight measured TN/TNDBG images
pass 80 string and 16 post-call parameter-alias cases. `Strcat` is 114→113
bytes and sampled calls are 356→355 cycles. Four TN sites qualify; `Inv` and
`Copy` retain their reloads because call-entry flag facts are unavailable.
The final stricter entry proof also retains TNDBG `Dbg` sites.
Additional typed candidates retain fallback for unsupported setup operations.
The optimized default remains off until slice 4 qualification.


Slice 4 enables the feature in the optimized configuration and pins it off in
historical exporters and baseline-sensitive helpers. All eight frozen hashes
match. Fourteen of 28 runtime/origin pairs shrink and fourteen are identical;
52 applications save 58 bytes across those images. TN is 9,862→9,858 bytes,
TNDBG 12,958→12,954 (cartridge); the predicate-snapshot control also saves
two bytes. Frames, storage capacities, reserved lengths and zero-page use
agree. Fresh default CLI neutral images and TN/TNDBG compilation are required
to match the measured artifacts. Source listings and complete facts remain in
ignored build artifacts; committed manifests hold hashes and compact results.

Final scoped checks cover 1,217 MIR6502 unit tests, 105 integration tests in
18 targets, 169 emission/load fixtures, the affected runtime/ABI targets,
TN panel/dispatch execution, eight LF/CRLF MADS reassembly/relocation checks,
scoped arithmetic/indexed/indirect-field runtime and selection assertions,
and frozen static/VM audits. See the measured report for exact target counts,
execution boundaries, final image hashes and release timing observations.

Final review strengthened scalar-entry facts with CFG entry liveness, treating
nested calls/helpers/barriers as flag observations. A scalar signature and
routine effects alone cannot exclude a wrapper forwarding incoming flags.
The added regression rejects that wrapper and an N/Z-only definition before
its nested call. The full affected-backend qualification was repeated after
this proof change; measured-image equality gates any changed selection.
