# MIR6502 callee entry capture implementation plan

Status: proposed; implementation has not started. Starting compiler: `72131db3`.
Implements the [callee capture boundary refinement](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-refinement-callee-entry-facts-through-parameter-capture).
The [parameter handoff plan](MIR6502_PARAMETER_ARGUMENT_HANDOFF_IMPLEMENTATION_PLAN.md)
is complete and supplies the production baseline. Work in the four slices below,
committing each after its checks pass. Keep unrelated working-tree changes outside
those commits.

## Objective and scope

An ordinary Action callee receiving a word in A/X may capture it into `$A0/$A1`.
Handoff admission must require the incoming A/X value, without additionally
requiring the caller to initialize the callee's working pair. Correct the shared
entry analysis so it accounts for that capture before interpreting body reads.

Retain the existing wrapper scope, canonical ABI, selected parameter homes,
source-storage protections and unknown-effect fallback. Keep machine callees on
their current bounded entry analysis. Do not add general residence across calls,
call-result forwarding, recursive callee analysis, new pointer reconstruction,
another placement search, or compiler-performance refactoring. SemIR/NIR meaning
and shared frontend contracts are unchanged.

Use the existing optimized default and handoff gate. A refinement of entry facts
does not require another production feature switch. Final proof, strict image
improvement, no routine growth in code plus ordinary homes, and complete rollback
remain the acceptance criteria. Additional TN savings are an open measurement,
not a promised result.

## Implementation boundaries

| Responsibility | Existing code |
| --- | --- |
| Canonical incoming lanes and capture construction | [abi.rs](../src/mir6502/abi.rs), [materialize/abi.rs](../src/mir6502/materialize/abi.rs) |
| Shared entry requirements and fact ownership | [known_callees/entry_inputs.rs](../src/mir6502/analysis/known_callees/entry_inputs.rs) |
| Definite memory writes, register and flag inputs | [home_liveness.rs](../src/mir6502/analysis/home_liveness.rs), [machine_liveness.rs](../src/mir6502/analysis/machine_liveness.rs), [effects.rs](../src/mir6502/analysis/effects.rs) |
| Selected input, capture boundary and resumption | [preparation.rs](../src/mir6502/preparation.rs), [materialize.rs](../src/mir6502/materialize.rs) |
| Admission, storage verification and final proof | [parameter_homes/handoff.rs](../src/mir6502/parameter_homes/handoff.rs), [parameter_homes.rs](../src/mir6502/parameter_homes.rs), [verify.rs](../src/mir6502/verify.rs) |
| Regression and measurement harnesses | [handoff unit tests](../src/mir6502/parameter_homes/handoff_tests.rs), [root integration target](../tests/mir6502_parameter_argument_handoff.rs), [VM target](../tools/vm-runtime-tests/tests/parameter_argument_handoff.rs), [measurement harness](../tools/compare6502/parameter_homes.py) |

Share or factor the existing capture construction into a MIR6502 component usable
by emission preparation and analysis. Keep one ABI mapper, one ordered capture
description and the existing liveness machinery. Do not duplicate the prologue
rules in the wrapper selector or suppress all reads of bound ZP addresses.

## Slice 0 — reproduce the false rejection and freeze the comparison

Add an independent ordinary Action caller/callee fixture. Retain a real call and
make the callee use a selected ZP word home. Disable inlining for the isolated
reproducer; also examine normal optimized callers. Use distinct low/high bytes
and observable results so constants or dead-code removal cannot hide the case.

At the selected pre-capture checkpoint, record the actual callee binding, the
reported extra fixed-byte inputs and the handoff rejection. Analyze the same
callee with its actual capture operations present to establish the boundary
mismatch. Commit a passing characterization of current behavior; change its
expected result with the fix. Do not leave a failing or permanently ignored test.

Record existing output and the potential capture/forwarding improvement. A forced
binding or illustrative assembly sequence is proof/debug evidence, not a claimed
production saving. If the source fixture does not reach the intended placement,
identify the blocker before broadening the fix; retain a typed MIR reproducer
for the entry-analysis defect.

Extend the existing handoff harness with the new fixture and save baseline
receipts under `docs/benchmarks/6502-callee-entry-capture/`. Record revision/diff,
compiler library and source hashes, configuration, images, maps/relocations,
routine code/ordinary/ZP homes, entry requirements and work counters. Keep large
artifacts under ignored `build/` and use the existing shared build directory.

Recheck the [qualified baseline](benchmarks/6502-parameter-handoff/README.md):

| Subject | Handoff-enabled baseline |
| --- | ---: |
| TN cart / standalone packaged bytes | 9,853 / 9,960 |
| Strcpy code + ordinary homes | 20 + 0 |
| Fnamecmp code + ordinary homes | 105 + 0 |
| Strcat code + ordinary homes | 113 + 4 |

**Checks and exit:** focused shared-entry/parameter-home unit tests and the
extended handoff integration/VM targets pass. The baseline receipt identifies a
real false requirement and separates it from unrelated exclusions. Compiler
behavior is unchanged. Commit the reproducer and baseline evidence.

## Slice 1 — share capture construction and make the analysis stage explicit

Expose one typed description of the existing direct capture operations. Preserve
their order, register/fixed-slot sources, memory destinations, omitted absent
lanes, retained registers and identity fixed-slot lanes. Preserve the existing
choice to omit compiler capture for applicable machine entries; SArgs/helper
capture remains outside the newly admitted scope. Prologue generation consumes
this same description without changing emitted code.

Introduce distinct typed entry-query inputs for planned capture and actual entry
operations, and retain that stage in the returned fact. Construct the planned
view from the selected pre-capture input and the actual view from the established
materialization boundary. Neither the presence of bindings nor an apparent
instruction prefix establishes the stage. Empty or later-elided capture still
has an explicit stage.

Audit every `MirCalleeEntryFact` query and the verifier path. In particular,
`verify_storage` currently runs at all verification phases, including the
pre-capture check in `resume_core`; changing discovery alone is insufficient.
Thread explicit capture provenance through discovery, pre-materialization
verification and expanded verification. Use an existing phase as provenance only
where its callers establish that boundary; otherwise carry the smallest required
MIR-only state. Do not weaken verification to make the ambiguity disappear.

Tie fact validity to the selected routine/entry CFG, ABI declarations, bindings,
capture description and stage, as well as machine payloads and global backings.
Materializing or rewriting capture invalidates the old fact. Keep the current
conservative scalar admission until the composition is implemented in slice 2.
Update the pseudo-machine contract if new MIR provenance is introduced.

**Checks and exit:** capture construction and ABI tests cover inline, ZP, mixed,
register, absent and identity-slot forms, plus existing capture exclusions.
Stage/staleness tests reject reuse across materialization, changed bindings or
capture elision. Run affected verifier, preparation, parameter availability,
call-ABI and materialization targets. Baseline images/maps/relocations, selected
bindings and continuation counts are identical. Commit this behavior-preserving
refactor separately.

## Slice 2 — compose entry requirements and integrate the corrected proof

For the planned view, compute entry requirements for the ordered capture followed
by the body, using the existing memory/register/flag analyses. Capture writes
discharge body requirements only for definitely initialized bytes; capture reads
introduce their actual incoming ABI dependencies. A retained `$A3` input remains
an entry requirement. A mixed word does not gain a fabricated memory byte.

Account for expansion effects as well as pseudo-operation effects. A capture
from memory can use A and set flags; direct register stores preserve flags.
Support a capture form only when those effects are established from shared
lowering rules. Keep unknown effects conservative. Preserve genuine reads before
overwrite and definitions that are incomplete or absent on a control-flow path.

For the actual-entry view, analyze the current operations once, including any
remaining capture. Never synthesize a second prologue. The before-capture contract
must conservatively cover the supported materialized entry requirements; final
verification still rebuilds the actual fact and checks the outgoing inputs.

Use complete typed MIR declarations, canonical ABI lanes, binding and effect
facts to establish eligibility. Replace reliance on `scalar_signature` only for
the forms those facts fully describe. Include a pointer-parameter positive case
when supported, and retain explicit unknown-contract fallback for incomplete,
aggregate, variadic, helper or opaque cases. Do not relax the separate shared
entry-flag or machine-callee contracts merely to admit a wrapper.

Wire the corrected planned query into handoff discovery and pre-capture
verification; wire the actual query into expanded verification. Preserve stale
call-site/definition rejection and existing exit-preservation replay. Exercise
rejection after the actual contract acquires an extra entry input, with exact
restoration of the selected input, image, maps, relocations, costs and trace.

**Checks and exit:** the reproducer now requires A/X without spurious `$A0/$A1`
inputs and qualifies through the production handoff selector when profitable.
Cover genuine extra registers/fixed bytes/individual flags, partial and
conditional writes, identity capture, changed dependencies, source ZP ownership,
unknown callees and post-call uses. VM observations check arguments before the
callee executes capture, public results/access order and stack balance. Commit
the correction only after focused proof, integration and runtime tests pass.

## Slice 3 — qualify savings and preserve the existing work bound

Compare the frozen compiler with the new compiler under the **same handoff-enabled
configuration**. Disabling handoff is an additional control, not an isolated
measurement of this refinement. Retain inlining-disabled probes and normal
optimized callers; report when inlining removes a candidate.

Run both runtimes and origins `$3000`/`$41C7`, actual LF/CRLF fixture loading,
trace/untraced equality, and MADS reassembly/relocation. Repeat TN and independent
leaf/handoff controls. Require production savings in an ordinary Action-to-Action
control, no accepted routine growth in code plus ordinary homes, and preservation
of the existing Strcpy/Fnamecmp savings. Report unchanged TN output honestly if
this correction finds no additional TN opportunity.

Keep the current single handoff batch. A formerly empty candidate set may now
use its one existing continuation; this is distinct from adding another trial
after that batch. Cases still without candidates add no continuation. Existing
productive cases must not gain a second handoff continuation or repeat upstream
search. Record downstream emission and preservation work separately. Record
elapsed time with the build profile; do not turn this qualification into the
deferred compiler-performance project.

Run the full affected MIR6502 unit/integration scope and relevant VM consumers
of parameter capture, entry flags, argument forwarding, pointers, preservation
and terminal cleanup. Use the exact target inventory in the previous
[qualification record](benchmarks/6502-parameter-handoff/qualification.json),
extending it for new tests. Keep strict per-pass oracles isolated and combined
mode checks independent of incidental private scratch traffic. Reuse passing
checks unless subsequent behavior changes or failures require a focused rerun.

Publish receipts and exact commands/results. Update the design status and any
changed MIR capture/provenance contract; leave other planned extensions clearly
separate. Commit qualification and documentation after the backend checks pass.

## Validation discipline and completion criteria

During development, use filtered library tests and the existing handoff targets;
broaden to their consumers as each slice changes behavior. Normalize host text
before newline-sensitive instrumentation and exercise LF and CRLF through the
actual loading path. Preserve binary/ATASCII bytes. Record actual test names and
counts, including skips and focused reruns.

This is MIR6502 work. No full repository/NIR sweep is required for documentation
or backend-only changes; if implementation changes shared NIR/frontend contracts,
apply AGENTS.md's shared-contract checks and affected backend coverage.

Completion requires one capture authority, explicit planned/actual provenance,
correct incoming requirements, stale-fact rejection, actual expanded validation,
independently checked execution and measured final savings in a general control.
The previous whole-artifact fallback and one-batch limit must hold. No TN saving
or newly supported signature may be claimed solely from a forced binding or a
more permissive admission predicate.
