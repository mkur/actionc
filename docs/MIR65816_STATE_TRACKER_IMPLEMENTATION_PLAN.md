# Native 65816 state-tracker implementation plan

Status: proposed on 2026-09-21 against main `63069a6`, following the
[design note](MIR65816_STATE_TRACKER_DESIGN.md). This plan implements its first
two stages: the state model and a byte-identical integration. Broader local
forwarding, block-entry width optimization and X/Y/DP optimization require
separate measured plans after this foundation is qualified.

## Deliverable and fixed scope

Introduce a private `State65816` and `TrackedEmitter65816` under
`src/mir65816/emit/`. Every production instruction emitted by native selection
must pass through the facade and have an explicit state effect. Replace the
current separate width cache and resident-word facts with this state owner.
Preserve the existing adjacent-word eligibility policy and every emitted byte.

Keep MIR/NIR semantics, register allocation, source accesses, temporary stores,
frame/staging reservations, stack guards, fault state, interrupt headroom,
physical ABI v1, image v3, o65 and Exec816's pin unchanged. No new opcode
selection, load omission, flag-dead exception, mode omission, short branch,
fallthrough, DP home or register lifetime is enabled by this plan. Existing
valid inputs and malformed-input/fallback diagnostics retain their behavior.

Use the 6502 implementation as a reference, without editing or generalizing it:

- [TrackedEmitter](../src/codegen/tracked_emitter.rs): couple bytes and effects,
  explicit barriers, snapshot tests, and consumer-specific proof queries.
- [NativeProcessorState](../src/codegen/native_state.rs): register/flag/memory
  separation and unknown/alias/dependency invalidation tests. Its byte-sized
  values and flat addresses are not suitable native representations.
- [MIR6502 emission](../src/mir6502/emit.rs): verified typed strategy enters the
  facade; source semantics and allocation stay outside it.

## Rechecked baseline and equality gate

Planning verified the existing [forwarding qualification](abi/action65816-accumulator-forwarding-qualification.json):
25 changed code/test hashes, 41 benchmark-file hashes, all 407 compiler/fixture
inputs and both sets of 302 saved native artifacts. All 224 file hashes across
56 comparison builds match; the debug/release reports contain the same 264
records, representing 528 executions per host. These are checks of existing
evidence, not a new test run.

The [baseline record](benchmarks/65816-state-tracker/baseline.json) freezes this
evidence. The immutable comparison directory is
`target/local-accumulator-forwarding-after`, qualified at `956facf`; its emitter
is `d946c33`. Reconstruct missing artifacts using that historical checkout and
runner. Do not rebuild the baseline with the new tracker or overwrite snapshots.

All fields below must remain exactly equal, alongside every other corpus record:

| Kernel / input | Mode | Bytes | VM cycles | Stack reads / writes | Stack peak | Forwarded loads |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| identity(13) | raw / optimized | 61 | 66 | 5 / 2 | 4 | 1 |
| sum_loop(13) | raw | 164 | 1,767 | 197 / 246 | 14 | 53 |
| sum_loop(13) | optimized | 140 | 1,395 | 165 / 188 | 16 | 40 |
| byte_sum($12FFFC,16) | raw | 262 | 4,678 | 530 / 559 | 16 | 65 |
| byte_sum($12FFFC,16) | optimized | 238 | 4,231 | 492 / 489 | 22 | 49 |

Retain all 76 current forwarding sites and 1,422 executions per incoming I
state. Retain existing fusion, word-edge and direct-edge counts and PCs. The
optimized vbcc unlink vector-0 failure must remain visible in both I states
and both host runs; it is not an expected-success exemption.

## Concrete state and facade

Add `state.rs`, `tracked.rs` and their focused unit-test modules; keep
`allocation.rs`, `liveness.rs` and the 6502 backend unchanged. `Builder` owns the
facade. The facade owns mutable encoding storage and `State65816`, exposes
read-only positions/bytes/fixups for checks, and yields the existing public
`Code`/`MachineRoutine` on completion. Final linking and relocation may still
patch their finalized buffers. Selection must have no mutable `Code` escape,
`DerefMut`, or arbitrary opcode-plus-caller-claimed-effect interface.

Use a private exhaustive instruction/addressing representation or typed methods
whose implementation selects both encoding and effect. Proposed operations
include checked load/store, ADC/SBC/logic, compare, carry, transfer, mode request,
stack operation, branch/jump, call and label binding. Parameterize concrete
address forms with existing `Memory`, `Target`, offsets and checked extents;
do not reconstruct storage classes from linked numeric addresses.

State requirements:

| Fact | First implementation |
| --- | --- |
| Values | Width-qualified constants and fresh opaque `ValueId`s. Copies share an immutable token; unknowns never compare equal. Bound stored facts by registers and allocated homes. |
| Registers | Full A/X/Y facts with explicit widths and conservative lane invalidation. A8 writes/width changes cannot recreate a full-word proof. X/Y facts remain observational. |
| Flags | N/Z tied to a value and width; separate known/unknown C/V. Model actual flag writes, including ones that preserve A. No new arithmetic folding or flag-based omission. |
| Machine environment | Known/unknown E/M/index width, decimal state, DBR and symbolic current-domain D; preserved I token or declared IRQ effect. Seed only from checked entry/call/sequence contracts. |
| Memory | Exact private stack ranges, contents generations and bindings to TempId. Overlapping writes invalidate home equality. Captured register tokens do not become references to mutable source memory. |
| Stack | Checked invocation anchor, outgoing displacement and explicit transfer phase. Account separately for temporary pushes and peak use. All stack movement still clears forwarding eligibility. |
| Optimization permission | Explicit local mode-omission and adjacent-word witnesses preserving current policy; richer observations alone never authorize new code changes. |

Current DP and unknown memory writes must invalidate overlapping or possibly
aliased facts even though DP-content reuse is deferred. Do not cache source
globals, addressable locals, parameters, indirect memory or volatile accesses.
Calls clear value/flag/memory relations and all 64 scratch bytes. Normal return
restores only the declared boundary guarantees and fresh result lanes; results
remain outside the current forwarding producer whitelist.

## Mode and control-flow contracts without new optimizations

There are two different questions: what width an instruction is required to
execute in, and whether today's selector may omit a mode-setting instruction.
Resolve that distinction explicitly during integration:

1. `State65816` is the sole owner of machine-mode facts. Entry/call contracts
   come from existing `Mir65816ModeState` and `abi::BoundaryContract` facts.
   Selected closed sequences declare and check their own local label contracts.
2. Keep a local mode-omission permission associated with an explicit mode
   request and its region. Binding any existing label revokes that permission
   exactly as `Code::mark` does today. A known ABI/sequence width by itself
   does not grant permission. Do not retain a second independent width cache.
3. `ensure_a8`/`ensure_a16` emit or omit the same REP/SEP as the old helpers.
   A genuine emitted instruction updates facts and the byte cursor; a no-op
   request leaves both unchanged. Status masks apply their complete effects.
4. Check required width against proven execution contracts before choosing
   immediate length or access width. Encoding a word immediate never establishes
   A16. Unknown preconditions need a checked sequence contract or a diagnostic;
   do not silently insert normalization that breaks the equality gate.

If an existing valid path lacks a tracker proof, fix the model or sequence
contract rather than weakening checks or rejecting that path. A discovered
emitter correctness bug needs a separately explained and qualified fix; do not
hide changed behavior inside this unchanged-output refactor.

Audit routine entry, guard labels, signed byte comparison/cast arms, pointer
arithmetic, shift/copy loops, fused edges, return teardown and the indirect-call
resume label. Internal branches can have byte-mode or word-mode postconditions;
a blanket native A16 assertion at every label is invalid.

Check contracts against all actual edges of each selected closed sequence,
including its local loop backedges. Keep only execution preconditions at those
joins, not register-value optimization facts. The existing requirement that
MIR transfers restore A16 must be asserted at every emitted exit before it can
justify a MIR-entry execution contract. This is validation of current emission,
not the later CFG pass that removes redundant width setup.

`Code::branch` currently writes an inverse short branch over one JML. Preserve
its bytes and labels/fixups exactly. Model it as a conditional transfer with a
fallthrough continuation, not an unconditional JML followed by unreachable
code. Do not allocate extra output labels merely to track the implicit skip.

## Stack, calls and raw fault paths

Replace direct assignments to `Builder.delta` with checked sequence operations.
The stack model must distinguish body-relative addressing from physical
transfer state; today's `delta = outgoing` does not count the indirect stub's
PHK/PER/PHA pushes. Do not force both concepts into one unchecked counter.

| Existing sequence | Contract to validate, with identical bytes |
| --- | --- |
| `check_stack(F)` and entry TCS | Normal path computes the checked S change before TCS; establish the body anchor only after it executes. Fault path retains exact required A/X/S and performs no prohibited write. |
| Outgoing reservation | Apply checked reservation, expose the same increased frame displacements, preserve padding and argument writes. |
| Direct JSL | Distinguish the three-byte transfer peak from net S on normal return. No pre-call value or flags survive. |
| Indirect transfer | Capture target before pushes; account for PHK/PER and one-/two-byte PHA, six-byte peak, RTL into callee, and the ordinary three-byte return at resume. |
| Cleanup and epilogue | Validate the existing TAY/TSC/CLC/ADC/TCS/TYA sequence and A/X result preservation; restore the expected stack anchor and delta. |

A call's return postcondition applies at its continuation, not when execution
enters the callee. Treat the synthetic RTL used for an indirect call separately
from a routine return. Keep typed PER continuation fixups, bank checks and
low-word wrap behavior unchanged.

TSC/TCS and other special transfers need their instruction-specific effects.
Where a concrete stack equation is needed, use a bounded stack-address token
or a sealed, verified sequence summary, not general symbolic expressions or
unchecked `assume_stack`/`assume_mode` calls from selection. Sequence summaries
cannot hide unmodeled instructions or contradictory mode/stack effects.

## Instruction inventory and migration order

Freeze an inventory before moving the encoder boundary. Start from the actual
calls in `select.rs`, `accumulator.rs` and `code.rs`; include dynamically chosen
opcodes and methods such as `memory`, `binary` and `pointer_step`. A grep of
literal opcodes alone is insufficient. Give every admitted form an encoding,
width precondition, register/flag effect, memory effect, stack effect and proof
test. Unknown forms fail closed.

Migrate these groups without changing their selected instruction sequences:

| Group | Existing coverage to retain |
| --- | --- |
| Immediate/stack/DP/long/symbolic/indirect loads and stores | Byte/word widths, volatile byte order, banked data, pointer aliases and relocations. |
| ADC/SBC, logic and compares | Word operations, byte carry chains, signed fallbacks, N/Z/C/V, fused and materialized predicates. |
| REP/SEP and transfers | A8/A16 transitions, X/Y use, hidden accumulator byte, XBA and result marshalling. |
| DP read-modify-write and index updates | ASL/ROL/LSR/ROR, DEX, pointer indexing, shift/copy loops and scratch overlap. |
| Guards, S operations and pushes | Entry checks, outgoing reservations, direct/indirect transfer peaks, cleanup and raw faults. |
| Branches, JML/JSL/PER/RTL and labels | Conditional skip sequences, calls versus returns, joins, backedges and fixups. |

All forms currently emitted require explicit handling. Unsupported instruction
families such as arbitrary status restores or E changes may remain barriers
with unknown facts or existing diagnostics; do not add source support for them.
For raw executable bytes, invalidate all possibly affected facts and require
a qualified continuation contract. A known but conservatively modeled form
must enumerate what it preserves; no default “probably keeps modes” effect.

## Existing forwarding policy must remain exact

Move `ResidentWord` into the state-owned proof, retaining its TempId, exact
home/range, contents identity, zero transient displacement and byte/label
cursor. Publish it only after the existing eligible direct Load or native
ADD/SUB and its retained private STA. The modeled instruction history must
already prove A16 and matching N/Z; publication cannot invent those facts.

Keep the four current consumers, their complete preflight and operand order,
including comparison swapping. A consumer must satisfy both the state proof
and the original adjacency witness. Even modeled CLC/SEC, NOP or a disjoint
store between producer and consumer still prevents forwarding in this plan.
Preserve explicit zero-byte barriers, all edge barriers, fallback/error behavior
and the current one-use eligibility lifecycle. Observation may retain facts
after a consumed proof; selection may not reuse that permission.

During migration, retain the old implementation only as a temporary shadow
oracle with assertions that decisions agree. Remove it before completion;
the final compiler must not maintain two authoritative trackers or a permanent
legacy-mode switch. Keep the test-only adjacent-span index independent and
unchanged in meaning. No broadened forwarding metric is needed here.

## Independent state validation

Unit snapshots alone can repeat an incorrect effect table. Add a native
`state_tracking` target that checks observations against independent VM
execution and hand-authored ca65 sequences, using the qualified runner.

The runtime workspace compiles actionc as a dependency, so root `cfg(test)`
helpers are not visible there. Provide an explicit opt-in
`native65816-state-proof` Cargo feature, default off, enabled only by the
isolated qualification workspace. A small `emit::proof` API may expose
immutable snapshots, checked probe cases and a materialization-with-trace
entry point. Keep mutable state/encoder access private. Ordinary compilation
does not collect per-instruction snapshots; trace enablement must not affect
selection, labels, fixups, allocation or serialized output.

Snapshots identify routine-relative instruction boundaries, exact widths,
observable value/flag relations, canonical home ranges and sequence events.
Normal-return events are distinct from transfer events. New tests validate
the actual linked/relocated bytes before checking reached observations; no
compiler trace participates in CPU execution or defines expected program results.
Compare known constants and simultaneous register/home/flag relations against
the VM, rather than treating opaque ValueIds as globally constant across loop
iterations or invocations. Rebase symbolic evidence for o65 explicitly.

Use a bounded matrix:

- Port 6502 unknown-value, register-copy, dependent-memory, alias-chain,
  call/label/raw-byte and zero-byte barrier cases.
- Add zero/sign/carry/overflow word boundaries, flag-only clobbers, hidden B,
  index narrowing, mixed widths, complete status masks and special transfers.
- Include overlapping/reused/partial homes, DP scratch overlap, displacement
  254/255, outgoing stack movement, indirect call/resume and malformed preflight.
- Check feature-off/on image and o65 byte equality. Independent ca65 probes
  compare actual encodings, CPU effects and traces for the audited instruction
  families; emitter snapshots supply claims to check, never the VM oracle.
- Retain all existing IRQ/NMI, alias/volatile, helper, recursion, guard and
  relocation probes. No new register lifetime is introduced, so preserve their
  current semantic and PC coverage rather than replacing them with trace tests.

## Commit-sized sequence

1. **Freeze baseline and boundary probes.** Recheck the recorded hashes, inventory
   encoder writes/forms and capture code, labels, fixups, PER fixups and MIR spans
   for focused raw/optimized fixtures. Add a strict no-change corpus checker with
   mutation tests. No production changes. Commit once current output passes it.
2. **Implement state and typed facade.** Add private facts, effects, closed-sequence
   contracts and read-only test snapshots. Port 6502 scenarios and native cases;
   add the proof feature and independent instruction probes. The production
   selector still uses the original path. Commit after model/facade and native
   probe checks pass, with feature-off/on output unchanged.
3. **Route selection through tracked emission.** Migrate every instruction group,
   move width ownership and checked stack bookkeeping, and retain current mode
   omission permissions. Use a temporary resident-word shadow during this step.
   Keep all output and diagnostics equal. Commit after targeted compiler/native
   execution and strict corpus equality pass.
4. **Make tracked forwarding authoritative.** Bind private homes and N/Z to the
   actual emitted producers, require the original adjacency permission, migrate
   affected unit-test constructors, and delete duplicate state/legacy writers.
   Audit encoder privacy and all barrier paths. Commit after focused execution,
   exact forwarding counts and proof-metadata equality pass.
5. **Qualify and document.** Run final complete native debug/release suites,
   both external corpus hosts, o65 and interrupt coverage, and applicable CRLF
   checks. Save an unchanged-output report and source/artifact-bound qualification
   JSON; update the emission contract, design status and roadmap links. Commit
   only the completed task files, preserving unrelated local changes.

The selector cutover may be split further by instruction group if each commit
remains buildable and byte-identical. Temporary shadow checks belong only to
the migration. Completion requires the final single-owner boundary and its
qualified evidence, not merely an unused state model alongside the old emitter.

## Comparison tooling and acceptance commands

Add `tools/compare65816/check_state_tracker.py` and focused negative tests.
Reuse manifest loading/hash validation, but require **complete measurement
record equality**, including metric fields and per-PC maps. The default
`delta.py` only checks selected invariants and nonregression; that is insufficient
for this refactor. Do not use forwarding/read/write exceptions or allow improved
size/cycles to conceal an unintended optimization.

Require identical Action artifact bytes, routine addresses/sizes/maps and all
vbcc code/records. Normalize only the existing exact vasm `Source:` header path
when comparing its raw listing. Provenance manifests legitimately have new
compiler/tool hashes and paths; compare their structured contracts separately.
Reject missing/extra records or files, changed instructions/fixups, metric
changes, and any other normalization. Existing 302 native artifact hashes must
remain a matching subset; added state-proof artifacts must match between hosts.

Compiler checks after integration:

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract \
  --test mir65816_emission --test mir65816_o65 \
  --test actionc_65816_cli --test actionc_65816_o65_cli
```

The new target below is to be implemented. Use the qualified VM runner, never
bare cargo in the isolated native workspace:

```sh
python3 tools/native65816-runtime-tests/qualify.py \
  --test state_tracking --test accumulator_forwarding --test word_arithmetic \
  --test word_returns --test word_comparisons --test compare_branch \
  --test word_edges --test empty_edges
python3 tools/native65816-runtime-tests/qualify.py --test preemption --test o65 \
  --test stack_faults --test indirect --test interop --test memory
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

Final corpus preparation and reporting (the new checker CLI is planned):

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/corpus.py --check
python3 tools/compare65816/build.py --output target/state-tracker-after --verify-crlf
# Run both external code_quality host commands from tools/compare65816/README.md,
# using target/state-tracker-after for the manifest and debug/release reports.
# Run the second host even when the first reports the known vbcc failure.
python3 tools/compare65816/report.py --input target/state-tracker-after \
  --output docs/benchmarks/65816-state-tracker/after
python3 tools/compare65816/check_state_tracker.py \
  target/local-accumulator-forwarding-after target/state-tracker-after \
  --baseline docs/benchmarks/65816-state-tracker/baseline.json \
  --output docs/benchmarks/65816-state-tracker/equality.json
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/compare65816 -p 'test_*.py'
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools -p test_disassemble65816.py
```

At completion retain all 84 existing native tests and their semantic coverage,
plus the new state probes; record actual totals. Expect all existing measurements,
guard outcomes, call/IRQ/NMI behavior and serialized artifacts to remain equal.
Publish compiler/fixture/tool hashes, effect coverage, feature-off/on results,
both native manifests, corpus equality and the known external failure.

Format only changed Rust files with `rustfmt --edition 2024 --config skip_children=true`.
Normalize newline-insensitive host text and verify changed fixture paths with
LF and CRLF, using an isolated checkout for affected embedded fixtures when
needed. Scope other checks to changed consumers; this plan changes no NIR or
semantic contract and therefore does not require the full root/NIR sweep unless
implementation crosses that boundary. Documentation-only planning checks links,
content and preserved local files, not a new compiler qualification.
