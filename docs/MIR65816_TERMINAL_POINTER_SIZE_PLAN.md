# MIR65816 pointer forwarding into final stores and calls

Status: slices 1–4 implemented; one slice remains. Measured saving: **14,668 B**;
the estimate is **6,543 B below** the cap. Slices 1–3: `4baf5e2e`, `175a2494`,
`506614e0`. See the [series measurements](benchmarks/65816-terminal-pointer/README.md).
Continue the five primary slices below, committing each after focused validation
and size measurement.
The smaller optimizations are a follow-up queue, outside this primary series.

## Objective and baseline

Reduce Exec's loaded code plus initialized data toward the **256 KiB** release
cap, excluding debug stack guards. Use compiler `53dd2af0` and the frozen Exec
`622b139-dirty` workload: 631 routines and 120 verified input hashes.

The [current analysis](benchmarks/65816-exec-opportunities-53dd2af0/README.md)
estimates **270,269 bytes (263.9 KiB)** loaded without guards, leaving an
**8,125-byte** gap. Pointer forwarding into final consumers models **15,024
bytes** saved. Budget **11–14 KiB** until implementation proves the candidate
counts; that would put this workload at roughly **250–253 KiB**.

The estimate subtracts 72,252 guard bytes from the guarded compiler image and
adds the existing 8,300-byte assembly and 2,307-byte initialized-data allowances.
It is not a separately linked guard-disabled release. Do not count compiler
data twice or attribute guard removal to these optimizations.

## Shared implementation contract

Extend the existing [pointer forwarding planner](../src/mir65816/emit/pointer_forwarding.rs)
and its read-only source bindings. All selection remains target-private
MIR65816, using typed IDs, addresses, operands, liveness and allocated homes.
The audit's printed-MIR parsing is not an implementation strategy.

A binding may consume a store or direct call only when that operation contains
the captured value's final uses. The operation remains an ordering barrier;
the binding does not extend past it. Keep these rules in every slice:

- Admit only canonical, complete, nonvolatile three-byte private captures.
  Incoming parameters must retain the existing whole-routine immutability and
  non-escape checks. Local sources must retain complete, non-addressable local
  ownership and disjoint frame extents.
- Count all occurrences across the routine, including hidden address/index
  uses, call arguments, terminators and edges. Every use must belong to the
  same block and a supported consumer. Repeated arguments count separately.
- Reject intervening calls, stores, aggregate copies and volatile operations,
  even if they appear unrelated to the source. Earlier consumers retain the
  current supported-operation checks. Cast consumers and cross-block uses
  continue to retain the capture.
- Classify the terminal use by role: store address base, call argument or
  stored pointer value. A pointer index, indirect callee target or unsupported
  additional role cannot silently become an admitted use.
- Preflight every consumer before omitting the capture. Unsupported but valid
  homes, roles or addressing combinations retain the original capture and all
  its original reads. Malformed MIR still receives its existing diagnostic.
- Keep borrowed read homes separate from writable temporary allocations.
  Destination stores and call-result capture always use allocated homes.
  Never fabricate a temporary definition, home-equivalence or A/NZ witness
  for storage that was not written.
- Preserve ABI layout, frame reservations, spill/edge slots, stack peaks,
  guard checks, DP layout, relocation rules and exact external memory traffic.
  No external access is omitted, widened, duplicated or reordered.

Keep the general barrier predicate conservative. Add an explicit final-consumer
admission check before stopping at a barrier; accept it only if its occurrences
complete the routine-wide use count. Retain the operation's normal tracking
barrier and expire the read binding when its authorized reads finish.

Update the [emission contract](MIR65816_EMISSION_CONTRACT.md) with these
invariants alongside each implemented slice. No NIR/SemIR or public ABI change
is planned.

## Five primary commits

| Slice | Scope | Candidates | Modeled saving | Cumulative |
|---|---|---:|---:|---:|
| 1 | Incoming pointer → final store address | 574 | 4,592 B | 4,592 B |
| 2 | Incoming pointer → final direct-call arguments | 501 | 4,008 B | 8,600 B |
| 3 | Local pointer → final store address | 495 | 3,960 B | 12,560 B |
| 4 | Local pointer → final direct-call arguments | 251 | 2,008 B | 14,568 B |
| 5 | Incoming/local pointer → final stored value | 57 | 456 B | 15,024 B |

These count captures, not distinct stores or calls. They are implementation
targets, not required numerical outcomes. Record exclusions and actual deltas.

### 1. Incoming pointers used as the final store address

Suggested commit: `65816: forward incoming pointers into final store addresses`.

Admit a nonvolatile scalar store whose indirect base is the captured pointer
and whose remaining operands do not use that capture. Resolve the original
incoming home through the existing read resolver while preparing the address;
then emit the store through the existing scalar/address selectors. Preserve
their displacement, index, width and bank-carry checks. A selector that cannot
consume the resolved source must keep the capture.

Do not yet admit stored-pointer values or local sources at the terminal store.
Keep existing eligible earlier reads and multiple uses within the stable window.

Tests: payload widths 1/2/3/4, constant and captured payloads, zero/nonzero
displacements, indexed paths, bank-crossing stores and exact access order.
Add refusal cases for a later use, intervening barrier, volatile store, cast,
pointer-as-index and a second unsupported role. Check the last legal source
byte at stack displacement 255 and rejection beyond it. Retain tests that
reject forged parameter immutability and address escape.

### 2. Incoming pointers used by the final direct call

Suggested commit: `65816: pack final call arguments from incoming pointer homes`.

Admit only resolved `Direct` calls with a matching native call plan and exact
three-byte argument homes for every occurrence of the bound pointer. Indirect
targets, helper/runtime target variants and width-changing arguments remain
outside this slice. Other arguments keep their existing selectors.

Preflight the original source through
[call argument selection](../src/mir65816/emit/call_copies.rs) and
[argument pushes](../src/mir65816/emit/call_pushes.rs). For source displacement
`h` and outgoing extent `O`, conservatively require `h + O + 2 <= 255`, then
retain each actual changing-S check. A valid call whose incoming source fails
this limit must fall back to its previously captured, nearer temporary.

Support both existing push construction and reservation/store fallback. Read
all bound arguments before transfer, then expire the binding before JSL and
result capture. Guard-before-construction order, argument/padding bytes,
cleanup and native results remain unchanged. A call whose result immediately
returns must still use the existing result-forwarding path correctly.

Tests: repeated pointer arguments, several borrowed inputs in one call, mixed
BYTE/word/pointer/LONG operands, PEA padding interactions, exact outgoing areas,
maximum displacements, guard faults before construction, recursion, both
construction paths and returned results. A later use after the call must
retain the original capture. Exercise IRQ/NMI during partial argument packing.

After this commit, report the budget checkpoint. The **8,600-byte model** is
only 475 bytes beyond the original gap; continue the local-pointer slices for
headroom rather than treating that small estimated margin as release readiness.

### 3. Local pointers used as the final store address

Suggested commit: `65816: forward local pointers into final store addresses`.

Reuse slice 1's terminal-role and consumer preflight with the existing local
source ownership checks. A canonical source assignment before the capture is
allowed; every store between capture and consumer remains a barrier. A source
update after the terminal store may participate in a new, separate binding.

Tests: initialized local pointers, earlier reads followed by the final store,
multiple captures around source reassignment and address-taken/partial/volatile
source rejection. Keep malformed frame overlap and out-of-extent checks.
Prove the final address preparation reads the latest eligible local value and
does not reload the omitted capture's reserved slot.

### 4. Local pointers used by the final direct call

Suggested commit: `65816: pack final call arguments from local pointer homes`.

Combine slice 3's source checks with slice 2's complete call preflight and
binding expiry. Keep the same outgoing layout and stack-peak bounds. A call
earlier in the window, or any captured-value use after the terminal call,
retains the capture even when the local source itself is non-addressable.

Tests: repeated local arguments, mixed incoming/local pointer sources, local
updates between separate windows, recursion/reentry, changing-S boundary
fallback and no borrowing through the callee. Extend the existing packing
interrupt fixture with local sources instead of duplicating the full matrix.

### 5. Pointers used as the final stored value

Suggested commit: `65816: forward private pointer values into final stores`.

Admit a complete three-byte value stored by a nonvolatile final store, using
the existing pointer transfer and geometry checks. Preflight address preparation
and both complete homes before elision. Require disjoint private homes where
applicable; keep source self-writes, partial overlaps and a capture used in both
address and value roles on the existing path in this slice.

Copying to external memory must retain the low-word plus exact bank-byte
traffic. Overlapping-word transfers are allowed only under the existing private
home rules. No fourth byte may be read or written. Expire the binding after
the payload transfer and keep the store barrier.

Tests: NULL and nonzero pointer values, private and indirect destinations,
bank-crossing three-byte stores, neighbor canaries, unsupported overlap and
later-use fallback. Cover stores where the address uses a separate borrowed
pointer. The 57-site model splits into 37 incoming and 20 local captures.

## Validation and evidence per commit

Extend the existing [planner unit tests](../src/mir65816/emit/pointer_forwarding_tests.rs)
and [runtime pointer tests](../tools/native65816-runtime-tests/tests/pointer_forwarding.rs).
Keep the existing barrier-refusal cases; add distinct positive terminal cases.
Use fresh typed definitions in MIR fixtures and verify them before emission.

Run affected 65816 checks, selected by the changed consumers:

- Planner/selector unit tests; call-copy and push unit tests for slices 2/4.
  Check successful elision and atomic fallback, including an omitted home that
  must not be read within the binding's use window.
- Relevant cases in `mir65816_emission`, `mir65816_address_selection` and
  `mir65816_state_boundary`; inspect any snapshot changes individually.
- Runtime `pointer_forwarding` and `replay`; add `call_copies`, `call_pushes`
  and `call_returns` coverage when their paths change. Run the new behavior in
  both debug and release. Use the existing `qualify.py` runner with explicit
  `--test` targets so the patched VM and provenance checks are retained.
- Raw/optimized source, LF/CRLF through the actual fixture/instrumentation
  paths, flat image and two rebased o65 placements. Check exact observed memory
  traffic, neighbor canaries, returned values, S and ABI state.
- IRQ/NMI restoration at reached instructions in the newly admitted windows,
  including same-routine reentry and both task domains. Reuse existing enabled
  and masked interrupt cases and stack-guard failure probes.

Do not repeat already passing unrelated suites. Broaden within the affected
backend when a failure or new interaction requires it. **Do not run full/final
backend or hosted Exec qualification, including after the last commit.** This
is the user's standing constraint; focused runtime targets remain required.

After each slice, compile the same frozen Exec inputs once and compare against
the preceding slice and `53dd2af0`. Reuse the
[inventory probe](benchmarks/65816-exec-opportunities-53dd2af0/probe.rs) and adapt
the existing [measurement checks](benchmarks/65816-constant-index-byte/measure.py)
to a new `docs/benchmarks/65816-terminal-pointer/` series directory when the
first implementation lands.

Verify all 120 hashes, unchanged MIR operations, ABI, frame/temporary placement,
stack peaks, initialized data and guard shapes/amounts. Record per-routine and
per-span deltas, actual admitted/excluded counts, direct and cumulative savings,
loaded-size estimate and remaining headroom. Investigate any growing routine
before committing. Keep full images and runtime artifacts under ignored target
directories; reuse existing build caches to limit disk growth.

Each commit contains implementation, focused regressions, the relevant contract
update and compact measurement evidence. Update this plan's status with actual
commit IDs and results. Keep unrelated working-tree changes outside the commits.

## Follow-up queue after the primary series

Recount these candidates after pointer forwarding; do not automatically add
them to the five-commit scope. They provide independent options if more margin
or further general compiler improvement is wanted.

| Priority | Follow-up slice | Model | Required boundary |
|---|---|---:|---|
| 1 | Direct BYTE Add/Sub/AND/OR/XOR operands | 868 B | Remove RHS scratch staging; preserve A8, carry setup, operand roles and result store. |
| 2 | Adjacent private LONG captures | 2,308 B | Sole adjacent consumer; typed read-home resolution; no external-load borrowing. |
| 3 | Adjacent private word captures | 772 B | Preserve existing A/home/NZ facts; retain source loads when reloads already vanished. |
| 4 | Adjacent private BYTE captures | 416 B | Separate from existing load/Eq/Ne fusion; require exact byte homes and complete use coverage. |
| 5 | Bounded indexed `AddressOf` | 1,288 B | Exact BYTE index, bounded scaling, full 24-bit carry; existing fallback for unbounded CARD indexes. |
| 6 | Native remaining LONG/mixed edge copies | 873 B | Preserve two-phase staging, all sources before destinations, A16 edge entry and cyclic-copy semantics. |

No general alias analysis, cross-call residence, allocator redesign, frame
compaction or new memory-access contract is needed for the primary series.
The broader immutable-parameter strategy adds only a **264-byte transfer
ceiling** beyond its overlap with these terminal windows and remains deferred.
