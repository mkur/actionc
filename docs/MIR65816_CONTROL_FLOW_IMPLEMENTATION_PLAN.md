# Native 65816 control-flow implementation plan: 3a–3c

Status: all three slices implemented and qualified as of 2026-09-22:
3a (`55579fd`), 3b (`7a2f616`) and 3c (`fc43602`). See
[results](MIR65816_CONTROL_FLOW.md). Originally proposed against main `a9e3c0b`
as the next three slices of the [code-quality roadmap](MIR65816_CODE_QUALITY_PLAN.md), following
the qualified [state-tracker foundation](MIR65816_STATE_TRACKER.md). The original
forecasts and implementation steps remain below; the results document records
their measured confirmation and per-slice qualification.

## Deliverable and boundaries

Implement, qualify and commit these improvements in order:

| Slice | Change | Deliberately bounded scope |
| --- | --- | --- |
| 3a | Omit redundant MIR-entry `REP #$20`. | Checked native A16/X16 block contracts; retain all value/flag/home barriers. |
| 3b | Omit a terminal JML to the physically following MIR block. | Emit every edge assignment first; retain block order and conditional dispatch. |
| 3c | Replace eligible conditional dispatch with a short branch. | Ordinary and fused MIR terminators, same-routine labels, signed-byte displacement; retain the existing long fallback. |

Keep the public [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md), image v3,
[o65 profile v1](MIR65816_O65_PROFILE.md), call/return conventions, allocation,
temporary homes/stores, copy scheduling/staging, stack guards and interrupt
headroom unchanged. No broader accumulator forwarding, source-memory caching,
DP/X/Y allocation, block reordering, jump threading, near calls, BRA or BRL.
Exec816 adoption remains a separate task; do not change its compiler pin.

All three are MIR65816 emission changes, applicable to raw and optimized NIR.
They consume verified MIR and existing ABI contracts. They need no SemIR/NIR
semantic changes and introduce no public optimization switch.

## Baseline and initial inventory

The initial baseline is the immutable [tracker snapshot](benchmarks/65816-state-tracker/after/tables.md),
qualified at `86fcef2`; the subsequent roadmap commit changed only documentation.
Planning rechecked 57 recorded source/test/benchmark hashes and all 224 artifact
files from its 56 comparison builds. Saved debug and release reports each contain
264 measurement records. This verifies saved evidence, not a new execution run.

Use `target/state-tracker-after` for the first comparison. If artifacts are
missing, reconstruct them in an isolated checkout of the qualified revision.
Never overwrite that directory or historical snapshots. Freeze the qualified 3a
output as 3b's baseline, then the qualified 3b output as 3c's baseline.

| Kernel / input | Mode | Bytes | VM cycles | Stack reads / writes | Stack peak |
| --- | --- | ---: | ---: | ---: | ---: |
| identity(13) | raw / optimized | 61 | 66 | 5 / 2 | 4 |
| sum_loop(13) | raw | 164 | 1,767 | 197 / 246 | 14 |
| sum_loop(13) | optimized | 140 | 1,395 | 165 / 188 | 16 |
| byte_sum($12FFFC,16) | raw | 262 | 4,678 | 530 / 559 | 16 |
| byte_sum($12FFFC,16) | optimized | 238 | 4,231 | 492 / 489 | 22 |

The saved [raw](benchmarks/65816-state-tracker/after/sum_loop.raw.actionc.lst)
and [optimized](benchmarks/65816-state-tracker/after/sum_loop.optimized.actionc.lst)
sum listings provide the following initial inventory. Addresses refer to the
original image at `$010000`; later slices must remap them to their own baseline.
Execution counts here are derived from the loop's 13 iterations and 14 tests,
not newly collected per-PC VM counters.

| Slice / mode | Original sites | Derived executions for input 13 |
| --- | --- | ---: |
| 3a raw | REP at `$01004E`, `$010074`, `$010095` | 14 + 13 + 1 = 28 |
| 3a optimized | REP at `$01003A`, `$010045`, `$01005E`, `$01007F` | 1 + 14 + 13 + 1 = 29 |
| 3b raw | JML at `$01004A`, `$010070` | 1 + 13 = 14 |
| 3b optimized | JML at `$010041`, `$01005A` | 1 + 13 = 14 |
| 3c raw / optimized | Conditional dispatch at `$010064` / `$01004E` | Predicate true 13 times, false once |

The REP at the internal true-arm label (`$01006E` raw / `$010058` optimized)
remains. Raw entry starts in byte mode and still needs SEP. Both facts distinguish
3a from removing every apparently repeated mode instruction.

For native execution, removing REP saves two bytes and three cycles per execution;
removing JML saves four bytes and four cycles. Replacing inverse-branch/JML with
the original short predicate saves four bytes, three cycles when the original
predicate is true and one when false. These timing assumptions agree with the
pinned qualification CPU's instruction paths; independently assemble and execute
the replacement sequences before accepting them.

| Conditional sum_loop forecast | Raw bytes / cycles | Optimized bytes / cycles |
| --- | ---: | ---: |
| After 3a | 158 / 1,683 | 132 / 1,308 |
| After 3b | 150 / 1,627 | 124 / 1,252 |
| After 3c | 146 / 1,587 | 120 / 1,212 |

These arithmetic forecasts assume exactly the sites above qualify, with no other
changes. Stack traffic and peaks stay at the baseline values. Identity should
lose its entry REP, predicting 59 bytes / 63 cycles in both modes. Do not promise
full-corpus savings from these examples: each slice starts with an independent
site inventory and dynamic counts across all 14 pairs / 66 vectors.

## Shared implementation and evidence rules

The typed [emitter](../src/mir65816/emit/tracked.rs) remains the only owner of
instruction effects. The [selector](../src/mir65816/emit/select.rs) supplies MIR
identity, edge arguments and layout intent. Encoding/finalization may change
representations of a proved transfer, but cannot recover semantics by scanning
opcode patterns or manufacture register/flag facts.

Keep logical edge identity separate from its encoding. Evidence identifies a
routine, source block and terminator/arm, target block, edge-copy ranges and final
transfer form. An omitted jump still has an edge; an empty emitted range is valid.
Proof metadata is nonserialized and cannot become a new ABI or loader dependency.

Before enabling each optimization, extend independent evidence tooling to:

1. Inventory candidate and rejected sites from verified MIR, finalized machine
   bytes and checked targets. Record reasons, old PCs, expected replacement and
   per-vector execution counts, including unreached sites. Freeze the inventory
   against source and artifact hashes before changing selection.
2. Compare results, exact memory traces, traffic, frames, peaks, guard behavior,
   existing forwarding/fusion/copy counts, and instruction/cycle deltas. Compare
   logical sites after address remapping; raw PC equality is no longer valid.
3. Keep a strict allowlist of byte changes: deleted mode/transfer instructions,
   selected short encodings, displaced addresses and derived metadata/layout.
   Surviving operations and memory accesses retain order. Distinguish routine
   code savings from file size changes due to bank padding and descriptors.
4. Reject missing sites, duplicate evidence, wrong predicates/targets, unexpected
   bytes and unexpected deltas. Use the independent decoder/VM to check emitted
   bytes; compiler traces and decisions are claims, never the expected oracle.

The current [equality checker](../tools/compare65816/check_state_tracker.py) is
appropriate for preparatory refactors only. Add a dedicated control-flow delta
checker with explicit per-slice expectations; do not weaken the equality gate
or accept arbitrary improvements under a general non-regression threshold.

## Slice 3a: checked MIR-entry width omission

### Implementation

`declare_blocks` currently seeds an A16 environment for every MIR label;
`edge` checks actual incoming environments, including backedges. `mark` clears
value facts and always revokes `mode_permission`. Consequently the first word
request at a block often emits REP despite a known execution width.

1. Build explicit MIR-entry proof eligibility from the routine entry and complete
   predecessor set: Goto, both branch arms, explicit Fallthrough and backedges.
   Do not treat `declare_blocks`' seeded environment or an unreachable block as
   evidence of an actual predecessor. Separate a required entry contract from
   the obligation to check its incoming transfers.
2. Establish the first block's execution contract from the checked ABI entry,
   prologue and parameter copies. Require native E=0, M=0, X=0, the established
   body stack anchor and no pending outgoing/push phase. Check every emitted
   incoming transfer against the same contract, including later-bound backedges.
   Verify predecessor obligations before returning finalized code. This uses
   the existing normalized-edge invariant; no value dataflow pass is needed.
3. Give eligible MIR bindings a private checked entry path that grants A16
   omission permission. Keep ordinary internal `mark` conservative. Granting
   permission does not retain A/X/Y, C/V/N/Z, home generations or the adjacency
   witness. It also does not authorize omitting SEP when entering byte mode.
4. When the entry proof is unavailable, retain existing explicit mode requests.
   A contradictory emitted edge is an emitter-contract error, not evidence that
   the join is safe. Never silently reinterpret an unknown or incompatible mode.
5. Keep guards, internal comparison/shift/copy labels, call and return sequences,
   helper boundaries and indirect-call resume labels on their existing policy.
   Calls still invalidate values and DP scratch; aliasing/memory barriers remain.

Confine behavior changes to `tracked.rs` and the MIR-entry setup in `select.rs`,
with focused tests in the emitter test modules. `State65816` remains authoritative;
do not add a competing width cache. Document the new distinction in the emission
contract when implemented. Byte positions, fixups, PER operands and spans continue
to be produced directly from the shorter stream, requiring no late compaction.

### Acceptance

- Positive: entry after parameter copies, multiple predecessors, forward edges,
  loop backedges, empty blocks and a first native arithmetic/comparison/return.
- Negative: byte-first blocks, unproved/dead entries, internal labels, indirect
  resumes, missing predecessors and incompatible width/stack contracts. Check
  both source-emission orders so a backedge cannot invalidate an accepted proof.
- Check A's hidden high byte and all preserved flags at joins, both incoming I
  states, clobbering calls/helpers, and the absence of new forwarding across joins.
- Require exactly two fewer code bytes per approved REP and three fewer cycles
  per execution; all data traffic and existing logical optimization counts match.
- Execute IRQ/NMI schedules around predecessor exits and the first successor
  instruction. Retain the established interrupt-state restoration contract.

Freeze its qualified artifacts and measured report before starting 3b.

## Slice 3b: terminal jumps to physically adjacent blocks

### Implementation

Today `edge` and `emit_word_edge` always finish with `jump`, even when the target
is the next MIR block. The source block's successor identity alone is insufficient:
the ordinary and fused branch emit the false edge, an internal true-arm label,
then the true edge. Falling out of the false edge would execute the other arm.

1. Preserve `routine.blocks` order. Pass the physically next block only to the
   final edge emitted for Goto/Fallthrough or the final true arm of an ordinary
   or fused branch. The preceding false arm receives no fallthrough permission.
   Neither equal targets nor CFG adjacency permits bypassing other-arm code.
2. Centralize the final transfer after edge preflight and all copy paths. Retain
   empty-edge checks, direct single-word copies, staged parallel word copies and
   mixed byte fallbacks, including self-copies and unused staging reservations.
3. Add a typed checked fallthrough operation that records the incoming edge and
   closes the current logical path without writing JML. Bind its pending target
   next; reject intervening instructions, a different label or end-of-routine.
   Treat this as a control-flow effect, not a fabricated CPU instruction. Keep
   the same barriers, width decisions and successor contract as the old jump.
4. Otherwise emit the existing JML and fixup. Do not remove internal-label jumps,
   tail transfers, calls, guard/fault jumps, backedges or jumps across routines.

This slice needs no general layout pass. Direct omission keeps all subsequently
emitted labels, spans, fixups and trace offsets current. Keeping the logical path
closed until the next binding prevents newly enabled register/flag forwarding
or accidental dependence on emitter traversal order.

### Acceptance

- Cover Goto, explicit Fallthrough, ordinary and fused branches; both branch
  outcomes; next block as either target; equal targets with different arguments;
  nonadjacent targets; backedges and unreachable intervening blocks.
- Exercise empty, direct-word, staged swap/rotation, repeated-source and mixed
  width copies. Compare their exact memory order, canaries and final live-ins.
- Retain malformed-edge/preflight diagnostics without partially emitted copies.
- Require four fewer bytes and four fewer cycles per removed/executed JML.
  Surviving REP/SEP policy, branch encodings, flags, data traffic and peaks match.
- Verify IRQ/NMI suspension with live copied A immediately before fallthrough
  and at the successor. A zero-byte transfer is counted via logical-edge reach,
  not as an executed opcode.

Update the JML-dependent evidence helpers before enabling omission, particularly
[word-edge indexing](../tools/native65816-runtime-tests/tests/support/word_edge.rs)
and [comparison/edge decoding](../tools/native65816-runtime-tests/tests/support/comparison.rs).
They must recognize proved fallthrough as well as relocated JML and reject a
missing or misdirected transfer. Preserve copy/fusion coverage instead of dropping
sites that no longer match the old byte pattern.

Freeze its qualified artifacts and measured report before starting 3c.

## Slice 3c: short conditional MIR dispatch

### Scope and representation

Initially relax only conditional transfers selected for ordinary MIR Branch and
compare-to-branch fusion. Materialized Boolean comparisons, guards/fault paths,
casts, shifts, helper/copy loops and indirect stubs retain their encodings. This
keeps fault timing and closed internal sequences outside the first relaxation.

The current `branch(predicate, label)` emits the inverse short predicate with
displacement +4 followed by JML. Record typed conditional sites at emission with
predicate, target label, origin and eligibility. A dispatch-specific API or typed
origin must distinguish eligible MIR terminators from internal sequences; do not
guess from opcode bytes or label numbering. No new predicate or comparison
semantics are introduced.

Add a private routine finalizer under `emit/`, after selection but before returning
`MachineRoutine`, relocation collection or either output writer. Initially run it
with all existing encodings retained and require byte/metadata equality. Keep
tracker effects coupled to logical emission; the finalizer only selects an
equivalent transfer encoding and resolves positions.

### Layout algorithm and metadata

1. Begin with every conditional site in its six-byte long form. With existing
   routine block order fixed and no internal alignment insertion, consider each
   eligible site in a hypothetical layout where that site becomes two bytes.
   Compute `target_offset - (branch_offset + 2)` in that layout, including the
   candidate's own four-byte removal for forward targets.
2. Select short form only for a resolved same-routine instruction label and a
   displacement in `[-128, 127]`. Repeat until no additional site fits. Other
   shortenings cannot increase the intervening distance under these constraints;
   each candidate changes at most once. Retain the long form when unproved or
   out of range. Bound iterations by candidate count and test determinism.
3. Emit the original predicate plus signed displacement for short sites. The
   retained long form uses its original inverse predicate and typed JML fixup.
   Do not invert the MIR arms, move copies or remove additional unconditional
   jumps during relaxation.
4. Produce one checked mapping from original boundaries to final boundaries.
   Remap labels, retained absolute fixup sites, PER operand sites, MIR span
   endpoints, conditional-site metadata and opt-in state-trace PCs together.
   Remove only the JML fixup owned by a shortened conditional. Permit empty
   spans and coincident labels; reject unrelated labels/fixups inside replaced
   instruction operands. Preserve trace event order when PCs coincide.
5. Revalidate ranges, sites, targets, nonoverlap and instruction boundaries after
   finalization. No metadata consumer sees a mixture of old and final offsets.
   In particular, PER still pushes continuation-minus-one after remapping.

Keep short-site records nonserialized, including operand and target positions,
so [relocation validation](../src/mir65816/relocation.rs) can check site overlap,
resolved displacement and placement. Resolve the local displacement in finalized
routine bytes; do not represent it as an absolute one-byte relocation or add an
o65 relocation kind. Test trace-on/off emission equality after finalization.

### Bank and output contracts

The [image linker](../src/mir65816/image.rs) already limits routines to 65,535
bytes, moves them to the next bank when necessary and leaves each bank's last
byte unused. [o65 preparation](../src/mir65816/o65/prepare.rs) applies the same
rule to text offsets; the reference relocator requires a 64 KiB-aligned text
base and validates routine extents. Therefore same-routine relative displacements
remain invariant at every accepted o65 placement.

Finalize before either packer computes routine sizes and before o65 collects
relocations. During placed-byte validation, check that the branch instruction,
its next PC and target share PBR, stay inside the routine and cannot depend on
low-word wrap. Preserve all existing placement rejection rules. Packing may
change later routine addresses or padding; regenerate their ordinary references
and descriptors from final sizes. No loader/profile or serialized trace change
is needed. A long local branch still needs a bank-contained routine; fallback
does not authorize an otherwise invalid placement.

### Acceptance

- Forward/backward displacement boundaries: -129, -128, -127, 0, 126, 127, 128;
  include the candidate's own shrink, cascading shortenings and retained long
  sites. Test near-bank-end routine placement and 24-bit address limits.
- Independently assemble all enabled predicate encodings. Execute taken and
  untaken cases with boundary signed/unsigned comparisons, ordinary byte
  conditions, both I states and all relevant N/Z/C outcomes.
- Include nonempty/cyclic edge copies, same-target arms, loops and backward
  layout targets. Internal helper/guard branches must remain byte-identical
  apart from relocated absolute targets.
- Validate remapped PER continuations, split/full address references, moved
  later routines, spans and traces. Reject missing labels, overlapping sites,
  invalid targets and stale metadata rather than emitting partial artifacts.
- Serialize/deserialize images and o65, relocate o65 at two legal text bases,
  then execute those bytes. Include multiple code banks, indirect calls,
  relocated imports/faults and preemption; compare branch displacement bytes
  across placements. Malformed/non-bank-aligned placements still fail.
- Require four fewer code bytes per short site, and `3 * true + false` fewer
  cycles for its original predicate. No data traffic, frame, guard cost,
  copy/fusion/forwarding count or semantic state change is allowed.

Extend the comparison decoder to validate both conditional encodings, using
decoded targets instead of fixed `branch + 6`/JML offsets. Audit the forwarding
instruction decoder, disassembler, all PC-indexed evidence and IRQ schedules.
Interrupt at newly selected branch boundaries and with comparison flags live;
retain seeded cycle schedules as well as instruction-address coverage.

## Validation, artifacts and commits

For each enabled slice run the relevant emitter unit tests and these affected
compiler integration targets:

```sh
cargo test --lib mir65816
cargo test --test mir65816_state_boundary --test mir65816_abi --test mir65816_contract --test mir65816_emission --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
```

The frozen state-boundary fixture will intentionally change. Retain its historical
version and describe each new snapshot as the slice's intentional code/offset
change, not a printer-only update. Review its metadata against the independent
inventory. No automatic accept-current-output path may replace that review.

Run focused native targets during development, then the full native workspace
through the [qualification runner](../tools/native65816-runtime-tests/README.md)
in debug and release before completing each behavior slice. This covers shared
emission consumers without running unrelated repository suites by default.

```sh
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

Use the [comparison workflow](../tools/compare65816/README.md) with separate
directories for each slice. Build the raw/optimized corpus with `--verify-crlf`,
execute its serialized artifacts in both hosts and incoming I states, run the
new delta checker and save listings, measurements, site coverage and provenance.
Retain the known optimized vbcc unlink vector-0 failure explicitly in both hosts
and I states; the optional comparison command reports failure after saving all
records. Any new failure remains a blocker.

Run affected Python decoder/checker tests. For new newline-sensitive fixture
handling, check LF and CRLF through the actual path; use an isolated CRLF checkout
for affected `include_str!` tests. Immutable trace checks must cover both host
builds and never supply CPU inputs or expected results.

Save per-slice baseline, inventory, delta and qualification records under new
`docs/benchmarks/` and `docs/abi/` names. Bind them to compiler/tool/fixture hashes,
VM base/patch, runner manifests and exact artifacts. Keep all historical records.
Update the emission contract, roadmap and results after each qualified slice.

Recommended commit sequence:

1. Freeze 3a inventory and independent expectations; enable checked MIR-entry
   width omission with tests, then commit qualification/results.
2. Commit any byte-identical edge-evidence refactor separately. Enable checked
   terminal fallthrough with tests, then commit its qualification/results.
3. Commit the typed conditional-site/finalization foundation with strict output
   equality. Enable bounded short dispatch with metadata/relocation tests, then
   commit its qualification/results.

Each behavior commit must have its focused checks; each completed slice must
have full applicable qualification before the next behavior starts. If an
inventory invalidates a forecast, explain and revise it before implementation.
Preserve unrelated local changes and stage only the completed slice's files.
