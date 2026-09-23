# Native 65816 outgoing argument padding

Status: proposed against main `07d7afb8` on 2026-09-23. Implementation has not
started. This is the first selected follow-up to the
[current-source Exec size audit](benchmarks/65816-exec-size-detail/README.md).

## Objective and scope

Initialize only padding in the outgoing call area, then write each argument
payload byte once through the existing argument-copy loop. Apply the same
selection to raw and optimized native emission, including direct Action calls,
resolved runtime/assembly calls and typed indirect calls.

Preserve ABI v1, image v3, o65, argument order and widths, alignment, zero
padding, outgoing extent, caller cleanup, result capture, every stack guard and
its amount, frame/home allocation, and task/IRQ/NMI contracts. Reserve no new
stack, DP or bank-zero storage. Do not change the Exec pin or live sources.

This is a local instruction-selection change. No SemIR/NIR change, public MIR
form, optimizer pass, liveness extension or general dead-store elimination is
needed. Native-width argument copies, return improvements, removal of the zero
load, guard compaction and broader branch relaxation remain separate work.

## Baseline and expected benefit

The [audit inventory](benchmarks/65816-exec-size-detail/inventory.json) inspected
final machine bytes and typed call plans for frozen Exec `c3500c8`, using
compiler `9f16e08b` (the subsequent `07d7afb8` is documentation only):

| Measurement | Raw | Optimized |
| --- | ---: | ---: |
| Static call sites | 1,777 | 1,769 |
| Sum of outgoing extents | 8,397 | 8,343 |
| Sum of actual argument widths | 6,923 | 6,878 |
| Padding bytes retained | 1,474 | 1,465 |
| Payload-clear instruction bytes removable | **13,846** | **13,756** |
| Compiler routine bytes before | 534,815 | 503,453 |
| All executable bytes before | 542,380 | 511,018 |
| Stack guards retained | 2,328 | 2,320 |

Totals are static sums over sites, not concurrent stack requirements. Each
removed `STA d,S` saves two code bytes and one executed stack write whenever
that call site is reached. Exact guard, frame and call contracts must remain
equal. Before secondary layout effects, the forecast is 520,969 / 489,697
compiler bytes and 528,534 / 497,262 executable bytes, raw / optimized.
Existing branch relaxation may produce additional savings; report those
separately rather than changing the payload-store forecast. XEX changes must be
measured, since packing and metadata also change.

Keep this workload frozen. Exec's new `stack_checks:true` field requires the
audit's explicit compatibility adapter for current main. Retain that same
checked-build input and record its use; integrating that feature is outside
this implementation. Revalidate baseline hashes before using retained artifacts.

## Existing contracts and implementation

[`emit::materialize`](../src/mir65816/emit/mod.rs) verifies MIR before selection.
[`verify_native_arguments`](../src/mir65816/lower.rs) already checks declaration
order, natural alignment, supported scalar widths, the end of the final
argument, and the required odd outgoing extent. Call verification also checks
argument count, stack displacement limits, transfer form and cleanup.

In [`Builder::call`](../src/mir65816/emit/select.rs), the current order is:

1. Check `outgoing + transfer_peak`, then reserve the outgoing area.
2. Select A8, load zero and clear stack displacements `1..=outgoing`.
3. Copy every argument byte to `1 + home.offset + byte_index`.
4. Perform the existing direct or indirect transfer, cleanup and result capture.

Replace only the displacement set in step 2. Add a small private helper beside
the call selector that computes checked padding displacements before any
emission. Walk the already verified `StackArgument` homes with a cursor:

- Emit padding for the half-open gap from the previous argument end to the
  next argument offset, then advance to `offset + size`.
- Include the trailing gap up to `plan.outgoing_bytes`.
- Convert each zero-based padding offset to a checked one-based `d,S`
  displacement using the existing stack arithmetic. Use checked addition and
  reject invalid order, overlap, extent or displacement; never truncate to u8.

A short vector of checked displacements is sufficient. Reuse the existing
verifier as the ABI authority; do not create a second ABI layout planner or
duplicate coverage metadata in MIR. Invalid MIR must still fail through the
normal verified materialization path. The helper's range checks protect its
local arithmetic and complete before any guard or store is emitted.

**Do not use `native.argument_bytes` as the number of bytes to skip.** It is the
extent through the last argument and can contain alignment holes. Only actual
home ranges are payload. For `(BYTE, CARD, BYTE POINTER, LONGINT)`, argument
offsets are `0,2,4,8` and outgoing extent is 13. Keep clears at stack
displacements **2, 8 and 13**; remove the ten other clears, saving 20 code bytes.

Retain the existing `a8()` request and `LDA #0`, even when there is no padding.
Only stores disappear; STA preserves A and flags. Leave all argument loads,
explicit zero extension, payload stores and their order unchanged. Keep
indirect target capture, PHK/PER/synthetic RTL setup, native call effects,
cleanup and result handling unchanged. No new typed instruction or replay
permission is required. Normal tracked emission and layout regenerate spans,
fixups, labels and continuations from the shorter sequence.

## Why skipping the clears is safe

Let `S_body` be the caller's body stack anchor and `O` the outgoing extent.
After reservation, `S_call = S_body - O`. Outgoing bytes are at
`S_call + 1 .. S_call + O`. A source home at positive body displacement `d`
is still read at `S_call + O + d`, strictly above the outgoing area. Existing
checked displacement calculation includes this S delta. Incoming parameters
and mutable parameter copies retain their established homes.

Other accepted value sources are DP homes or immediate/address values. Passing
a pointer copies its captured numeric value; it does not dereference the
pointer during argument construction. Earlier volatile/aliased source reads
and argument-producing calls stay in their original order. `value_byte` writes
every byte of each argument home, including explicit extension bytes, before
control reaches the callee. No helper or nested call is introduced in this
window. The indirect target uses its existing capture path with the same delta.

Thus a payload byte has a single complete defining write before use, while
padding remains explicitly zero. No-argument calls still reserve and clear the
mandatory one-byte area. A single BYTE argument has no padding and still gets
its payload write. Zero-valued arguments retain their payload write as well.

The full area remains reserved before initialization. IRQ/NMI may interrupt a
partially constructed call; the existing context protocol must restore S,
A/X/Y, flags and execution-domain DP, and resume construction before transfer.
It must not expose another task's scratch or consume unfinished arguments.
No change to I, interrupt masking or stack-switch ordering is permitted. The
tests below verify this through emitted code, rather than assuming interrupts
cannot arrive between stores.

## Implementation commits

### 0. Capture a reproducible baseline and ABI observations

Add a focused `call_padding` native runtime test target and small fixtures,
reusing the assembly-import and bus-trace facilities in
[`interop`](../tools/native65816-runtime-tests/tests/interop.rs),
[`indirect`](../tools/native65816-runtime-tests/tests/indirect.rs) and
[`support`](../tools/native65816-runtime-tests/tests/support/mod.rs).
Seed unused outgoing memory with nonzero bytes. Have an independent assembly
callee record the complete incoming area, including holes and the tail, and
clobber caller-saved registers/scratch under the existing ABI.

Baseline tests assert argument values, zero padding, preserved caller homes,
cleanup, results and guards. Record the old per-byte write counts as measurement
evidence, not a permanent requirement for duplicate writes. Preserve the
small-corpus, Dijkstra and frozen Exec raw/optimized before artifacts and hashes
under a dedicated call-padding measurement directory. Reuse authenticated old
results when all relevant inputs match; do not regenerate unrelated baselines.

### 1. Select padding-only clears and prove the changed behavior

Implement the private padding calculation and replace the full-area clear
loop. Add focused emission/contract coverage to
[`mir65816_emission`](../tests/mir65816_emission.rs) and
[`mir65816_abi`](../tests/mir65816_abi.rs), with private helper tests only where
needed to exercise invalid range arithmetic. Keep production scope within the
native call selector.

Extend the runtime tests to require **exactly one write per outgoing byte**
between reservation and transfer: one argument write for payload and one zero
write for padding. Trace actual bus writes, including stores of zero; checking
only the final memory contents would miss the optimization. Exclude callee and
interrupt-stack activity from the construction window. Cover these distinct
boundaries in both compiler modes:

- No arguments; a single BYTE with no padding; a word with tail padding;
  mixed arguments with internal holes and a tail; odd payload with internal
  holes but no tail; adjacent three-byte pointers with alignment one; ADDRESS
  and SIZE with alignment two.
- Nonzero/zero literals, captured locals and incoming parameters, null and
  relocated data/code addresses, repeated arguments and nested argument calls.
  Keep source-side volatile/alias evaluation and narrow-value extension intact.
- Direct Action calls, resolved assembly imports and typed indirect calls;
  void and representative A/A+X result captures, plus repeated/recursive calls.
- The largest accepted outgoing displacement and source displacement with the
  reservation included; reject one-past limits, overlapping/unsorted homes,
  inconsistent extents, wrong argument counts and non-stack argument homes.
  Use constant sources for maximal outgoing-area cases where stack sources
  themselves cannot fit the current d,S limit.
- Direct and indirect reservation failures: exact floor, one byte short,
  ceiling violation and bank-zero underflow. Preserve failure A/X/S and forbid
  outgoing writes or transfer pushes before a failed guard.

Use the existing two-task context harness for a bounded mixed-argument probe.
Inject IRQ and NMI at each reachable instruction boundary in its changed
construction window, including after reservation, between padding/payload
stores and immediately before transfer. Check full restored state, task/DP
isolation, final argument bytes and bounded completion. Exercise direct and
indirect windows; reuse existing scheduler machinery rather than adding one.
Also execute moved o65 calls with mixed padding and a relocated indirect target
at both existing test placements, checking PER continuation and result behavior.

Update the [emission contract](MIR65816_EMISSION_CONTRACT.md) to describe complete
argument/padding initialization at transfer and the unchanged ownership and
stack invariants. Commit the selector, its focused tests and that contract
together after the focused checks pass.

### 2. Qualify and publish measured deltas

Run the full native 65816 suite once in debug and once in release after the
final code/fixture changes. Use the authenticated runner; keep its inputs stable
throughout each run. Run native root unit/integration/CLI/o65 targets and the
affected LF/CRLF fixture path. Do not execute 6502/68k suites or a full NIR sweep
for this backend-only selection change.

Rebuild the small corpus, Dijkstra and the frozen Exec shell in raw/optimized
modes. Record per-call payload bytes and removed stores, per-routine code
deltas, executable/XEX totals, unchanged ABI/frame/home/guard contracts and
bank-zero reservation delta **0**. Attribute further layout savings separately.
Measure cycles and stack/DP traffic through the corpus and Dijkstra execution
harnesses; call-containing routines are expected to shrink, while allocation
and stack peaks remain unchanged. Retain existing external-compiler failures
as failures in comparison reports. These compiler checks do not qualify hosted
Exec or authorize a compiler-pin update.

Commit the results, qualification manifest and backlog/quality-plan status
after validation. Do not retain an old full-clear production path or add a
feature flag solely for benchmarking.

## Validation commands and completion gate

Use `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0` for these local runs. During implementation:

```sh
cargo test --features native65816-state-proof --lib mir65816::
cargo test --features native65816-state-proof --test mir65816_abi --test mir65816_emission
python3 -B tools/native65816-runtime-tests/qualify.py --test call_padding --test interop --test indirect --test stack_faults --test stack_allocation
```

Final backend qualification:

```sh
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
cargo test --features native65816-state-proof --test mir65816_contract --test mir65816_abi --test mir65816_emission --test mir65816_state_boundary --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
```

Normalize host fixture text before newline-sensitive instrumentation; verify LF
and CRLF through the actual source/assembly preparation path, rebuilding embedded
fixtures when necessary. Preserve binary and ATASCII bytes. Rerun only checks
affected by subsequent changes, and report exactly what ran.

Completion requires correct raw/optimized emitted execution, zero padding and
one payload write per byte, unchanged guard/ABI/stack contracts, passing
relocation and asynchronous controls, and explained measured code deltas against
the frozen baseline. This plan itself requires documentation/link checks only.
