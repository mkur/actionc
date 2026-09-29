# MIR65816 expressions and their consumers

## Objective

Select an expression together with the operation that uses its result, before
allocating temporary storage or emitting instructions. Extend the existing
[storage-demand plan](MIR65816_STORAGE_DEMAND_PLAN.md), using typed MIR and
explicit register lifetimes. Do not emit captures merely to remove them later.

Keep expression evaluation order, integer widths and overflow behavior intact.
An intermediate CARD calculation still wraps at 16 bits before widening to
SIZE or LONGCARD. Do not reassociate arithmetic or move memory reads.

## Executable slices

1. **Adjacent CARD expression chains ending in widening.** Walk backward from
   the existing unsigned widening consumers. Admit a direct nonvolatile load
   or native CARD calculation whose sole use is the left operand of the next
   already-selected calculation. Support Add/Sub/And/Or/Xor and constant shifts
   by 1–3 bits. Emit each operation at its original site, passing A16 directly
   along the chain; only the widened result gets a memory home. This removes
   the remaining load-to-shift capture in `MetadataBytes`. Keep the same
   planner authoritative for allocation, verification and emission.
2. **Stores and returns.** Add terminal consumers for BYTE/CARD results: direct
   stores with complete destination bounds, and returns using the ABI result
   registers. Preflight the entire selected region before omitting a home.
   Integrate with direct assignments, scalar-DP allocation and existing return
   selection so each operation has one owner. Preserve epilogue and stack-check
   requirements; an empty temporary map alone never permits removing a guard.
3. **Comparisons and branches.** Allow an adjacent native comparison to consume
   an expression in A, then use its flags for a sole-use branch when eligible.
   Keep signedness, comparison operand order and flag establishment explicit.
   Unify ownership with the existing compare/branch and top-bit selectors.
4. **Call arguments and wider results, separately.** First select a sole-use
   expression into its outgoing ABI argument slot. Account for guards, argument
   evaluation order, S movement and call clobbers; retain captures whenever
   setup would destroy the value. Then inventory SIZE/LONGCARD consumers and
   implement only forms with an explicit low-word/high-lane register strategy.
   Do not extend the A16 plan by pretending wider values fit in A.

Each slice must be independently usable and measured before expanding the
consumer set. No general register allocator or shared NIR change is needed.

## Admission and fallback

- One definition and one operand occurrence, counted across the entire routine.
- Producer and consumer are consecutive operations in one block. Every link
  has its own typed width and operation identity.
- Expression chains follow only the left operand. The right operand is an
  immediate or an existing complete home; it is never another register-only
  value.
- No crossing calls, helpers, labels, stores or unrelated operations. Volatile,
  indirect/indexed, signed and unsupported-width producers retain their normal
  captures. A rejected link may leave a shorter safe suffix selected.
- Ordinary memory-backed values retain their interference and alias rules.
  Register-only values have no fake stack or DP entries in image maps.
- No deferred loads, commutation, reassociation or instruction peepholes.

The closed scalar-DP/X profile retains ownership of its whole routine. Existing
top-bit tests take precedence over generic expression comparisons. Pointer and
ADDRESS results retain their current profiles; arithmetic SIZE results can use
the existing checked borrowed-parameter bindings. These decisions are made
before sparse allocation and do not depend on emission order.

Reserved bank-zero change: **0 fixed bytes, 0 bytes per task**. Smaller frames
remain inside existing task stack reservations. No public ABI, compiler pin,
ROM configuration or play-image change is included.

## Validation per slice

Test selection and absent homes, raw and optimized NIR, integer boundaries and
wrapping, dirty hidden B, source-span ownership, calls/barriers and rejected
multi-use or reordered operands. Check emitted machine code under the native
runtime harness, including guards and IRQ/NMI restoration during selected
chains. Exercise LF/CRLF through any changed fixture instrumentation.

Run affected MIR65816 units and emission/artifact integrations as a batch;
use focused native runtime targets. Measure `MetadataBytes` and compile the
existing Exec inputs with an explicitly recorded local compiler override.
Report routine bytes and frames separately. Compile-only measurements do not
qualify a hosted Exec image. Do not run unrelated backends or release suites.

## Status

All four bounded slices are implemented in
[`home_demand.rs`](../src/mir65816/emit/home_demand.rs) and
[`accumulator_homes.rs`](../src/mir65816/emit/accumulator_homes.rs).
The backward walk selects a chain only when its final consumer is supported;
an arithmetic operation can now consume one register-only temporary and
produce the next.

- **Slice 2:** BYTE/CARD chains feed exact-width native returns and stores into
  bounded mutable frame objects/parameters. Absolute, global, indirect and
  indexed destinations retain their existing path. Direct store selection
  updates the existing stored-word witness for subsequent loads.
- **Slice 3:** unsigned BYTE/CARD Eq/Ne/Lt/Ge consume A, with ordinary Boolean
  materialization or fused branches. Zero equality tests reuse matching-width
  producer flags; signed ordering and Gt/Le retain captures. Existing top-bit
  selection keeps priority.
- **Slice 4a:** one exact-width BYTE/CARD argument can flow to its outgoing slot
  without a capture. The checked push plan proves payload and padding coverage;
  Y preserves the value across the stack guard, then is consumed before JSL.
  Multi-argument and indirect calls retain captures. Existing direct call-result
  forwarding remains available.
- **Slice 4b:** unsigned SIZE/LONGCARD Add/Sub/And/Or/Xor and narrow unsigned
  widening can return in A/X, including adjacent same-width identity casts.
  Wide ALU operands retain complete homes or are immediate; no wide expression
  chains or register values across calls are admitted. The low word stays in Y
  while the high lane consumes carry/borrow. SIZE explicitly clears high X.

Validation: 318 MIR65816 unit tests pass (one existing ignored test), plus 39
emission/o65/state-boundary integration tests. Focused native runtime targets
cover expression consumers, direct assignments, casts, BYTE/word comparisons,
top-bit branches, native SIZE/LONGCARD arithmetic, call copies/pushes/returns,
shared epilogues, stack checks and stack faults. Both raw and optimized NIR
execute through wrapping, dirty hidden B, exact memory extents, helper calls and
o65 relocation. The expanded IRQ/NMI test covers selected stores, comparisons,
argument setup and widening in both task domains, with LF/CRLF instrumentation.
Typed replay, sparse-home verification and the reviewed emission snapshot pass.
No unrelated backend or full release qualification was run.

## Slice 1 measurements

| Measurement | Before | After |
| --- | ---: | ---: |
| `MetadataBytes` routine | 157 bytes | 153 bytes |
| `MetadataBytes` frame / local stack peak | 12 / 22 bytes | 12 / 22 bytes |
| Existing generated Exec routine code, 960 routines | 491,585 bytes | 491,529 bytes |

Twelve Exec routines shrink, none grow, and all frames retain their previous
size. The Exec frame-map validator accepts all 960 maps. The shift portion
now emits the following with A16 active:

```asm
LDA $10,S
LSR A
LSR A
STA $06,S
SEP #$20
LDA #$00
STA $08,S
```

Only the widened result is stored. The former intermediate `STA $06,S` and
`LDA $06,S` are never selected or emitted.

Baseline: compiler `bb5890ca6a45ef14697e489a8d65ca45765c115e`. Candidate:
that revision with this slice in the working tree. Candidate `src/**/*.rs`
digest: `1cdcfe33f1ca0c014d49a6d779d902d028059510c06a1863f6d577f020c94129`;
compiler executable SHA-256:
`2833a6cafdf661ecfbbb14cf14b2d4b02ad3f20d22d186ffba54a790ec0f06ea`.
Exec revision `134130bdb8ce1374ca7d5e6011ed98c4ccd62151` and its 133
source/layout inputs are unchanged. Input digest:
`751059513e938ac69f5d578219029b39347388dccda46797916d30d03eb7898d`.
Digest procedure and compile command are recorded in the
[storage-demand measurements](MIR65816_STORAGE_DEMAND_PLAN.md#measurements).
These measurements use the local compiler override; Exec's recorded compiler
pin and play image have not changed.

## Remaining-slice measurements

The comparison below uses the slice 1 result as its baseline and the same 960
Exec routines and source/layout inputs.

| Measurement | Slice 1 | All four slices |
| --- | ---: | ---: |
| `MetadataBytes` routine | 153 bytes | 146 bytes |
| `MetadataBytes` frame / local stack peak | 12 / 22 bytes | 8 / 18 bytes |
| Existing generated Exec routine code | 491,529 bytes | 489,875 bytes |

The remaining slices save **1,654 bytes**: 194 routines shrink, none grow. Five
frames shrink and none grow. The Exec frame-map validator accepts all 960 maps.
The final SIZE addition in `MetadataBytes` now produces A/X directly:

```asm
LDA $02,S
CLC
ADC $06,S
TAY
SEP #$20
LDA $04,S
ADC $08,S
REP #$20
AND #$00FF
TAX
TYA
```

Its inputs still need complete homes because multiplication is a helper call
and the shift result feeds wide arithmetic. The result needs no home; the
ordinary epilogue preserves A/X while releasing the remaining eight-byte frame.
Reserved bank-zero change is **0 fixed bytes, 0 bytes per task** for each slice.

Candidate: compiler `bb5890ca6a45ef14697e489a8d65ca45765c115e` with all four
slices in the working tree. Candidate `src/**/*.rs` digest:
`843fa1d4f72af1e5a82183023e3fad110531dd39ab946a31c3d9cce3e0997b14`;
compiler executable SHA-256:
`15c23247a975911bfdd9ed24d2edd78d177d104f2b970709c3a190903226946d`.
Exec revision and its 133-input digest are unchanged from slice 1. The compile
command is the same with output `/tmp/exec816-expression-final.a816.json`.
This is a compile-only measurement using the local override. Exec's compiler
pin and play image remain unchanged; these results do not qualify a hosted
image.
