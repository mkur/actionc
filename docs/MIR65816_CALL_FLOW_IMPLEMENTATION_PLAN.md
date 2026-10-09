# MIR65816 argument and result flow implementation plan

Status: **stages 0–6 implemented and qualified**.

Qualified baseline: [stage-0 scorecard](benchmarks/65816-call-flow-stage0/README.md).
Behavior-neutral route foundation: [stage 1](benchmarks/65816-call-flow-stage1/README.md).
Native output ownership and atomic trial allocation: [stage 2](benchmarks/65816-call-flow-stage2/README.md).
Zero-test consumers: [stage 3](benchmarks/65816-call-flow-stage3/README.md).
Final private destinations: [stage 4](benchmarks/65816-call-flow-stage4/README.md).
Bounded private inputs: [stage 5](benchmarks/65816-call-flow-stage5/README.md).
Integrated qualification, compiler-cost acceptance and remaining application
gaps: [stage 6](benchmarks/65816-call-flow-stage6/README.md).

The final compiler saves 4,137 release code bytes against stage 0, with no
routine code, frame, spill or local-peak growth. All three serial compiler
time/RSS medians meet their review limits. The carried release-size,
representative-size, private-traffic and unchecked-provider targets remain open.

Implement the [call-flow design](MIR65816_CALL_FLOW_DESIGN.md) through the common
MIR65816 placement and emission contracts. Deliver the three investigated
consumer slices after establishing native result ownership. Exec816 remains
the primary workload; independent native programs establish correctness.

The design note owns contracts and invariants. This plan owns delivery order,
implementation boundaries, validation and completion evidence. No public ABI,
image format, NIR executable form or bank-zero reservation change is planned.

## Scope and acceptance

Initial result consumers use resolved Direct, Helper and Runtime targets with
verified native contracts. Bounded argument borrowing terminates at Direct
calls. Keep indirect calls, exposed destinations, cross-block result lifetimes,
wide register arguments and cast chains on their existing strategies.

Success requires qualified ownership without fictitious homes, measured benefits
from each new consumer family, and a net reduction in complete frozen Exec816
release code. Capture/read instruction totals in the
[investigation](benchmarks/65816-exec-call-audit/argument-result-flow/README.md)
are candidate footprints, not a savings commitment: 692 scalar zero tests,
287 private-local assignments and 121 bounded private scalar argument reads.
Compiler admission may accept fewer sites; explain refusals rather than widening
the proof to match the screen.

Preserve exact source-visible accesses, argument bytes and padding, result
lanes, native execution state, reentrancy and interruption behavior. No routine
may increase its frame, spill extent or local peak relative to the implementation
baseline. Retain the existing native vector regression limits and bank-zero
reservations. Report code growth in individual routines and its cause.

Carry forward the [stage-7 acceptance gates](benchmarks/65816-record-placement-stage7/README.md#frozen-final-acceptance),
including their open size, private-traffic and unchecked hosted-provider gates.
Completing this plan does not by itself close those gates or establish the
256 KiB application objective. Review compiler cost with the same 5% time and
10% RSS median limits and a consistent before/after environment.

## Delivery sequence

Each stage is an independently reviewable change with its focused checks,
contract updates and compact evidence. Run stages in order; do not defer proof
or runtime coverage to final integration.

| Stage | Deliverable | Exit condition |
| ---: | --- | --- |
| 0 | Frozen baseline and independent call-flow fixtures | Existing output reproduced; semantic and measurement oracles ready |
| 1 | Common result declaration and route validation | Behavior-neutral refactor; all three frozen images unchanged |
| 2 | Checked native outputs and immediate Return migration | Sole eligible Return results use common ownership without a required temp home |
| 3 | Native byte/word zero-test consumers | Eligible comparisons consume the returned lanes; flags are established explicitly |
| 4 | Native results assigned to private locals | Required writes remain at Store; intermediate captures and reloads disappear |
| 5 | Bounded private scalar arguments | Eligible byte/word loads borrow owned storage through complete multi-argument construction |
| 6 | Integrated backend and application qualification | Correctness, resource, footprint and compiler-cost evidence published |

Stages 0–3 establish the foundation and first investigated consumer slice.
Stages 4 and 5 deliver the remaining two slices. Stage 6 qualifies their
interaction and reports the remaining application gaps.

## Stage 0: baseline and independent fixtures

Reuse the existing frozen Exec816 inputs (`57df0d7`, 256 source files and 1,156
routines) and the post-stage-7 implementation baseline `af7c504c`. The
[call audit](benchmarks/65816-exec-call-audit/README.md) already records all three
profiles; its images match stage 7. Verify their source, tool and artifact
provenance before using them. Do not substitute the live Exec816 checkout.

1. Freeze compiler/runtime/fixture/tool hashes, Rust 1.99 build settings and
   candidate/reference binaries. Reproduce optimized release, optimized guarded
   and raw guarded images and the existing native vectors. Preserve the original
   stage-0 and stage-7 evidence.
2. Add independent call-flow fixtures using the existing assembly-import and
   qualified native VM harnesses. Cover result widths 1–4, zero tests, local
   writes, immediate returns and private inputs among mixed-width arguments.
   Fixtures must pass with today's conservative strategies before asserting new
   ownership or omitted homes.
3. Establish value, exact-access, padding, canary, native-state and stack oracles.
   Poison only ABI-unspecified lanes and flags; the defined byte/pointer zero
   extensions remain valid. Include negative admission shapes in typed MIR.
4. Extend observation tooling with a separate candidate collection/report path
   and explicit baseline lineage. Existing audit collectors require frozen
   image identity and must keep that guarantee. Candidate output may differ;
   authenticate its own hashes and record real routes, captures and reads.
5. Record fresh baseline compiler time/RSS with serial builds and pinned images.
   Reuse build caches and keep bulky outputs under ignored target directories;
   publish compact manifests and reports.

Validation: existing audit and flow `report --check`, their focused Python
tests, affected native fixture targets and frozen-profile identity. Verify LF
and CRLF through any new newline-sensitive parsing or instrumentation path.

Publish a stage-0 scorecard under `docs/benchmarks/65816-call-flow-stage0/` with
commands, identities, baseline measurements and the acceptance policy above.
The qualified scorecard records image identity, independent ABI checks and the
Rust 1.99 host baseline.

## Stage 1: separate declaration and destination validation

Start in [call preflight/copies](../src/mir65816/emit/call_copies.rs),
[call selection](../src/mir65816/emit/select.rs),
[home demand](../src/mir65816/emit/home_demand.rs) and
[placement](../src/mir65816/emit/placement.rs).

- Introduce checked native result facts independent of `self.temp(id)`: Call
  point, temp/width, target/signature contract and native lane convention.
  Validate an allocated destination separately when a route needs one.
- Make capture, discard and existing immediate Return decisions explicit under
  the common plan. Preserve current allocation, reserved Return homes, cleanup,
  captures, instructions and source spans in this stage.
- Bind route facts to the immutable routine and complete logical use census.
  Retain exact native contract, argument and physical-home preflight; malformed
  declarations remain errors even when capture is omitted.
- Provide the common logical-admission, trial-allocation and final-validation
  path needed by later stages. A valid rejected trial restores the complete
  conservative plan before allocation is committed. A broken admitted/sealed
  plan must fail rather than trigger a silent retry.

Validation: call/copy/Return, placement and replay unit coverage, affected root
65816 contract/emission tests, and native `call_copies`, `call_returns` and
`replay` targets. Test mismatched widths/conventions, discarded results and
invalid homes. Rebuild the three frozen profiles and require byte identity;
measure analysis overhead without claiming code savings.

## Stage 2: native output ownership and immediate Return

Extend common placement/resource validation and the
[tracked state](../src/mir65816/emit/state.rs),
[typed call effects](../src/mir65816/emit/effects.rs) and
[replay](../src/mir65816/emit/replay.rs) boundary as required. Migrate
[immediate returns](../src/mir65816/emit/call_returns.rs) as the first consumer.

- Model callee-return lanes as a new output origin and independently check their
  preservation through outgoing cleanup. Publish following-operation read
  permission only at body S with zero transient displacement.
- Keep the Call's incoming register/flag/scratch barrier. Whole-operation masks
  do not authorize output residence; replay derives permission from the native
  contract and actual cleanup actions. No useful callee flags are assumed.
- Admit sole adjacent Return uses at widths 1–4 through the common capture and
  read-location model, preserving current target and convention restrictions.
  Include the terminator occurrence in the ownership proof.
- Omit the reserved result home only after complete use, lane, teardown and
  trial-allocation checks. Preserve existing whole-routine terminal forwarding
  under its separate qualified contract.
- Keep allocation maps truthful, propagate generation ownership through selected
  rewrites, and reconcile final source spans and layout. There must be no hidden
  temp-home lookup in native result or Return preflight.

Validation: absent-home, hidden-use, stale-owner and wrong-cleanup rejection;
native `call_returns`, `home_demand`, `replay` and affected stack-allocation
targets. Exercise full A/X values, defined zero extensions, nonzero outgoing
areas/frames, recursion, relocation and interruption through cleanup/teardown.
Show that removal of a reserved home is distinct from the already implemented
omission of Return capture/reload instructions. Record frozen-profile resource
and code deltas; no frame/spill/local-peak growth is permitted.

## Stage 3: scalar result zero tests

Extend the common native-output producer admission and the existing comparison
consumer. Admit same-block, sole adjacent `Eq`/`Ne` uses against literal zero
for one- or two-byte results. Include branch and materialized Boolean paths.

The comparison reads the returned lanes and ends that result interval. Emit a
fresh comparison at the declared width; cleanup flags are not a shortcut.
The Boolean result retains its own normal ownership. Unsupported comparisons,
multiple/unreachable/address/edge uses, intervening operations and indirect
calls retain complete capture/read fallback.

Validation: home-demand, comparison, branch, placement and replay unit tests;
native zero/nonzero extremes with poisoned flags and unspecified registers in
all required modes. Check admitted results have no canonical temp home and no
physical capture/read, while comparisons and Boolean values remain correct.
Exercise relocation and interruption at the native-output/consumer boundary.

Publish admitted/refused zero-test counts and complete code/traffic deltas
against stage 2 and the frozen implementation baseline. Account for the fresh
comparison and mode changes; do not score removed stores alone.

## Stage 4: final private local destinations

Extend the common result route and the Store consumer using existing
[local destination](../src/mir65816/emit/local_loads.rs) and
[address-consumer](../src/mir65816/emit/address_consumers.rs) infrastructure where
their contracts apply. Recompute admission rather than reusing a looser
generic frame-store predicate.

- Admit a sole adjacent payload Store into an unindexed, nonvolatile, unexposed
  mutable automatic local. Result and Store widths must match at 1–4 bytes;
  the resolved destination range must fit its owned object. Parameter-backed
  frames and address uses of the result remain excluded.
- Bridge the native lanes through cleanup to Store. Perform the semantic write
  in Store's source span, not the Call span. Check address preparation, full
  lane lifetime and exact writes, including the three-byte tail.
- Omit the intermediate temp home atomically. Retain the final object's real
  map, ordinary later storage reads and conservative call effects.

Validation: placement/Store/capture/replay tests and affected native local,
memory, home-demand and call-flow fixtures. Use neighboring canaries, nonzero
object displacements, bank-valued pointers and full 32-bit payloads. Reject
escaped, volatile, indexed, indirect, static and parameter-backed destinations,
hidden uses and stale object identities. Exercise relocation and interruption
between Call and Store. Verify source attribution after final layout.

Publish accepted/refused destination counts and incremental/total application
and native deltas. A retained final local write is never counted as removable.

## Stage 5: bounded private byte/word arguments

Extend [scalar borrowing](../src/mir65816/emit/scalar_forwarding.rs) through the
common input-owner and read-binding model. Integrate it with
[argument preflight](../src/mir65816/emit/call_copies.rs) and
[complete construction](../src/mir65816/emit/call_pushes.rs).

- Admit full canonical one/two-byte reads from immutable incoming parameters
  or unexposed locals, each with one terminal Direct-call argument occurrence.
  Keep the source Load's logical identity even when its physical span is empty.
- Reuse complete ownership/escape facts and prove unchanged storage through
  the same-block interval. Reject mixed/partial/volatile/copy views,
  parameter-backed frame aliases and intervening ordering barriers. Preserve
  existing four-byte scalar and pointer admissions.
- Validate all admitted bindings and ordinary operands in one complete call
  schedule before omitting homes. Check actual partial S displacements, every
  source/destination byte, padding, guard ordering, transfer peak and cleanup
  for both supported construction strategies.
- End borrowed permission before callee execution. A used call result must
  retain its independent output owner; it inherits no input binding.
- Keep the current bounded accumulator-to-Y argument rule unchanged. This
  slice reads private homes and does not admit general register arguments in
  multi-argument calls or wide A/X result-to-call chains.

Validation: scalar-borrowing, call-copy/push, allocation, placement and replay
tests; native `scalar_forwarding`, `call_pushes`, `call_copies`, `call_padding`,
`terminal_pointer_calls` and affected call-flow fixtures. Cover several borrowed
inputs mixed with constants, symbols and wide captured values, repeated reads
of one private object, nested-call rejection, range boundaries and atomic
fallback. Verify argument bytes/padding independently and suspend construction
at changed instruction boundaries with task/IRQ/NMI contexts.

Publish actual admitted argument bindings and complete deltas against stage 4
and the implementation baseline. Keep storage-read deferral evidence separate
from captured-value forwarding and wide-argument estimates.

## Stage 6: integrated qualification and final report

1. Run the full affected 65816 unit, root integration and qualified native suites,
   including opt-in comparison vectors. Cover raw/optimized, guarded/release,
   supported relocation, exact access extent and task/IRQ/NMI execution. Check
   artifact consumers and both LF/CRLF paths affected by fixture/tool changes.
2. Rebuild the three frozen Exec816 profiles and their native vectors. Bind each
   measurement to candidate binary, source, tool and artifact hashes. Report
   native correctness/traffic/cycles, complete code and initialized-data costs,
   per-routine frame/spill/local peaks, and every unexplained growth or refusal.
3. Re-run the affected hosted fixtures against the pinned compiler/runtime and
   retain known failures with their diagnoses. Score new failures explicitly.
   Keep unchecked provider refusal and hardware qualification limits visible;
   a guarded package does not establish unchecked release behavior.
4. Measure serial compiler wall time and peak RSS after qualification finishes,
   with warm-up, alternating profile order and all samples retained. Compare to
   the stage-0 implementation baseline using the same Rust/build environment.
   For cumulative review against `d7d536c9`, rebuild that baseline in the same
   environment; historical timing from another toolchain is not interchangeable.
5. Publish compact stage evidence and final acceptance under
   `docs/benchmarks/65816-call-flow-stage6/`. Update the design, placement and
   emission documents to distinguish qualified forms from deferred ones.
   Record remaining stage-7 and application gaps without resetting targets.

Every functional stage must demonstrate its own admitted cases and measurable
benefit; final integration must retain a net release-code benefit and all
correctness/resource gates. If a form fails cost or qualification, keep its
conservative route and report the incomplete obligation. Do not mark a stage
complete from structural opportunity counts.

## Validation entry points

During development select unit filters and root/native targets from the changed
code and actual consumers. Existing native targets can be batched through the
[qualified runner](../tools/native65816-runtime-tests/qualify.py), for example:

```sh
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test call_returns --test home_demand --test replay -j2
python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_call_*.py'
```

Final backend checks include:

```sh
cargo test --locked --features native65816-state-proof --lib mir65816::
cargo test --locked --features native65816-state-proof \
  --test mir65816_abi --test mir65816_contract --test mir65816_emission \
  --test mir65816_state_boundary --test mir65816_logical_analysis \
  --test mir65816_address_selection --test mir65816_arithmetic \
  --test mir65816_register_inventory --test mir65816_copy_inventory \
  --test mir65816_movement_inventory --test mir65816_scalar_dp_inventory \
  --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -B tools/native65816-runtime-tests/qualify.py -j2
```

Run ignored native comparison vectors with the pinned candidate/reference
manifest and results environment, as described in
[stage-7 reproduction](benchmarks/65816-record-placement-stage7/README.md#reproduction-and-evidence).
Candidate collection commands must be documented when their stage-0 path is
implemented; existing frozen audit checks are not candidate publishers.

Backend-only work does not require executing other backends' suites. If a
missing fact requires changing shared NIR/lowering/verifier/printer contracts,
document that boundary change and run the required shared checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Keep full CI coverage. Re-run passing suites only when later changes or failures
affect them; report the scope actually checked with each delivered stage.
