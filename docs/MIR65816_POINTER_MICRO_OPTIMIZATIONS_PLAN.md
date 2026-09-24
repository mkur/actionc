# Native 65816 pointer micro-optimization plan

Status: slices 1-7 implemented and qualified; slices 8-12 pending. Baseline:
`87feebf1`, after native 24/32-bit returns. Each numbered implementation slice is a separate
commit. Prioritize emitted code size and report execution-cycle tradeoffs.

## Evidence and objective

The [Exec816 list analysis](benchmarks/65816-execlists/README.md) covers twelve
list operations from frozen Exec revision `c3500c8`. Optimized Action code
occupies 3,557 bytes, including
540 bytes of entry/call guards; the body total is 3,017 bytes. Equivalent
optimized Calypsi code occupies 1,089 bytes, including 169 bytes of shared
helpers. Its packed fields use `__far24`, with `__huge` working pointers to
retain bank carry. All 135 input vectors pass for Action and Calypsi in both
compiler modes, with matching debug/release host results and both I states.
vbcc's 1,038-byte optimized result fails 54 vectors and is not a correctness
target. These are operation comparisons with different ABIs.

A handwritten Remove body demonstrates 50 bytes and 121 cycles, compared with
the current 82 bytes and 154 cycles, excluding the entry guard in both cases.
It uses six DP scratch bytes, overlapping word transfers and a short-lived X
value. Fifteen cases, including bank crossings and aliases, pass with both I
states. This is a reference for later slices, not a promised compiler result
or evidence that repeated external accesses are generally legal.

All forecasts below describe this baseline. Refresh them after each slice;
mode transitions, eliminated copies and branch relaxation make savings overlap.

## Boundaries

- Keep ABI v1, guards, result normalization and per-domain scratch ownership.
  Add no bank-zero reservation. Preserve allocation in the early selection
  slices; edge staging and the final lifetime slice may change private storage
  only through their checked allocation contracts.
- Implement general target behavior in MIR65816 selection or checked rewrites.
  Do not recognize particular Exec routine names or recover facts from SemIR.
- Reuse existing physical effects, machine/home liveness, typed instructions,
  checked rewrite transactions, replay and branch layout. Extend their facts
  where necessary rather than adding competing state tracking.
- Preflight complete homes and subranges, widths, stack deltas, live register
  lanes and flags before emission. Malformed facts are errors; legal unsupported
  cases retain a fallback without partial emission.
- Keep source-memory reads and ordering distinct from captured private values.
  Never read or write a fourth byte of a packed three-byte pointer.
- The current [emission contract](MIR65816_EMISSION_CONTRACT.md) permits
  overlapping word copies in specified private homes but forbids duplicated
  external/indirect accesses. Slices 1-8 preserve that boundary. Slice 9 is an
  explicit prerequisite for slices 10-11; this plan does not itself change it.

Related foundations: [checked analysis and rewrites](MIR65816_ANALYSIS_REWRITE_FOUNDATION_PLAN.md),
[pointer allocation](MIR65816_POINTER_ALLOCATION_PLAN.md),
[word edge copies](MIR65816_WORD_EDGE_COPIES_PLAN.md), and the completed
[BYTE/pointer comparisons](MIR65816_BYTE_POINTER_COMPARISONS_PLAN.md).

## First group: addressing and private values

### 1. Zero-offset indirect accesses

Implemented. The closed checked rewrite removes 23 zero-index setups in each
list-module mode, saving 69 bytes. Optimized code is 3,488 bytes; Remove's body
is 76 bytes / 148 cycles. All 135 vectors pass in both modes and both host
builds, with unchanged memory traffic and storage. See the
[slice measurements and qualification](benchmarks/65816-pointer-micro/README.md).

Select `LDA/STA [dp]` for displacement zero. Remove an associated `LDY #0`
only when machine liveness and effects prove its Y assignment and N/Z effects
unneeded. Preserve access width, memory order and all live state, including
store sequences whose flags came from LDY. Use the existing addressing and
rewrite machinery; no textual assembly peephole.

The audit contains 23 adjacent candidates: up to 69 bytes before layout effects,
including six bytes in Remove. Check both load and store paths, live Y/N/Z
rejection, bank crossings and exact volatile access traces.

### 2. Captured three-byte null reduction

Implemented and qualified. The list module saves another 42 bytes in each
mode (3,446 optimized). Per-vector cycle changes range from −7 to +30; the
additional stack reads and the nonzero-value slowdown are retained explicitly
in the [slice 2 measurements](benchmarks/65816-pointer-micro/slice2/delta.csv).
All 135 list vectors pass in both modes, both I states and both host builds.

For Eq/Ne against null on either side, reduce a captured pointer in A16 with
two overlapping words. For a three-byte incoming pointer at `4,S`:

```asm
LDA 4,S
ORA 5,S
BEQ is_null              ; BNE for non-null
```

This core is six bytes including a short branch, versus the current twelve-byte
low-word/bank short-circuit core. It reads only the three owned bytes, repeating
the middle byte. It stays in A16. Nonzero low words may take more cycles because
the second load is unconditional; retain that tradeoff in the measurements.

Start with the existing checked stack-temp and parameter classifier. Do not
widen the DP allocator whitelist merely to add a DP form. Reuse canonical BYTE
0/1 materialization and existing adjacent-branch sole-use proofs. General
non-null equality remains on its existing path. Account for changed A/N/Z
results and require proof that only the intended Boolean/branch result is live.
Earlier external or volatile pointer captures remain unchanged.

Cover zero, every individual pointer bit, `$000100`, bank-only values, null on
either side, mutable parameters, final valid displacements, both Boolean
consumers and nonempty/same-target edges. Switching to A8 before OR-ing the bank
is not a valid reduction: it loses the middle byte's contribution to Z.

### 3. Native three-byte representation-preserving casts

Implemented and qualified. Raw list code saves 60 bytes and optimized code
saves 56 (3,390 optimized bytes). All list vectors pass in both modes and host
builds; cycles improve or stay equal. The repeated private middle byte adds
reads/writes without changing frames, DP traffic or reservations. See
[slice 3](benchmarks/65816-pointer-micro/slice3/delta.csv).

Route bit-preserving three-byte pointer/address casts through the existing
native private-transfer machinery. Admit identical or disjoint checked homes
first. Preserve semantic cast operations; do not combine this with temporary
elimination, widening or narrowing. Include entry/exit mode costs in selection.

Thirteen casts currently occupy 162 bytes. That is the cost being targeted,
not a predicted saving. Check self-copies, partial-overlap fallback, mutable
parameter homes, full byte extents and differing entry widths.

### 4. Captured-pointer AddressOf at offset zero

Implemented and qualified. Both modes save 60 list-module bytes (3,330 optimized).
All oracle vectors pass in both host builds; cycles improve by up to 62 with no
regressions. Staging DP traffic falls while allocation stays unchanged. See
[slice 4](benchmarks/65816-pointer-micro/slice4/delta.csv).

Implement address formation from a captured pointer plus zero as a native
private copy, avoiding the DP staging and bytewise address arithmetic. Start
with disjoint checked homes and retain fallbacks for symbolic/object bases and
unsupported addressing forms. Reuse slice 3's private-copy eligibility.

Three measured sites cost 26 bytes each. An A16-entry native copy suggests
eight bytes per site. Empty width excursions disappear with replaced code and
must not be counted again as separate savings.

### 5. Captured-pointer AddressOf at constant nonzero offsets

Implemented and qualified. Both list modes save 116 bytes (3,214 optimized),
with up to 92 fewer cycles per vector and no regressions. All oracle vectors
pass in both host builds. Boundary and IRQ/NMI probes verify the live bank carry
and modulo-24-bit result; larger/indexed/overlapping forms retain the fallback.
See [slice 5](benchmarks/65816-pointer-micro/slice5/delta.csv).

Extend slice 4 with native low-word addition and bank carry. Start with positive
16-bit constants, keeping address movement modulo 2^24. Preflight the complete
source/destination relationship; any overlapping home needs its own safe
schedule or the fallback. Do not materialize an unnecessary four-byte pointer.

Four measured `+3` sites cost 45 bytes each; an A16-entry candidate suggests
16 bytes each. Verify carry at `$xxFFFF`, wrap at `$FFFFFF`, upper constant
boundaries, neighboring canaries and unchanged nonconstant/unsupported forms.

### 6. Single three-byte edge copy

Implemented and qualified. Optimized lists save 40 bytes (3,174 total) and up to
56 cycles per vector, with no regressions; raw output is unchanged. Direct
copies retain one two-byte A-save word to preserve hidden B, while identities
need only final bank-byte state repair. Shared allocation/verification preflight,
full-register backedge checks, relocated execution and IRQ/NMI re-entry pass.
All list oracle vectors pass in both host builds. See
[slice 6](benchmarks/65816-pointer-micro/slice6/delta.csv).

Select one native private pointer transfer for an eligible edge assignment
instead of complete byte staging. Start with disjoint homes and handle an
identity assignment with the required state repair. Preserve simultaneous
edge semantics and final A/N/Z obligations. Derive staging capacity from the
same checked plan used by emission.

Check loop backedges, edge arguments that remain live, final stack bounds,
identity moves, unsupported partial overlaps and unchanged fallback behavior.

### 7. Acyclic three-byte edge copies

Implemented and qualified. Optimized FindName saves another 34 bytes; the list
module totals 3,140 optimized bytes. Per-vector cycles improve by up to 310,
without regressions; raw output is unchanged. Exhaustive three-move schedules,
independent byte-staging comparisons, backedges, IRQ/NMI and o65 tests pass.
All list vectors agree across host builds. See
[slice 7](benchmarks/65816-pointer-micro/slice7/delta.csv).

Extend the checked scheduler to multiple complete three-byte assignments.
Remove identities and order transfers only when the entire schedule is safe.
Cycles, partial overlaps and unproved geometry retain staging. Do not schedule
the overlapping word pieces independently: one pointer move must not destroy
another pending source. Recheck final allocated homes and staging capacity.

FindName currently spends 172 bytes on edge transfers and Enqueue spends 63.
These are baseline costs, not promised savings. Cover permutations, repeated
sources, identities, cycles, partial overlaps and successor live-ins.

### 8. Captured pointer increment/decrement

Specialize typed 24-bit pointer `+1` and `-1` with native low-word arithmetic and
bank carry/borrow. Reuse slice 5's checked arithmetic where appropriate, while
keeping address formation and pointer-value computation distinct. Start with
private captured values and preserve the existing fallback for other operands.

This targets FindName's two string-pointer updates. Cover both wrap directions,
bank-zero/high-bank values, source/destination relationships and live state.
No source-memory operation may move across a call or alias barrier.

### Measurement checkpoint

After slice 8, rebuild the complete list module, frozen Exec, the small native
corpus and Dijkstra in both modes. Report per-routine and whole-image sizes,
guard bytes, cycles and private storage. Use the remaining costs to prioritize
the second group. Do not extrapolate the list-module ratio to all of Exec.

## Second group: approach the handwritten pointer sequence

### 9. Define eligibility for repeated external accesses

Specify which existing facts, if any, prove that overlapping word accesses to
ordinary RAM may repeat the middle byte. Absence of a volatile marker alone
does not establish every required property. Address extent, observable access
behavior, aliases and concurrent access assumptions must be explicit.

Unknown memory, MMIO and volatile accesses keep their current sequences. Do
not infer permission from an Exec routine name or from successful benchmarks.
If the proof needs new NIR storage/effect facts, introduce them as a separate
shared-contract vertical slice with verifier and boundary documentation changes.
Do not enable slices 10-11 until eligibility is representable and verified.

### 10. Overlapping word loads from eligible three-byte memory

For admitted memory, load words at offsets zero and one while retaining A16.
Start with a distinct private destination and stable address base. Prove all
accesses remain within the three-byte extent, including bank crossings and
displacement limits. Keep exact existing traces on every ineligible path.

### 11. Overlapping word stores to eligible three-byte memory

Add the corresponding stores separately. Prove the repeated middle-byte write
is permitted, the complete source survives both stores, the pointer base stays
valid and no neighboring byte changes. Test aliases and interrupted execution;
do not claim a multi-instruction pointer update is atomic.

### 12. Reuse a dead base and a short-lived register value

Within one bounded, checked selection window, capture the complete replacement
pointer before overwriting a dead address-base home. Add the short-lived X
handoff only with explicit register-lane and instruction-level lifetime facts.
Keep source-memory ordering and account for every intermediate instruction.

The current allocator deliberately keeps inputs and outputs interfering for
the whole MIR operation. Do not globally weaken that rule to fit the example.
Validate the exceptional sequence, effects, scratch ownership and continuation;
reject calls, unsupported aliases or live register conflicts. Reuse and the X
handoff may be delivered as separate commits if their proofs are independent.

Measure progress against the handwritten Remove reference, including its
six-byte scratch use. The general compiler result need not match it exactly.

## Delivery and qualification

For every implementation slice:

1. Freeze the preceding compiler/artifact baseline. Add focused correctness,
   unsafe-admission rejection and fallback coverage for the behavior changed.
2. Run affected native unit, integration and qualified VM targets in the
   relevant raw/optimized modes. Check physical effects, replay and layout as
   applicable; include fixed/o65 relocation and IRQ/NMI coverage when changed
   instruction/state behavior requires it. Exercise actual LF/CRLF parsing or
   instrumentation paths whenever newline-sensitive fixtures change.
3. Run the list-state oracle in both compiler modes and preserve complete
   memory/canary, result, stack, DP and interrupt-mask checks. Treat the known
   external vbcc failures as recorded failures, not expected-success exemptions.
   Use the committed comparison tools/fixtures and retain their artifact hashes
   as the reproducible implementation gate.
4. Record code bytes and cycles, including guard and helper accounting, plus
   stack/DP traffic and reservations. Attribute reductions to the current
   slice; distinguish encoding forecasts from measured changes.
5. Update the relevant emission/allocation contract and this plan's status,
   then commit the bounded change. Broaden to full affected-backend qualification
   for substantial emission changes and the group checkpoint; avoid repeating
   passing unrelated suites. Keep qualification inputs stable during each run.

Use the [native qualification runner](../tools/native65816-runtime-tests/README.md).
If slice 9 changes shared NIR/semantic/verifier/printer contracts, also run the
required fixture snapshots, NIR sweep and full `cargo test`; backend scoping
does not waive those shared checks.

Tail-call selection, cross-routine helper extraction, broad register allocation
and public ABI changes remain separate work, to be prioritized from the new
measurements. Exec's compiler pin and hosted qualification are also separate.
