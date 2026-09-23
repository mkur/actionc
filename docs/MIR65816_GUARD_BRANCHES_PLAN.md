# Native 65816 short branches inside stack guards

Status: completed on 2026-09-23. Baseline/accounting `74d7c660`, implementation
`dc8106be`, and [qualification/results](benchmarks/65816-guard-branches/README.md)
complete all three slices. Frozen Exec saves 37,248 raw / 37,120 optimized
executable bytes, exactly the direct forecast, with every guard retained.
See the [qualification manifest](abi/action65816-guard-branches-qualification.json).
The original plan follows. This follows the completed
[outgoing argument padding slice](benchmarks/65816-call-padding/README.md)
and selects the guard opportunity from the
[Exec size audit](benchmarks/65816-exec-size-detail/README.md).

## Objective and scope

Replace the four inverse-branch/JML conditional transfers inside each existing
stack guard with short conditional branches through the existing typed layout
finalizer. A guard's core encoding shrinks from **45 to 29 bytes**, saving
16 bytes. Apply this to raw and optimized native emission.

Preserve every guard, its amount, order and position before reservation,
argument stores or transfer pushes. Preserve ABI v1, image v3, o65, frame/home
allocation, outgoing extents, result behavior, interrupt headroom and bank-zero
reservations. Keep the local unconditional JML to the fault arm and the external
JML to the overflow adapter. No BRA selection, guard removal/hoisting, new
stack-check option, ABI change or general branch-eligibility expansion belongs
in this slice. Live Exec sources and its compiler pin remain unchanged.

## Current code and implementation choice

[`Builder::check_stack`](../src/mir65816/emit/select.rs) emits four calls to
`TrackedEmitter65816::branch`. This writes the conservative six-byte encoding
but does not register it for relaxation. By comparison,
[`TrackedEmitter65816::dispatch`](../src/mir65816/emit/tracked.rs) emits the same
branch and records `ConditionalBranch` metadata inside a replayable
`Request::Dispatch`.

[`layout::finalize`](../src/mir65816/emit/layout.rs) already starts with long
forms, shrinks eligible sites to a fixed point and validates their encodings.
Both raw and optimized materialization use it. It remaps labels, fixups, PER
operands, MIR spans/transfers, selected records and optional proof observations
together. Image and o65 placement validate bank containment afterward.

Use `dispatch` for **only the four branches in `check_stack`**. Its existing
request has no requirement to be a MIR terminator: selected reconciliation
records its local predicate/target separately from logical MIR transfers.
Retain `Instruction::Branch`, its effects, labels, joins and barriers. No new
instruction/request variant, guard registry, replay path or byte-patching pass
is needed. Clarify the existing API/contract wording to include these eligible
local guard branches without a broad rename.

The resulting guard has the same logical sequence:

```asm
        TSC
        TAX
        CMP dp_stack_ceiling
        BCC within
        BEQ within
        JML fault
within:
        SEC
        SBC #required_bytes
        BCC fault
        CMP dp_stack_floor
        BCS done
fault:
        LDA #required_bytes
        JML stack_overflow
done:
        ; existing reservation or continuation
```

The four compact relative operands are +6, +4, +4 and +7 for this unchanged
sequence. Derive them from labels through the finalizer; never hard-code them
in production selection. Each removes one owned 24-bit local fixup. The two
retained JML fixups still resolve normally. Preserve the existing out-of-range
fallback and reject malformed metadata before deleting any bytes.

## Baseline and forecast

The completed call-padding build is the baseline. Its compiler implementation
is `835f0cb0`; `183c7a5d` adds qualification/documentation and the reviewed
emission snapshot. Planning reinspection of retained final machine images
confirms 2,328 raw / 2,320 optimized guards, each matching the full 45-byte
sequence and local/fault targets.

| Measurement | Raw | Optimized |
| --- | ---: | ---: |
| Frozen Exec guards retained | 2,328 | 2,320 |
| Guard bytes before | 104,760 | 104,400 |
| Guard bytes forecast | 67,512 | 67,280 |
| Direct encoding saving | **37,248** | **37,120** |
| Compiler routine bytes before → forecast | 520,969 → 483,721 | 489,697 → 452,577 |
| All executable bytes before → forecast | 528,534 → 491,286 | 497,262 → 460,142 |
| XEX bytes before | 545,790 | 513,926 |
| Dijkstra code bytes before → forecast | 5,790 → 5,438 | 5,202 → 4,850 |

Dijkstra retains 22 guards, giving 352 bytes of direct saving in each mode.
The optimized Exec forecast is approximately 7.5% of current executable size.
These are forecasts, not implementation results. Guard shrinking may bring
already-eligible ordinary dispatches into short range; report those secondary
savings separately. Do not enable additional non-guard branch sites. Measure
XEX/o65 sizes and packing separately; fewer relocations also affect file size.

Keep Exec `c3500c8`, its eight-task shell/console/MyDOS configuration and the
existing checked-layout compatibility adapter frozen. Revalidate retained
input/artifact hashes before reuse. The adapter only asserts and removes
`stack_checks:true` for main, which always emits guards; it does not disable
checks. Runtime bank-zero budgets remain 24,672 bytes excluding OS and 61,536
including OS. Added reservation must be zero.

## Preserved behavior and proof obligations

The guard accepts exactly `S <= ceiling`, subtraction without bank-zero
underflow, and `S - required_bytes >= floor`. Equality at ceiling and floor is
accepted. Entry checks use the allocated frame, including zero-frame checks;
call checks use `O + 3` for direct transfers and `O + 6` for indirect transfers.

At success, A contains `S - required_bytes`, X contains the original S, Y and
S are unchanged, and arithmetic flags come from the same operations as before.
At failure, control reaches the same nonreturning adapter with the amount in A,
the original S in X and S unchanged. Preserve full status on corresponding
success/fault exits, including path-dependent V/C, and preserve native widths,
I, decimal-clear state, D and DBR. Branches must introduce no data reads/writes,
mode changes or helpers; ceiling/floor reads keep their original order and width.

Instruction addresses, counts and cycles change. Compare architectural state
at corresponding logical boundaries, rather than equating PCs of differently
sized code. IRQ/NMI may arrive between a comparison and its branch. Restore the
full interrupted state and execution-domain DP before resumption, retaining
the same success/fault decision and stack protection.

## Implementation commits

### 0. Capture guard behavior and make accounting accept both encodings

Add a focused `guard_branches` native execution target using the existing VM,
assembler and bus-trace harnesses. Execute current emitted guards and independent
assembly reference sequences with explicit inputs. Record success/fault state,
data-access traces, cycles, amounts and sizes before selection changes. Retain
authenticated call-padding corpus/Dijkstra/Exec baselines instead of rebuilding
unrelated old compiler revisions.

Update [`tools/compare65816/build.py`](../tools/compare65816/build.py): its
`check_ranges` currently recognizes only the 45-byte form, and corpus accounting
uses `45 * guard_count`. Recognize both complete 45-byte and 29-byte sequences,
validating predicates, local destinations, repeated reservation immediate and
the configured fault target. Pass the image fault address explicitly where
needed. Sum actual range lengths for `static_stack_check_bytes`. Dijkstra
imports this recognizer and must retain complete guard-cycle attribution.

Add focused decoder tests (`tools/compare65816/test_guard_ranges.py`) for both
forms at moved origins/fault addresses, mixed sequences and mutations of branch
offsets, JML targets, amounts, truncation and overlapping candidates. Unknown
or malformed guards must not silently reduce reported overhead: cross-check
entry plus typed call/guard inventories when building new evidence. This is
measurement code only; the compiler must not recognize guards from byte patterns.
Keep legacy recognition so existing artifacts remain readable.

Commit the baseline/tests/accounting after their focused checks pass, with
production machine bytes unchanged.

### 1. Admit the four branches and validate the complete vertical slice

Change the four calls in `check_stack` to the existing `dispatch` API. Add
focused selector/layout tests for four eligible sites per guard, a 16-byte
reduction, retained unconditional/fault JMLs, identical amounts and unchanged
non-control instructions. Keep zero-amount guards. Check deterministic replay
and finalization, selected CFG/effects, and proof-enabled/ordinary output equality.

Reuse existing layout boundary/cascade/metadata rejection tests. Add guard
coverage for multiple guards with ordinary short/long dispatches, retained
external fixups, data fixups and indirect PER continuations after shrinking.
Exercise same-bank placement near a bank end and invalid wrap/cross-bank cases;
preserve the existing signed displacement limits and long fallback.

Expand the runtime boundary matrix in both compiler modes and incoming I states:

- S below, equal to and above ceiling; candidate S below, equal to and above
  floor; subtraction underflow; zero and nonzero frame amounts; direct and
  indirect call amounts. Include representative arithmetic sign/overflow cases
  and multiple seeded input flag values under the valid native ABI environment.
- Compare the old reference and new emitted guard at success or adapter entry,
  checking full A/X/Y/S/P/D/DBR and ordered DP reads. PC is matched by logical
  destination. Require no stores or transfer pushes before a failed guard.
- Execute complete direct/indirect calls and returns, preserving argument
  padding, payload writes, PER continuation, cleanup and result capture.
- Run success and failure through serialized o65 at both existing placements,
  moving the fault adapter as well as code/data. Check surviving relocations and
  execute calls whose continuations follow shortened guards.

Reuse [`preemption.rs`](../tools/native65816-runtime-tests/tests/preemption.rs)
and the existing context bridge for IRQ/NMI at every reached instruction boundary
of representative entry/direct/indirect guards in both task domains. Include
ceiling equality, floor equality and an injected fault path with valid physical
interrupt headroom. Verify immediate full-state restoration and final decision;
separate interrupt-frame writes from forbidden guard/application writes. True
invalid-S/underflow cases are synchronous fault tests, not unsafe interrupt
stack setups. IRQ delivery obeys I; cover NMI and retained I when IRQ is masked.

Review consumers of `Code::conditional_branches`: its population will now include
guards. In particular, the whole-routine branch count/index assertions in
[`control_flow.rs`](../tools/native65816-runtime-tests/tests/control_flow.rs)
must select their intended
MIR dispatch by source span/target. Do not weaken them to accept any short branch.
Review replay/state tests and exact size/cycle budgets similarly. Refresh the
state-boundary fixture only after proving intended guard shrinkage, any secondary
existing dispatch relaxation and metadata rebasing; frames/homes remain equal.

Update the [emission contract](MIR65816_EMISSION_CONTRACT.md), including its
statement that guard branches keep long encodings, and the
[selected-action contract](MIR65816_SELECTED_ACTIONS.md) where eligibility is
described. Retain padding-only call initialization in the stack-check description.
Commit selection, affected tests and contracts together after focused validation.

### 2. Qualify and publish measured deltas

Run the full native suite once in debug and release after final code/fixture
changes, with authenticated inputs stable throughout each run. Run the native
root unit/integration/CLI/o65 tests and affected LF/CRLF paths. Normalize host
fixture text before instrumentation and rebuild embedded fixtures in an isolated
CRLF checkout when affected; do not normalize binary or ATASCII evidence.

Rebuild and execute the small corpus and Dijkstra in both emission modes. Rebuild
the frozen Exec shell in both modes. Record per-guard amounts, destinations and
encoding, per-routine deltas, total executable/XEX sizes and secondary dispatch
shortening. Preserve all ABI/frame/home/local-peak contracts and guard counts.
Stack/DP data traffic should match; branch instruction counts, cycles and
guard-cycle totals should improve and must be measured rather than held to old
budgets. Keep existing vbcc failures as failures in comparison reports.

Publish results under `docs/benchmarks/65816-guard-branches/`, qualification
under `docs/abi/action65816-guard-branches-qualification.json`, and update this
plan, backlog and quality-plan status. Commit the qualification slice. Compiler
qualification does not qualify hosted Exec or update its compiler pin.

## Validation commands and completion gate

Use `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0` and Python `-B` for local runs. Focused commands:

```sh
python3 -B tools/compare65816/test_guard_ranges.py
cargo test --features native65816-state-proof --lib mir65816::
cargo test --features native65816-state-proof --test mir65816_emission --test mir65816_state_boundary --test mir65816_o65
python3 -B tools/native65816-runtime-tests/qualify.py --test guard_branches --test stack_faults --test call_padding --test control_flow --test state_tracking --test preemption --test indirect --test o65
```

Final backend qualification:

```sh
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
cargo test --features native65816-state-proof --test mir65816_contract --test mir65816_abi --test mir65816_emission --test mir65816_state_boundary --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
```

Use the existing comparison build/execution commands for corpus and Dijkstra,
including generator `--verify-crlf`. Do not run 6502/68k suites or a repository-wide
NIR sweep for this backend-only change. Repeat passing checks only for affected
subsequent changes or unresolved failures. This proposed plan itself requires
documentation/link checks only.

Completion requires correct raw/optimized machine execution, four short
conditionals and both retained JMLs in each unchanged guard, exact fault/success
state and ordering, passing relocation and asynchronous checks, complete guard
accounting, and explained measurements against the call-padding baseline.
