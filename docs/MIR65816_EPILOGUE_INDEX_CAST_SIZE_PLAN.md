# MIR65816 epilogue, BYTE-index and integer-cast size plan

Status: slices 1–4 implemented; slices 5–6 pending. See
[measured results](benchmarks/65816-epilogue-index-casts/README.md).

Implement six independently reviewable compiler slices, committing after each
slice's focused checks and frozen Exec measurement. Optimize loaded code plus
initialized data toward the **256 KiB (262,144-byte) release cap, without stack
guards**. Full/final backend and hosted Exec qualification remain deferred at
the user's request, including after the last slice.

## Baseline and expected return

Use compiler `fc729cf0` and the same frozen Exec `622b139-dirty` workload:
631 routines and 120 verified source hashes. The committed
[pointer-forwarding measurements](benchmarks/65816-pointer-forwarding/README.md)
and [summary](benchmarks/65816-pointer-forwarding/completed-plan-summary.json)
provide the baseline.

| Component | Bytes |
|---|---:|
| Compiler code, including guards | 354,944 |
| Subtracted guards: 2,676 guards | 72,252 |
| Compiler code, excluding guard ranges | 282,692 |
| Package assembly, carried forward | 8,300 |
| All initialized data | 2,307 |
| **Estimated loaded size without guards** | **293,299** |
| **Gap to the release cap** | **31,155** |

The 951 compiler data bytes are already included in initialized data. This is
an estimate obtained by subtracting guard ranges from the frozen guarded build,
not a separately linked build with guards disabled.

| Work | Current cohort / footprint | Modeled saving |
|---|---|---:|
| Shared return epilogues, slices 1–2 | 1,488 tails in 383 routines / 13,164 B | 6,488 B |
| Bounded BYTE-index accesses, slices 3–5 | 124 sites / 7,937 B | 4,266 B |
| Unsigned integer casts, slice 6 | 219 nonempty sites / 3,338 B | 924 B |
| **Total** | | **11,678 B (11.4 KiB)** |

Treat roughly 10–11 KiB as a working budget, not a guaranteed result. Realizing
the entire model would leave **281,621 B (275.0 KiB)**, still **19,477 B
(19.0 KiB)** above the cap. Shared tails use a BRL cost model; any extra BRA
relaxation is unbudgeted upside. Recount selections and refusals after each
slice instead of crediting the entire cohort in advance.

## Ownership and common constraints

Keep selection and profitability in MIR65816, using verified typed operations,
existing storage identities, frame facts and resolved symbols. No SemIR/NIR
changes, source-pattern recognition or optimization over assembly text are
needed. Emission retains responsibility for typed instruction effects, labels,
fixups, layout, maps and proof replay.

Preserve the existing ABI, frame reservations, stack peaks, direct-page layout,
guard ordering and initialized data. Preserve live X loop state. Calls and
machine blocks remain barriers; this plan introduces no cross-call forwarding
or new alias assumptions. Reject an optimization before emission if its full
address, width, ownership or cost proof fails, and retain the existing path.

Every new instruction or internal control-flow form needs consistent encoding,
physical effects, state tracking and independent replay. Follow the
[emission contract](MIR65816_EMISSION_CONTRACT.md),
[selected-action contract](MIR65816_SELECTED_ACTIONS.md),
[typed replay contract](MIR65816_TYPED_REPLAY.md) and
[control-flow contract](MIR65816_CONTROL_FLOW.md). Update the relevant contract
document in the slice that changes it; do not introduce a raw-byte escape.

## Slice 1 — shared void-return epilogues

Suggested commit: `65816: share void return epilogues`.

Start with ordinary void routines with a nonzero frame and multiple reachable
returns. Their repeated teardown is typically seven bytes:
`TSC; CLC; ADC #frame_extent; TCS; RTL`. Keep one teardown and redirect eligible
returns to it. Single-return and zero-frame routines retain their current tails.

Implementation:

1. Add a typed per-routine epilogue plan around the existing `return_tail` and
   `release` paths in [select.rs](../src/mir65816/emit/select.rs). Choose a
   deterministic placement and account for every redirected jump and any mode
   repair. Select sharing only when the complete routine becomes smaller.
2. Represent the epilogue as a checked internal join. Every incoming path must
   agree on native environment, allocated frame, stack depth, no outstanding
   argument/transfer pushes, and required register widths. Clear value/home
   facts at the join. Do not pretend it is an original MIR block or alter MIR
   predecessor counts to obtain a width permission.
3. Resolve the internal-label width contract explicitly: the current emitter
   does not automatically retain mode permission at an internal label. Either
   emit and cost a conservative mode repair, or prove the permission on every
   predecessor and validate it through fresh CFG/replay checks.
4. Use the existing typed jump and
   [local relaxation](MIR65816_LOCAL_RELAXATION_PLAN.md) machinery. Release the
   frame exactly once and finish with the ordinary typed native return. Keep
   helper, fault and indirect-call RTL mechanisms outside the initial scope.
5. Preserve truthful source spans for each return-site jump and one ownership
   location for the shared tail. Update trace/fixup positions through ordinary
   layout; do not find repeated suffix bytes after emission.

Acceptance: two and many returns share a tail when profitable; small or
incompatible cases retain their tails. Cover forward/backward edges, unreachable
returns, branch reach boundaries, returns after calls and recursive routines.
Negative proof tests must reject inconsistent depth, mode and jump targets.
Verify S, D, DBR and interrupt state restoration, guard order and interrupt
reentry during teardown. Record the extra branch's cycle cost.

## Slice 2 — shared value-return epilogues

Suggested commit: `65816: share native value return epilogues`.

Extend the established join to value routines. Keep result preparation at each
return site; share only the teardown. The current value tail is typically nine
bytes because TAY/TYA preserve A during stack release. X must remain intact.

Admit every existing native result class: BYTE with zero-extended A, word in A,
24-bit value in A/X with zero-extended X.high, and 32-bit value in A/X. Retain
the contracts in [wide returns](MIR65816_WIDE_RETURNS.md) and
[captured BYTE returns](MIR65816_CAPTURED_BYTE_RETURNS.md).

The typed return summary and physical liveness must show the result lanes live
through the join and teardown. Do not invent known A/X values at the join.
Preserve immediate call-result forwarding in
[call_returns.rs](../src/mir65816/emit/call_returns.rs): outgoing argument
cleanup still precedes the jump, with no new result store/reload. Resolve
borrowed pointer sources before the jump; bindings do not become shared-tail
facts. Count all width repairs in the size decision.

Acceptance: distinct values returned by different paths remain distinct,
including BYTE high-byte clearing, pointer bank bytes and both LONG halves.
Cover constants, captured values, direct/indirect calls, nested calls and
recursion. Inject interrupts throughout result preparation and the shared tail,
including reentry of the same routine. Recheck slice 1 and native result tests.
Report the combined result against the **6,488-byte** model, separating branch
relaxation and necessary join repairs.

## Slice 3 — stride-one BYTE indexing through Y

Suggested commit: `65816: select BYTE indexed byte accesses with Y`.

Extend [addresses.rs](../src/mir65816/emit/addresses.rs), which already handles
CARD-indexed stride-one BYTE accesses. Start with verified unsigned BYTE index
temps, BYTE loads/stores, stride one, and bounded constant displacement.

Build the same captured or symbolic 24-bit base in the existing pointer scratch.
Read exactly one byte from the index home, explicitly zero-extend it into A16,
add any admitted displacement and transfer the offset to Y. Perform the access
through `[pointer],Y`. Do not read the adjacent index byte, and do not reuse a
generic helper that overwrites Y with a constant after preparing dynamic Y.

Create a reusable checked offset proof, using wide host arithmetic:

```text
255 * stride + displacement + access_width - 1 <= 65535
```

Require nonnegative displacement, a positive representable constant stride,
and complete private homes for the index and captured base. Keep volatile or
unsupported accesses on the existing path. Preflight stored values as well as
addresses, and preserve the existing external byte access and ordering.

Use the existing pointer-source resolver so bounded forwarding still works.
Do not read stale homes of forwarded temps. Symbol bases retain checked
relocations; Y provides the runtime offset. Keep full 24-bit address addition,
bank carry and wrap behavior. Do not convert the runtime offset into a symbol
addend without an existing object-interior proof. Reject scratch conflicts
without widening allocation permissions.

Acceptance: load and store fixtures cover every index 0–255, nonzero
displacements, bank crossing, 24-bit wrap, relocated symbols, reused inputs and
forwarded bases. Trace external accesses and poison the byte adjacent to the
index home. Include signed/wrong-width indexes and failed offset-bound proofs
as refusal cases. Preserve live X and test interrupt reentry across preparation
and access. Historical model split: **52 sites / 1,119 B**; recount the actual
admitted subset.

## Slice 4 — power-of-two strides and wider elements

Suggested commit: `65816: select scaled BYTE indexes and wide accesses with Y`.

Reuse slice 3's bound and base proof for power-of-two strides and 2/3/4-byte
payloads, including stride-one wider accesses. Scale the zero-extended BYTE
index with A16 shifts and transfer the completed offset to Y. Traverse payload
pieces with checked Y increments, without repeating address construction.

Add typed accumulator shifts and Y increments if needed; the current implied
instruction set lacks these forms. Define physical effects, flag changes,
width requirements, encoding and replay before using them in selection.
Validate their encodings independently with ca65 and execution in the VM.

Use the existing memory-access policy for payload widths and order. The offset
proof alone does not authorize combining external byte accesses into words.
Use exact byte pieces where required; admit word pieces only where the existing
contract permits them. Never touch a fourth byte of a three-byte object.
Ensure store preparation preserves Y and the prepared base, including when
values or addresses share source storage. Reject unsafe overlap or scratch
geometry before emitting any part of the selected sequence.

Acceptance: load/store coverage for each payload width and representative
power-of-two strides; boundary cases where the last byte is offset 65535 and
where it would be 65536; odd homes, neighboring canaries, bank crossings and
exact access order. Exercise newly typed instructions under replay and
interrupt injection. Historical model split: **49 sites / 1,816 B**; retain
fallbacks where the required memory contract is unavailable.

## Slice 5 — other bounded constant strides

Suggested commit: `65816: select bounded constant stride BYTE indexes`.

Extend the same address plan with a small deterministic A16 shift/add sequence
for other positive constant strides. Use existing private index scratch where
necessary; introduce no multiplication helper, extra stack pushes or new
direct-page reservation. Audited strides such as 3 and 44 must follow from the
general rule, without special cases for Exec routines.

Prove each intermediate and the final displacement/payload bound before
selection. Establish carry explicitly before additions. Include scratch
loads/stores, mode transitions, displacement adjustment and payload transfer in
the cost; retain generic lowering unless the complete candidate is smaller.
Keep Y operation-local and preserve the base pointer and store value throughout.

Acceptance: exhaustive BYTE index values for representative odd and composite
strides, with zero/nonzero displacement and each supported payload width.
Include maximum-bound, overflow, expensive-sequence and scratch-conflict
fallbacks. Reuse exact-access, relocation and preemption checks from slices 3–4.
Reconcile all three indexing slices against the **4,266-byte** combined model;
the historical non-power-of-two split is **23 sites / 1,331 B**.

## Slice 6 — native unsigned integer casts

Suggested commit: `65816: select native unsigned integer casts`.

Add a small selector before the bytewise Cast fallback in
[select.rs](../src/mir65816/emit/select.rs). Admit typed integer casts with an
unsigned source and verified widths, initially complete private stack homes
for the modeled 2/3/4-byte transfers. Copy full words in A16, handle an odd byte
exactly, and zero only the destination's extension bytes. Narrowing copies
only retained bytes. Keep pointer/address casts and signed-source casts on
their existing paths.

Preflight complete source/destination extents and geometry. Permit disjoint
homes or an explicitly safe same-start transfer; reject partial overlap without
a separate copy-order proof. Eliminate an identity cast only when width and
complete storage home both match. A three-byte transfer must not read or write
a fourth byte. Resolve source storage through existing facts and obtain the
destination's actual writable home independently.

Compare the full cost with the byte fallback, including entry/exit mode
requirements. Do not regress pointer-copy coalescing or borrowed source
bindings. The model credits **924 B**, with no assumed frame shrink.

Acceptance: 2→3/4, 3→2/4 and 4→2/3 casts, identity and same-start cases, live
sources and unsupported overlap/width fallbacks. Check zero, sign-bit patterns,
all-ones values and truncation, including `0x80`, `0x8000`, `0xffffff`,
`0x80000000` and `0xffffffff`. Use odd homes, last legal stack displacements
and poisoned neighbors. Verify repeated execution and interrupt reentry.

## Focused validation and measurement per commit

Add focused native targets for shared epilogues, BYTE indexing and integer
casts; those target names are proposals, not existing tests. Batch related
cases within each target. Run relevant existing consumers as follows:

| Slices | Existing native coverage to select |
|---|---|
| 1–2 | call_returns, byte_returns, captured_byte_returns, word_returns, wide_returns, guard_branches, preemption |
| 3–5 | address_selection, memory, pointer_values, pointer_forwarding, pointer_preemption |
| 6 | pointer_values, pointer_coalescing, state_tracking, replay, affected wide-value consumers |
| New typed forms / joins | effects, instruction_effects, state_tracking, replay, affected home analysis |

Run affected selector unit tests and the `mir65816_emission` and
`mir65816_o65` integration targets when their contracts change. Check the
`mir65816_state_boundary` snapshot; review any intentional differences instead
of accepting them mechanically. Use focused native debug checks plus release
checks for new behavior, raw/optimized fixtures, flat execution and two o65
placements. Preserve existing IRQ/NMI coverage across both task domains and
interrupt states. When adding newline-sensitive host fixture processing, run
both LF and CRLF through the actual loader/instrumentation path.

Use selected test targets, for example:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test call_returns
```

Do not edit compiler or fixture sources while this runner is checking hashes.
Do not run the unfiltered qualifier, a full backend suite, hosted Exec
qualification or repository-wide tests as a routine step in this plan.
If shared frontend/IR contracts unexpectedly become necessary, stop that
expansion and revise the scope and required checks first.

For each slice, rebuild the frozen workload using the existing size-inventory
probe and compare it to the preceding slice and `fc729cf0`. Verify all 120
frozen input hashes. The local cache is under `target/exec-current-audit/`,
with the baseline inventory stem `pointer-local-final`; these ignored files
are working artifacts, not durable evidence. Preserve or reconstruct the
matching workload before measuring if the cache has been cleaned.

Store compact reproducible summaries, routine/span deltas and input identities
under `docs/benchmarks/65816-epilogue-index-casts/`. Report:

- Total code, guard bytes, guard-subtracted code, initialized data and the
  updated loaded-size estimate and gap. Do not double-count compiler data.
- Selected/refused sites, direct savings, width-repair costs and branch-layout
  interactions; explain every routine that grows.
- Frames, local peaks, direct-page use, ABI and guards, expected unchanged.
- Relevant cycle changes, especially the extra branch on shared returns.
- Checks actually run, with full/final qualification explicitly deferred.

Shared epilogues deliberately change internal edges, jump counts and span
ownership. Adapt inventory comparisons to verify those specific structural
changes and valid remapping; do not require identical transfer counts or
silently weaken unrelated invariants. Do not count disappearance of duplicated
source spans as a saving unless the emitted image shrinks accordingly.

Each commit is complete when the slice's behavior and refusals are covered,
typed proofs replay, focused checks pass, and the frozen size delta is recorded.
After slice 6, publish the measured combined result and rerank remaining
footprints. Do not mark the 256 KiB target achieved from this model or start
broader pointer-lifetime/alias work without a new plan.
