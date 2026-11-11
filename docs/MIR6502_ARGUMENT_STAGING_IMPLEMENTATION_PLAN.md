# MIR6502 argument staging reduction implementation

Prepared 2026-11-11. Status: all four slices implemented and qualified; see the
completion record for one existing baseline check discrepancy.
Implements the [design note](MIR6502_ARGUMENT_STAGING_DESIGN.md) committed as
`961f30e8`. The compiler baseline is `476cfecf`; freeze fresh artifacts before
changing selection. Each slice below is a separate commit after its checks pass.

Deliver one bounded extension to direct-call argument scheduling, with the
existing ABI, source evaluation and conservative fallback intact. A neutral
call must improve independently of TN. Measure TN/TNDBG without treating the
125-versus-51-byte `Strcat` comparison as an expected staging saving.

## Scope and integration

Admit one same-block producer/call region, direct calls, BYTE and complete WORD
values, and canonical layouts of at most five argument bytes. Keep source reads
and producer order fixed. Use already captured values, constants, relocatable
addresses and supported ordinary direct scalar sources. New indirect/indexed
read schedules, read deferral, nested-call motion, tail/indirect/variadic/
aggregate calls and CFG-wide scheduling remain outside this implementation.
Existing selectors for those shapes continue through their current paths.

No SemIR/NIR change is expected. MIR6502 owns the schedule and machine-state
proofs; emission receives explicit operations and supplies final placement.
Preserve source-visible parameter writes, public return homes and call effects.
Treat register forwarding, capture-store deletion and home deletion as separate
decisions. A useful forwarding schedule may retain a store that is observable.

| Code home | Planned work |
| --- | --- |
| [call_plan.rs](../src/mir6502/call_plan.rs), [abi.rs](../src/mir6502/abi.rs) | Consume existing typed targets and canonical byte homes; preserve ABI assignment |
| [calls.rs](../src/mir6502/materialize/calls.rs) | Extend typed call-expression planning and explicit setup emission beside existing direct and staged paths |
| [pilots.rs](../src/mir6502/rewrite/pilots.rs), [context.rs](../src/mir6502/rewrite/context.rs) | Discover candidates and query generation-scoped lane provenance, use/def, availability and liveness |
| [effects.rs](../src/mir6502/analysis/effects.rs), [machine_values.rs](../src/mir6502/analysis/machine_values.rs) | Reuse exact setup reads/writes, register values and flags; add only missing shared queries |
| [private_cells.rs](../src/mir6502/analysis/private_cells.rs), [scalar_captures.rs](../src/mir6502/materialize/scalar_captures.rs) | Reuse observation and frame-exposure proofs for any store/home removal |
| [known_callees.rs](../src/mir6502/analysis/known_callees.rs) | Use complete, owner-resolved call effects; missing facts remain blockers |
| [driver.rs](../src/mir6502/rewrite/driver.rs), [plan.rs](../src/mir6502/rewrite/plan.rs) | Apply typed transactions, validate their effect projection and invalidate analyses |
| [materialize.rs](../src/mir6502/materialize.rs), [mod.rs](../src/mir6502/mod.rs), [passes.rs](../src/mir6502/passes.rs) | Place selection before generic expansion, add feature control and final costing |

Prefer a small schedule helper beside `calls.rs` if needed; do not reorganize
the materializer. Reuse the existing `CallArgExprPlan` and ordinary call
materialization. Do not add another expression parser, alias scanner, emitter
cache or machine-code decoder.

## First schedule to investigate

Start with the five-byte family below, using runtime inputs and an independent
callee that records all argument bytes:

```action
PROC Forward(CARD first,second BYTE tag)
  Capture(first+1,second,tag)
RETURN
```

This is a fixture candidate, not a confirmed missed optimization. Existing
direct two-word arithmetic selection accepts exactly two arguments; the third
BYTE deserves a fresh raw/materialized inspection before choosing a template.
Keep a two-word sibling as a control already handled by the current compiler.

The first proposed template retains the first word's high byte in X while
keeping its low-byte capture. At the original second-word read point, a
supported direct load can establish its low byte in Y and high byte in `$A3`
without changing X. At the original tag read point, A can carry the tag to
`$A4` without changing X/Y. Reload only the first low byte into A before the
call. Every step must prove its actual clobbers, byte identities and physical
dependencies. Retain high-byte staging stores unless separately unobservable.

Use the complete A/X pair only when every remaining setup step preserves both
registers. Do not assume later word or pointer materialization has that property.
If current forwarding already produces these schedules, freeze another genuinely
missed case within the same scope and record the exact remaining pipeline gap.
Do not count existing savings as new work or widen the design to force a TN win.

## Slice 1: freeze the gap and establish proof queries

1. Freeze the baseline executable/hash, sources and dependencies, compile
   options, raw/optimized NIR mode, raw/materialized MIR, assembly, images,
   routine sizes, ordinary homes and zero-page occupancy. Include TN/TNDBG in
   both runtimes and neutral fixtures at `$3000` and `$41C7`.
2. Add a neutral runtime fixture, an integration target and an independent VM
   argument observer. Proposed homes are `fixtures/runtime/argument_staging.act`,
   `tests/mir6502_argument_staging.rs` and the VM `argument_staging` target.
   Give each routine a live caller so entry-reachable linking retains it.
3. Locate the missed form relative to `collect_call_arg_expr_plan`,
   `materialize_call_arg_expr_plan`, `run_analyzed_call_arg_exprs` and final
   register-copy cleanup. Record which existing guard/path selects staging.
   Confirm the primary missed case with optimized NIR, as used by production
   MIR6502 compilation; raw-NIR coverage cannot substitute for that evidence.
4. Add only the missing shared schedule queries and typed facts. Record owning
   routine, call site/generation, argument ordinal, complete byte lanes,
   reaching definitions, original producer/read sites, final homes, retained
   captures and protected registers/flags/memory. Keep discovery inert.
5. Validate provenance, preservation of X versus A/X, carry dependencies,
   overlapping copies, extra/successor uses and stale-generation rejection.
   A missing ownership or incoming-read-before-overwrite fact is a blocker;
   extending that analysis must be explicit and focused within this slice.

Initially scan at most 64 producer operations for one call, admit at most five
argument bytes and evaluate only the frozen template and its fully staged
fallback. Test these limits. Use shared routine analyses for nonlocal uses;
the local scan limit cannot replace CFG/use-def validation.

Store large artifacts under ignored `build/argument-staging/before/`. Commit
the neutral fixture/oracle, inert proof tests and a compact baseline manifest
under `docs/benchmarks/6502-argument-staging/`.

Acceptance: one reproducible missed case has correct baseline arguments and
an identified pipeline cause. New facts reject unsafe examples and normal
compiler output remains byte-identical. If no missed case exists in scope,
record that result before introducing a new optimization consumer.

Suggested commit: `Freeze argument staging baselines and schedule proofs`.

## Slice 2: select one bounded schedule with final costing

1. Extend the existing typed producer/call family at the phase established in
   slice 1. Retain source reads at their original points. Select register and
   copy destinations without changing source evaluation or delaying home reads.
2. Carry the earlier complete word's certified high byte through X, or its
   complete value through A/X when supported. Check every subsequent setup
   operation, including implicit pointer, helper, Y and flag effects.
3. Return a generation-qualified transaction through the shared driver.
   Reconstruct the schedule at application; prove each removed definition has
   no surviving reader. Keep needed capture stores and source-visible homes.
   Use `MaterializedCallArguments` only within its existing validation rules;
   ordered accesses and store observation require their own proofs.
4. Add an independent `enable_argument_staging_reduction` configuration control
   inside optimized peepholes. Feature-off must use the complete prior
   selection pipeline, including all existing argument optimizations.
5. Add final cost qualification before enabling the optimized default. Reuse
   existing code/home costing and measure actual final XEX length separately
   from the emitter's address span. Require no affected-routine code/home,
   complete-image or occupied-zero-page growth, plus a measured byte or cycle
   improvement. Replay physical alias proofs against the resulting layout.
6. Restore rejected routines and recheck the combined layout. A failed trial,
   unknown cost or alias failure retains the previous valid program. Bound
   costing to one baseline, one selected batch and at most one partial-batch
   repair; reporting may replay the accepted selection. Do not materialize the
   whole program independently for every call site.

Use a decreasing rank appropriate to the rewrite, such as remaining eligible
staging sites; operation count alone may not decrease. Rebuild invalidated
analyses after every accepted transaction. Preserve any better existing
indexed-word, arithmetic, field-transfer or result-forwarding selection.

Report `argument-staging-candidate`, `argument-staging-selected` for legal
proposals and `argument-staging-applied` only after final qualification. Add
site-specific blockers for register/flag clobber, ordered read, extra use,
ABI/source overlap, observation, unknown callee, unsupported shape, analysis
budget and final cost. Deduplicate counts by typed site identity.

Acceptance: an optimized-NIR neutral case supplies correct ABI bytes with fewer
emitted bytes or executed cycles and no image growth. Feature-off reproduces the
baseline. Test rejection, partial rollback, preservation of earlier optimizations
and identical images with reporting off/on. Only then enable the optimized
default; conservative/default configuration retains its previous policy.

Suggested commit: `Select proven direct call argument schedules with costing`.

## Slice 3: qualify aliases, effects and machine state

Batch related boundaries rather than multiplying every input dimension across
every test. Extend the neutral fixture with general counterexamples, keeping
the admitted first-template scope unchanged.

| Tests | Required cases |
| --- | --- |
| Def/use | Redefinition, ambiguous reaching definition, partial WORD, shared argument value, surviving extra/edge/backedge use, stale plans and fixed-point limits |
| Registers/flags | Later producers that preserve X only, overwrite X, overwrite A/X/Y, or require carry/N/Z/V; nonzero high bytes, add/sub carry boundaries and required incidental callee inputs |
| Memory | Exact aliases and partial overlaps with `$A0..$AF`, argument/return cells, both parameter bytes and allocated homes; separately named absolute aliases, exposed frames and copy cycles |
| Effects | Hardware/absolute reads, unknown pointer accesses, intervening writes/calls/machine blocks, nested-call argument clobbers, recursive/opaque callees and observed staging stores |
| Placement | Two code origins, page/wrap boundaries, physical source/home relocation and combined-layout changes after partial rollback |
| Existing consumers | Two-word arithmetic, indexed byte/word arguments, protected indirect field transfers, predicate arguments, scalar captures, call results and indirect-call fallback |

Observe A/X/Y and `$A3/$A4` at callee entry before the compiler's parameter
prologue, using the existing VM hook machinery or an independent bounded
machine callee. Check complete arguments, call count, results, protected
inputs, stack canaries and balance. Poison unused staging cells so a missing
producer or stale capture cannot pass through coincidentally correct memory.

Use BYTE boundaries 0, 1, 127, 128, 254 and 255, and WORD boundaries `$0000`,
`$0001`, `$00FF`, `$0100`, `$7FFF`, `$8000`, `$FF00` and `$FFFF`. Derive expected
arithmetic and argument bytes independently in the host. Exercise repeated
calls, distinct high bytes and modulo-65536 carry/borrow; avoid an unnecessary
Cartesian product of all WORD inputs.

Run feature-off/on with raw/optimized NIR and both runtimes. Normalize host
text before instrumentation and execute actual LF/CRLF inputs through parsing;
preserve binary/ATASCII bytes. Use read traces and a changing-input test to
check original snapshots and exactly-once accesses, including fallback cases.

Acceptance: all admitted cases match the independent oracle and required
machine state. Unsupported shapes retain valid staging. Retaining a capture
or store is tested independently of forwarding; missing alias/effect facts do
not silently become permission to delete it.

Suggested commit: `Qualify argument schedules across aliases effects and ABI state`.

## Slice 4: measure applicability and qualify MIR6502

Count candidates, final applications and blockers in neutral fixtures,
TN/TNDBG, the existing arithmetic/indexed-call fixtures, scalar/predicate
argument fixtures and representative library/graphics programs. Keep every
other optimization option fixed while toggling this extension.

Record source/compiler/image identities, origin, runtime and NIR policy;
routine and whole-XEX bytes; initialized/reserved storage extents; ordinary
homes; declared/allocated zero page; executed staging loads/stores; and target
cycles with wrapper or output-stub boundaries stated. Include compilation time
and materialization-trial counts to expose a costly scheduling implementation.
Do not change unrelated inlining or pointer-retention settings to manufacture
a staging saving.

The current [qualified TN controls](benchmarks/6502-direct-comparisons/README.md)
are 9,873/9,998 XEX bytes for TN and 12,969/13,094 for TNDBG, cartridge/
standalone respectively; `Strcat` remains 125 bytes. Revalidate these against
the frozen compiler and input hashes rather than hard-coding them as permanent
test expectations. Test the exact measured images, including runtime-populated
arguments and overlapping/same-buffer string cases where applicable.

An unchanged `Strcat` result must name its actual blocker: unsupported producer,
setup clobber, physical overlap, observable staging cell, missing callee fact
or final cost. Its unknown indirect accesses may legitimately exclude it from
this first scope. A broader indirect-producer or pointer-retention optimization
requires a separate design slice; do not add a `MovePage` or TN special case.

Run the final backend checks below. Commit compact measurements and commands
under `docs/benchmarks/6502-argument-staging/`; keep dumps, binaries and logs
under ignored `build/argument-staging/`. Update the design/plan status, index
and relevant pseudo-machine contract with the qualified invariant and limits.

Acceptance: at least one neutral call improves, inspected images and affected
routine code/homes do not grow, all required checks pass, and TN/TNDBG results
and blockers are reported independently of the historical Oscar64 whole-body gap.

Suggested commit: `Qualify MIR6502 argument staging and record application measurements`.

## Validation commands

These are implementation-time commands. New target/filter names below are to
be introduced by the corresponding slices. Documentation preparation requires
content/link checks, rather than executing compiler suites.

During development, run the focused proof and new regression targets:

```sh
cargo test --locked --lib argument_staging
cargo test --locked --test mir6502_argument_staging
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test argument_staging
```

Final qualification runs the full affected backend's library tests, all its
integration targets and the emission/load sweep:

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
```

Run affected VM targets for the new schedule and existing capture/call families:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test argument_staging --test scalar_captures \
  --test call_predicates --test call_predicate_qualification \
  --test register_copy_forwarding --test pointer_retention \
  --test tn_codegen_reproducers --test tn_dispatch
bash fixtures/runtime/run-direct-action-word-arithmetic-args-vm.sh
bash fixtures/runtime/run-indexed-byte-fixed-action-args-vm.sh
bash fixtures/runtime/run-indirect-call-fields-vm.sh
```

Some existing fixture scripts include paired backend controls; no broad
classic, 65816 or 68k suite is required for MIR6502-only changes. If staging
changes affect listing/MADS reconstruction, run the affected listing targets
as well. Add relevant result/return ABI VM coverage when touched.

Under [AGENTS.md](../AGENTS.md), changing semantic lowering, NIR, NIR
verification/printing or another shared compiler contract additionally requires:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Existing raw NIR/MIR fixtures should remain unchanged for a selection-only
extension. Explain any intentionally changed materialized expectation as a
target-selection improvement or a correctness fix.

## Completion record

| Slice | Status | Commit and evidence |
| --- | --- | --- |
| 1. Baseline and proofs | Complete, `97b8b4b3` | Frozen compiler/images, neutral entry oracle, three proof tests, integration and VM target; inert output is byte-identical |
| 2. One schedule and costing | Complete, `6311c110` | Captured high-byte carrier, retained stores/homes, exact final costing and physical replay |
| 3. Alias/effect qualification | Complete, `ff75b768` | Eight post-home proof tests, exhaustive emitted arithmetic fragments and three independent VM tests |
| 4. Corpus and backend qualification | Complete | [Measurements and checks](benchmarks/6502-argument-staging/README.md), unchanged TN/TNDBG and the remaining indirect-producer/machine-target blockers |

### Slice 1 evidence

The [baseline manifest](benchmarks/6502-argument-staging/baseline.json) records
fresh neutral and TN/TNDBG images from the frozen compiler. All four TN image
sizes match the preceding qualified controls. `ForwardAdd` and `ForwardSub`
in [the neutral fixture](../fixtures/runtime/argument_staging.act) each occupy
41 bytes. The four-byte sibling is 33 and the plain five-byte sibling is 28.
The neutral cartridge/standalone XEX sizes are 336/418 at both origins.

The five-byte arithmetic form is a confirmed optimized-NIR miss. Canonical
loaded-argument forwarding exposes an adjacent word arithmetic producer/call,
but the direct arithmetic expression selector admits exactly two arguments.
The generic path captures both result bytes, then reloads the high byte into X
after the remaining setup. Those intervening direct setup operations preserve X.
The inert shared fact checks complete reaching definitions, exclusive uses,
canonical five-byte homes, ordinary source storage, a 64-operation limit and
generation identity. It grants no permission to remove captures or source reads.

Three new proof tests and the neutral integration target pass. The VM observes
all argument bytes before the callee prologue and checks independent host
results with raw/optimized NIR, both runtimes, two origins and actual LF/CRLF
sources. For `$00FF` and tag 128, the sampled arithmetic calls take 430 cycles;
the four-byte/plain-five-byte controls take 111/412, including the common
wrapper and callee. The emitted neutral image matches the frozen compiler
byte for byte. Large artifacts and logs are under `build/argument-staging/`.

### Slice 2 implementation

Slice 1 is commit `97b8b4b3`. The pre-home witness now restricts admission to
the qualified ordinary-source, complete-word shape. It runs on an analysis
clone; it does not move the existing call/producer passes. Its generation
facts are never reused after materialization.

The phase choice is narrower than the initial proposal. An early-X probe
merely relocated `LDX` and changed private zero-page coloring without reducing
code. The selected transaction instead runs through the existing post-home
driver after normal allocation and all earlier optimizations. It reconstructs
the complete low/high arithmetic capture, fixed ABI call and actual setup.
It changes `STA high; ...; LDX high; LDY secondLow` into
`STA high; TAX; ...; LDY secondLow`, preserving every capture store, home and
source-read point. Shared liveness proves X and N/Z dead at the insertion and
N/Z dead after the removed reload. The gap cannot read/write X or contain
calls, indirect accesses or arbitrary effects. Final physical addresses prove
that its explicit writes cannot overwrite the saved high byte.

The call, argument bytes, required machine state, frame allocation and stores
remain identical. Exact code/home/XEX costing, pre-emission verification and
physical-placement replay guard the final transaction, with per-routine
rollback and a bounded full-batch recheck. Feature-off retains the preceding
pipeline. Reporting distinguishes candidates/selected plans from final
applications. No capture or home deletion is introduced by this family.

Both arithmetic wrappers shrink from 41 to 40 bytes. The neutral image saves
two bytes in each runtime and at each origin, with raw/optimized NIR. The
sampled calls improve from 430 to 429 cycles, while both existing controls
retain their prior sizes/cycles. Independent callee-entry and host-oracle VM
checks pass for both feature settings and actual LF/CRLF input.

### Slice 3 qualification

Slice 2 is commit `6311c110`. Eight post-home tests now cover register/flag
clobbers, intervening calls and opaque/indirect effects, hardware sources,
unknown/noncanonical callees, physical aliases despite distinct logical names,
the exact 64-operation boundary, stale generations, observed capture stores,
independent accepted/rejected routines and complete fallback on unknown cost.
The emitted arithmetic fragments execute all 65,536 first-word values for
addition and subtraction with feature-off/on (262,144 executions), checking
independent modulo-65536 results, every ABI argument byte and memory guards.

Three VM tests pass. The entry observer compares A/X/Y, `$A3/$A4`, status,
stack depth and watched ABI/private/result writes before the callee prologue;
6,144 calls exercise boundary inputs, both feature settings, both runtimes,
raw/optimized NIR, two origins and actual LF/CRLF parsing. A changing-input
hook proves the first arithmetic operand stays captured when its source home
changes during later setup. An indirect callee observer reads aliases of ABI
cells, private captures and both source parameter bytes, including a page
boundary; feature-off/on preserve the same observed memory. Forwarding does
not authorize capture-store or home deletion.

### Slice 4 results and limits

The [results report](benchmarks/6502-argument-staging/README.md) and
[manifest](benchmarks/6502-argument-staging/results.json) cover 22
program/runtime/origin pairs. Every image is non-growing. Four neutral pairs
save two bytes and one cycle per arithmetic wrapper; the other 18 pairs are
byte-identical. All eight frozen feature-off controls match their baseline
hashes. Ordinary homes, capture stores and declared/allocated zero page are
unchanged. The selected batch uses seven additional emission passes and zero
additional whole-program materializations; corpus timings expose their cost.

TN/TNDBG retain 9,873/9,998 and 12,969/13,094 XEX bytes respectively, with
125-byte `Strcat`. That routine has indirect reads in the destination and
length producers, a computed second word and a machine target without the
admitted scalar signature. No schedule is proposed for it. Executing the
exact measured images checks 80 string cases across both variants, runtimes
and feature settings, including same-buffer and overlapping copies. The
sampled `Strcat` call remains 370 cycles. The neutral and TN cartridge VM
tests also require exact equality with the measured files.

The final scoped checks pass 1,182 MIR6502 library tests, 102 integration
tests across all 15 targets, 169 emission/load fixtures, 27 tests across eight
affected VM targets, the measured-string test and three focused runtime
assertions. No shared semantic/NIR contract was changed. The direct-word and
indirect-field fixture scripts pass. The indexed-byte script's six-selection
expectation fails with zero selections on both the pinned baseline and the
final compiler; its underlying VM assertion passes directly, and its complete
feature-off/on images are identical. This existing count discrepancy is
recorded separately from the passing execution and image checks.

The implementation remains deliberately narrow: direct scalar WORD/WORD/BYTE,
one complete add/subtract-by-constant capture, same-block setup, at most 64
operations, no setup calls/indirect/hardware accesses and no store/home
deletion. Broader source forms and TN's indirect producers require subsequent
design work. No TN-specific selector or unrelated optimizer setting was added.
