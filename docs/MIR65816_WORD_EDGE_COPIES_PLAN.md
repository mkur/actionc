# Native 65816 word parallel edge-copy implementation plan

Status: proposed on 2026-09-21 against main `01ea393`, following
[compare-to-branch fusion](MIR65816_COMPARE_BRANCH_FUSION.md). This document
plans the next slice; it does not change compiler behavior.

## Objective and scope

Select A16 LDA/STA for a nonempty control-flow edge when every argument and
block parameter is exactly two bytes and every source/destination has a checked
word representation. Keep the existing two-phase parallel-copy algorithm:
capture every source in its staging slot, then assign every destination.
Apply the same rule to raw and optimized MIR.

This is an emission change in [`select.rs`](../src/mir65816/emit/select.rs).
Preserve physical ABI v1, image v3, the o65 profile, typed JML fixups, guards,
frame allocation, storage maps, stack depth and Exec816's pin. Staging slots
remain four bytes each, with only their existing low two bytes used for words.
Preserve source-memory operations and call barriers; no values stay in CPU
registers or DP across edges, blocks or calls.

Keep empty edges and mixed-width edges on their current paths. Do not add
partial word selection within a mixed edge, staging-slot shrinking, parallel-copy
scheduling without staging, self-copy elimination, load forwarding, branch
relaxation, general mode propagation, native argument/prologue copies, signed
comparison expansion, or NIR/MIR representation changes. These are separate
slices with different proofs and measurement expectations.

## Current implementation and verified baseline

`Builder::edge` resolves the successor and checks arity, selects A8, then copies
each argument byte into `frame.edge_copies[n]`. A second loop copies those bytes
into the successor's parameter temps. It restores A16 and emits JML. The loops
already implement the correct parallel assignment semantics for swaps, cycles,
source/destination aliasing and repeated sources.

[`AllocatedFrame`](../src/mir65816/emit/allocation.rs) reserves one four-byte
staging slot per maximum block-parameter position. Its verifier proves those
slots disjoint from all temporary homes, frame objects and other staging slots.
The allocator also protects distinct successor parameter homes, including unused
parameters. Retain that proof and allocation policy without modification.

The immutable baseline is
[`65816-compare-branch/after`](benchmarks/65816-compare-branch/after/tables.md),
emitted by `4687170` and qualified at `01ea393`. Planning rechecked all 224 saved
file hashes across 56 builds, plus all 264 matching debug/release records in
`target/compare-branch-after`. The existing qualification has 67 native tests
per host build and 192 identical saved artifacts.

Current MIR inspection of the unchanged 14-kernel corpus finds:

| Mode / kernel | Nonempty edges | Arguments per edge | Source forms |
| --- | ---: | ---: | --- |
| Optimized loop rotation | 2 | 3 | Entry: two word temps and U16(0); backedge: three word temps |
| Optimized sum loop | 2 | 1 | Entry: U16(0); backedge: one word temp |
| Optimized byte sum | 2 | 1 | Entry: U16(0); backedge: one word temp |
| Other optimized kernels | 0 | 0 | None |
| All raw corpus kernels | 0 | 0 | None |

The six nonempty edges satisfy the proposed logical width/source gate. Physical
selection must still preflight their allocated homes at emission time. Raw
semantic qualification requires constructed verified MIR with nonempty edges;
the existing raw benchmark corpus alone cannot exercise this optimization.

Representative baseline measurements include entry guards and RTL:

| Optimized kernel / input | Code bytes | VM cycles | Stack peak | Stack byte reads / writes |
| --- | ---: | ---: | ---: | ---: |
| sum loop(13) | 183 | 2,030 | 16 | 273 / 216 |
| loop rotation(13) | 247 | 1,720 | 26 | 201 / 178 |
| byte sum($12FFFC,16) | 281 | 5,004 | 22 | 624 / 523 |

The [sum-loop listing](benchmarks/65816-compare-branch/after/sum_loop.optimized.actionc.lst)
contains a bytewise immediate initialization edge and a bytewise stack-to-stack
backedge. On entry with A16 already known, their copy bodies use 20 bytes each;
word selection reduces them to nine and eight bytes respectively. JML is
unchanged. The initial edge saves 18 cycles and each backedge saves 20.

| Kernel / input | Forecast bytes | Forecast cycles | Unchanged stack peak |
| --- | ---: | ---: | ---: |
| sum loop(13), optimized | 160 | 1,752 | 16 |
| loop rotation(13), optimized | 192 | 1,290 | 26 |
| byte sum($12FFFC,16), optimized | 258 | 4,666 | 22 |
| sum loop(13), raw | 188 | 2,161 | 14 |
| maximum(13,41), either mode | 116 | 117 | 6 |

These are listing-derived forecasts, not post-implementation measurements. For
loop rotation, entry saves 27 bytes / 46 cycles and its eight three-word
backedges save 28 static bytes / 48 cycles each execution. Byte sum has the same
copy shapes as sum loop, with 16 backedges. Set initial regression ceilings of
165 bytes / 1,800 cycles for optimized sum loop, 200 / 1,350 for loop rotation
and 265 / 4,750 for byte sum. Require exact unchanged raw corpus code and
measurements and unchanged unaffected optimized kernels. Relocated absolute
addresses in general fixtures may change; compare their behavior and contracts.

## Checked selection

Resolve and validate the edge target and argument count before selection.
Attempt a private whole-edge preflight only for a nonempty edge whose block
parameters all have width two. Represent successful preflight with a target
Label and an ordered list of checked source, staging displacement and destination
displacement. Do not allocate another frame area or expose a public IR form.

For each argument/parameter pair:

1. Preserve the existing exact argument-width check. `value_width(value)` must
   equal the parameter width; a U8 argument to a word parameter is an error,
   not an implicit zero-extension. The arithmetic helper `word_operand` accepts
   U8, so its eligibility is broader than this edge contract.
2. Use checked `word_operand` sources: U16 constants, exact two-byte stack temps,
   or physical two-byte parameters. Resolve mutable parameters through their
   authoritative frame object rather than their original incoming argument.
3. Require an exact two-byte stack destination for the block-parameter TempId.
   Unknown IDs, inconsistent home widths and invalid locations remain errors.
4. Obtain the existing staging slot with checked indexing. Missing slots and
   violations of the current four-byte staging-slot contract are errors. Use
   `word_displacement` for the two accessed bytes of source, staging and
   destination, including actual `Builder::delta`; do not access staging bytes
   two and three or mistake four-byte capacity for a four-byte copy.
5. Validate the target label and every move before mutating code, fixups, labels
   or local mode knowledge. A legal unsupported source/home makes the whole
   edge fall back. Still check later entries in an otherwise all-word candidate
   so an early unsupported operand does not hide a malformed later operand.

Legal unsupported word forms, such as a supported bytewise null/symbolic value
or DP home, retain the existing bytewise path. Empty/mixed/byte/wide edges also
retain that path. Do not introduce new acceptance of malformed MIR or weaken
allocation verification. Preserve ordinary diagnostics when the generic path
rejects an unsupported form. Preflight errors and fallback must emit no partial
word-copy prefix.

## Emission and invariants

After complete successful preflight:

```text
code.a16()
for each move in original edge argument order:
    LDA word_source
    STA staging_word
for each move in original successor parameter order:
    LDA staging_word
    STA destination_word
JML original_target
```

Every source is captured before any destination is written. Keep repeated reads,
self-copies and unused destination writes; removing them would change this
slice's traffic and allocation assumptions. Sources and destinations may share
physical temp bytes across the edge. The disjoint staging region is what makes
that safe; never turn the two loops into one interleaved assignment loop.

Use `Code::a16()` rather than assuming mode from the preceding block. Ordinary
terminators already restore A16, while `Code::mark` invalidates local knowledge
at a fused comparison's true-edge label. A word edge at that label must emit REP
when needed. Preserve X16, I, D, DBR and S; A and N/Z are scratch during copies.
The comparison decision has already consumed its flags before either edge runs.
Keep current JML ownership and fixups, emit exactly one transfer, and do not
alter mode state for fallback/empty edges.

This path introduces no helper call, DP access, push, memory reload through a
pointer or wider external/volatile access. Original loads and stores remain
separate operations. A source word captured from volatile or aliased memory is
private at the edge; do not consult source-level alias facts or move that load
across a call. Invocation-owned staging stays live across IRQ/task switches;
preemption must restore live A/P and retain partially completed staging and
assignment phases.

Each stack source contributes two source reads, two staging writes, two staging
reads and two destination writes. An immediate omits only the source-memory
reads, exactly as before. Byte counts and accessed private addresses remain the
same. Read/write interleaving changes within a word: word LDA reads both bytes
before word STA writes them, whereas the old path interleaves low-byte load/store
and high-byte load/store. Test the new ordered trace and two-phase invariant;
do not demand identical interleaving or claim an atomic shared-memory update.

## Commit 1: baseline and semantic probes

Record baseline provenance and hashes under `docs/benchmarks/65816-word-edges/`,
reusing intact `target/compare-branch-after`. If reconstruction is necessary,
use the compiler and build runner from an isolated `01ea393` checkout. Do not
regenerate the historical snapshot with the new emitter.

Add `tools/native65816-runtime-tests/tests/word_edges.rs`. Reuse a small verified
MIR fixture builder from the [existing edge tests](../tools/native65816-runtime-tests/tests/compare_branch.rs)
if sharing is useful; avoid unrelated harness refactoring. Prove the actual
nonempty edge shape and widths after each frontend mode. Baseline tests must
execute serialized images successfully in debug and release before changing
selection.

Cover one and several word parameters; stack/U16/parameter sources; mutable
parameter homes; repeated sources; swaps and three-value rotations; self edges,
loop backedges and values live into successors outside their parameter list;
unused parameters; and the same successor reached with different arguments.
Exercise Goto, ordinary Boolean Branch and fused Compare/Branch paths. Include
mixed BYTE/CARD/24-bit/32-bit edges as fallback coverage and both true/false arms.

Check full word patterns at 0, 1, `$00FF`, `$0100`, `$7FFF`, `$8000`, `$FFFF`,
independent host results, stack/domain guards and both incoming I states. Include
captured volatile/aliased bank-crossing inputs and live words across direct and
indirect clobbering calls; preserve original external byte traces. Verify both
LF and CRLF through any new newline-sensitive fixture instrumentation.

Predeclare expected benchmark transfers before enabling selection: optimized
sum loop and byte sum execute one initialization plus n backedges; loop rotation
executes one initialization plus eight backedges, copying three words each time.
Other corpus cases and every raw case select zero word edges. Confirm those
counts against reached machine instructions during final qualification.

## Commit 2: word selection, decoding and focused execution

Add private checked word-edge preparation and emission in `select.rs`, with
focused `edge_tests.rs` coverage. Test A16 already known, A8 and unknown incoming
mode knowledge; complete selected bytes and the existing target fixup; and
byte-identical empty/mixed/unsupported fallback. Check exact argument widths,
bad later sources/destinations, missing/invalid staging slots and target labels,
nonzero delta, arithmetic overflow and access endpoints. Word displacement 254
is valid and 255 is not; exercise the last accessed staging byte independently
of its untouched reserved bytes. Keep malformed/fallback preflight nonmutating.

Execute serialized word edges and decode actual instruction boundaries. Require
that all staging writes precede any destination write, source/staging reads are
exact, each destination receives both bytes, and the upper two staging bytes
retain canaries. Verify no DP traffic or pushes, unchanged frame/maps/guards,
and A16/X16 at each successor. Check swapping homes with different high bytes,
not only values below 256. Add independent ca65 encodings for representative
stack/immediate word-copy sequences.

Update the test-side [comparison window decoder](../tools/native65816-runtime-tests/tests/support/comparison.rs)
in the same commit: its present edge recognizer requires SEP, byte pairs, REP
and JML, and would otherwise stop recognizing comparisons with word edges.
Recognize both complete byte and word edge forms, with or without the word
path's initial REP, using the A16 state at dispatch and label boundaries.
Retain relocated-target checks and routine bounds; validate the staging/assignment
shape, not just a run of arbitrary load/store opcodes. Add positive, truncated,
wrong-mode and corrupt-edge decoder cases. Expose/reuse a small edge-window
recognizer for standalone Goto edges if necessary; test-only decoding must not
become a production optimization dependency.

Keep fusion recognition/counts, compare-branch traffic checks and targeted
preemption assertions meaningful. A shorter word edge legitimately reduces its
instruction-site count; require all newly decoded sites instead of preserving
the historical 152-site count or accepting missing windows. Retain all six fused
windows and 24 post-CMP truth/task combinations. Update the
[emission contract](MIR65816_EMISSION_CONTRACT.md) to specify two-phase word copies
and A16 successor guarantees; replace the claim that every edge requires A8/A16
transitions. Run focused compiler/native checks before committing.

## Commit 3: interruption, relocation and measured qualification

Extend the targeted context fixture to use several word arguments with cyclic
source/destination overlap in both task domains. Arm IRQ at every reached word
load/store, the staging-to-assignment boundary, mode setup and final JML. Check
full restored registers, full word outputs, staging state and guards. Use the
existing independent CPU-step approach: IRQ armed at a boundary is sampled at
instruction completion, so expected restored state is the state after that
instruction. Retain both CMP flag outcomes in both tasks, byte/mixed-edge probes,
the materialized-comparison probe and both seeded IRQ/NMI schedules.

Add serialized o65 nonempty word-edge execution at both existing placements,
covering Goto/backedges and both conditional arms. The current o65 fused-branch
probe has empty edges and is insufficient for this feature. Build verified MIR
when needed, discard compiler objects before loading, and check relocated JMLs,
parallel-copy results and unchanged ABI/guards. Save bytes, placements, metrics
and decoded coverage with the qualification artifacts.

Run full native qualification in debug and release with `qualify.py`, recording
actual test/site counts and identical saved artifacts. Rebuild the unchanged
14-pair / 66-vector corpus with LF/CRLF equivalence and execute both host modes,
even when the first reports the known optimized vbcc unlink failure.

Save `docs/benchmarks/65816-word-edges/after/` and a delta against the unchanged
fusion snapshot. Use `delta.py` with its **strict default**: all stack byte reads
and writes must match. Do not apply `--fused-branch-counts` or old read exceptions;
this baseline already includes those earlier changes. Also explicitly assert
unchanged DP reads/writes/touched offsets and per-vector `fused_branches` counts.
Absolute PCs in `fused_branch_sites` may move. Record independently decoded word
edge execution counts separately; do not infer selection from cycle reductions.

Require no Action correctness/size/cycle regression, identical complete routine
storage contracts and guard costs, unchanged raw corpus output, and identical
vbcc records. Retain raw/optimized sum-loop listings; add loop-rotation and
byte-sum listings to this snapshot to show all affected kernels. Explain any
intentional test expectation changes as emission-shape/coverage updates, not IR
contract changes. Commit a results document, machine-readable qualification
record, runner README updates and this plan's completed status.

## Implementation validation commands

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission \
  --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -m unittest discover -s tools -p 'test_disassemble65816.py'
python3 -m unittest discover -s tools/compare65816 -p 'test_delta.py'

python3 tools/native65816-runtime-tests/qualify.py \
  --test word_edges --test compare_branch --test word_comparisons \
  --test word_arithmetic --test word_returns --test execution --test memory \
  --test interop --test indirect --test stack_allocation --test stack_faults
python3 tools/native65816-runtime-tests/qualify.py --test preemption --test o65

# Final qualification after focused failures are resolved:
python3 tools/native65816-runtime-tests/qualify.py -- --nocapture
python3 tools/native65816-runtime-tests/qualify.py --release -- --nocapture

cargo build --release --bin actionc-65816
python3 tools/compare65816/build.py --output target/word-edges-after --verify-crlf
# Run both comparison host commands from the runner README with word-edges-after paths.
python3 tools/compare65816/report.py \
  --input target/word-edges-after --output docs/benchmarks/65816-word-edges/after
python3 tools/compare65816/delta.py target/compare-branch-after target/word-edges-after \
  --output docs/benchmarks/65816-word-edges --title 'Native word edge copies: before / after'
```

Follow the [native runner](../tools/native65816-runtime-tests/README.md) for the
pinned VM timing correction and the [comparison runner](../tools/compare65816/README.md)
for complete debug/release corpus execution. Run the corpus generator's `--check`
if fixtures change. Check changed-file Rust formatting, documentation links and
`git diff --check`. No full root suite or NIR sweep is required while changes
stay within emission and its test tooling; broaden if NIR/semantic contracts
change. Preserve unrelated local changes and commit each completed slice.
