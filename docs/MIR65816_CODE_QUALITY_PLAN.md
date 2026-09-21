# Native 65816 code-quality improvement plan

Status: proposed on 2026-09-21, following empty-edge cleanup at `192b6d8`
(emitter implementation `3dd5cb2`). This document records the next implementation
slices; none of the improvements below is implemented by this plan commit.

## Objective and baseline

Preserve the public ABI and reduce internal data movement, then improve control
flow and register use. Current main already has CFG-aware stack-slot reuse and
a restricted pointer-leaf DP allocator. General lifetime-based stack reuse is
therefore an existing capability, not a new item in this plan. See the
[temporary-allocation contract](MIR65816_TEMPORARY_ALLOCATION.md) and
[empty-edge results](MIR65816_EMPTY_EDGES.md).

The immutable comparison baseline is the
[empty-edge snapshot](benchmarks/65816-empty-edges/after/tables.md). Both compilers
implement a general unsigned 16-bit sum loop; input 13 is supplied at runtime,
and both return 91. Measurements run from function entry through RTL, including
Action's stack guards and excluding caller setup:

| Mode / compiler | Code bytes | VM cycles | Additional stack bytes |
| --- | ---: | ---: | ---: |
| Optimized actionc | 154 | 1,735 | 16 |
| Optimized vbcc | 22 | 344 | 0 |
| Raw actionc | 174 | 2,032 | 14 |
| Raw vbcc | 32 | 533 | 4 |

The optimized [Action listing](benchmarks/65816-empty-edges/after/sum_loop.optimized.actionc.lst)
uses native word arithmetic but repeatedly copies stack temporaries and stages
edge assignments. Its repeating path takes 29 instructions and 123 cycles.
The [vbcc listing](benchmarks/65816-empty-edges/after/sum_loop.optimized.vbcc.lst)
retains the counter in X and the sum in DP, taking eight instructions and
25 cycles per continuing iteration. Action's guard accounts for 45 static
bytes and 32 executed cycles; most of the cycle gap remains inside the loop.
Raw vbcc's compact stack-based implementation also demonstrates scope for
improvement without changing public argument placement.

## Ordered implementation slices

Implement and measure each slice separately. The larger entries below should
be divided into independently qualified commits rather than combined into one
allocator or emitter rewrite.

| Order | Improvement | Initial scope |
| --- | --- | --- |
| 1 | Direct single-word edge copies | Bypass staging for one checked word argument. Preserve multi-value edges, frame allocation and guards. |
| 2 | Local accumulator forwarding | Avoid reloading private stack values already held in A. Retain stores and existing homes initially. |
| 3 | Simpler control flow and width handling | Use proven A16 block-entry contracts, remove jumps to adjacent blocks, then select short branches where final placement permits. |
| 4 | Parallel-copy scheduling and coalescing | Remove self-copies, schedule independent moves directly and retain staging for cycles; subsequently shrink unused storage and coalesce compatible homes. |
| 5 | Scalar DP allocation | Extend allocation to verified, call-free scalar routines with loops, after defining operation and helper clobbers. |
| 6 | X/Y residency across loops | Retain suitable scalar values across basic blocks, enabling counters updated directly with instructions such as DEX. |

These are native MIR65816 strategy and emission changes. Consume verified typed
facts; do not recover semantics from source strings or SemIR. If a later slice
needs stronger NIR facts, introduce and verify those in a separate boundary
change before relying on them.

## First slice: direct single-word edge copies

The [detailed implementation plan](MIR65816_SINGLE_WORD_EDGE_COPIES_PLAN.md)
records checked selection, independent counts, forecasts and qualification.

Start in [`select.rs`](../src/mir65816/emit/select.rs), using the existing checked
word-edge preflight. Select only an edge with exactly one argument and one
successor parameter, both exactly two bytes, with supported physical homes or
an immediate source. Validate the complete transfer before emitting anything.

For a stack source, replace:

```asm
LDA source,S
STA staging,S
LDA staging,S
STA destination,S
```

with:

```asm
LDA source,S
STA destination,S
```

An immediate source similarly needs only LDA immediate and STA destination.
The word is fully captured in A before the destination write; there is no
second assignment whose source could be destroyed. Preserve A16 restoration,
typed JML fixups, argument validation and existing diagnostic behavior.
Multi-value, mixed-width and unsupported edges keep their existing paths.

Keep the staging reservation and all storage maps initially. Do not combine
this slice with self-copy elimination, frame shrinking, branch relaxation or
general register retention. This isolates instruction selection from allocation.

The optimized sum loop has two static single-word edges: initialization and
the backedge. At input 13 they execute 14 times. Removing one word store/load
pair saves four bytes per static edge, ten cycles per execution, and two stack
bytes read and written per execution:

| Metric | Baseline | First-slice forecast |
| --- | ---: | ---: |
| Code bytes | 154 | 146 |
| VM cycles | 1,735 | 1,595 |
| Stack bytes read | 273 | 245 |
| Stack bytes written | 216 | 188 |
| Fixed frame bytes | 16 | 16 |

These are listing-derived forecasts, not measured implementation results.
Verify them through executed machine code. The raw sum-loop baseline has no
nonempty edges and should remain unchanged; constructed verified MIR must
exercise selected edges in both compiler modes.

## Proof obligations for subsequent slices

**Accumulator forwarding.** Initially track only private compiler storage,
including exact byte range, S-relative identity and accumulator width. Removing
LDA must preserve its required N/Z effect as well as its value. Invalidate
knowledge at calls, helpers, unknown instructions, joins and relevant writes or
stack movement. Retain external and volatile access order and width. Do not
infer that unchanged A implies unchanged flags.

**Control flow and mode knowledge.** Distinguish MIR block entries with a proved
A16 contract from arbitrary internal labels. Prove all incoming paths before
retaining mode knowledge. Remove jumps only when control can reach the intended
successor directly, including any required edge assignments. Short branches
need range and bank checks after layout, compatible relocation handling, and
long-transfer fallbacks. Keep guard behavior and fault paths intact.

**Copy scheduling and coalescing.** Prove physical byte-range compatibility and
preserve swaps, cycles, repeated sources, mutable parameter homes and successor
live-ins. Retain a checked staging fallback. Do not globally weaken the current
closed-operation interference rule. Remove reservations only after selection
no longer writes them, then recompute frame extent, incoming displacements and
stack-guard accounting.

**DP and register allocation.** First make per-operation and helper scratch,
A/X/Y, flags and width clobbers explicit. Allocate only approved scratch ranges;
apparently unused DP bytes alone are not a sufficient contract. Begin with
call-free scalar routines and a conservative operation whitelist. Keep values
spanning ordinary or helper calls on their invocation's stack; preserve fallback
emission for unsupported operations. Mutable parameters and addressable locals
need separate storage and alias proofs before promotion. Add X/Y residency only
when selection can honor its addressing, width and clobber constraints.

**Preemption.** Preserve the existing domain-owned DP and complete CPU-state
restoration contract. Live scratch may survive asynchronous suspension only
under that contract. Qualify interruptions while resident values and arithmetic
flags are live. Keep IRQ/NMI isolation and interrupt reserves unchanged.

## Validation and completion criteria

For each slice:

1. Preserve the previous snapshot and declare expected instruction and traffic
   changes before implementation. Use the unchanged 14-pair / 66-vector corpus
   and add general regression cases, never sample-specific selection rules.
2. Check exact selection, rejection and fallback cases. Include boundary words,
   frame displacements, both branch arms, backedges, cyclic copies and malformed
   plans as relevant. Keep independent ca65 encoding checks for new sequences.
3. Execute serialized raw and optimized machine code through the
   [qualification runner](../tools/native65816-runtime-tests/README.md) in debug
   and release hosts, with both incoming interrupt-mask states. Cover clobbering
   direct/indirect/helper calls, aliasing and volatile traffic, IRQ/NMI suspension,
   relocated o65 code, recursion and stack-boundary failures as affected.
4. Record code bytes, cycles, stack depth and stack/DP traffic. Permit only the
   explicitly predicted traffic changes; require unchanged unaffected artifacts
   and behavior. Recheck LF/CRLF corpus builds and retain the known external vbcc
   optimized unlink failure rather than exempting it.
5. Run the compiler checks required by the affected contracts. NIR changes also
   require the contributor-mandated snapshots, sweep and full compiler suite.
   Update the relevant contract, save qualification evidence and commit the
   completed slice while preserving unrelated local changes.

Preserve [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md), image v3, the
[o65 profile](MIR65816_O65_PROFILE.md), public argument/result placement and stack
guards. Frame sizes may shrink only with verified accounting. No slice reduces
the platform's interrupt headroom or changes Exec816's compiler pin; adopting
new compiler output in Exec816 remains a separate integration qualification.

The first four steps target stack-resident code. DP and X/Y allocation address
the larger remaining gap to vbcc. Set further numerical targets from each new
baseline rather than promising cumulative gains before implementation.
