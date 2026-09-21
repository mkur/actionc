# Native 65816 direct single-word edge-copy implementation plan

Status: proposed on 2026-09-21 against main `bdb0f67`. This is the first slice of
the [code-quality plan](MIR65816_CODE_QUALITY_PLAN.md); no compiler implementation
is included in this planning commit.

## Objective and boundaries

For an already eligible word edge containing exactly one assignment, load its
source into A16 and store directly into the successor parameter's stack home.
Bypass the staging write and reload. Apply the same selection rule to raw and
optimized MIR.

Preserve physical ABI v1, image v3, the o65 profile, typed JML relocations,
argument/result placement, frame allocation, complete storage maps, stack guards,
interrupt reserves and Exec816's compiler pin. Keep the four-byte staging
reservation even when an edge no longer accesses it. This changes private stack
traffic, not source-memory access order or width.

Empty edges retain their recent cleanup. Multi-value word edges retain their
complete two-phase capture/assignment sequence, including unused parameters.
Mixed-width and otherwise unsupported edges retain bytewise fallback. Self-copies
still perform a load and store. Frame shrinking, copy coalescing, general copy
scheduling, accumulator forwarding, DP/register allocation, mode propagation and
branch relaxation are separate slices.

## Rechecked baseline and forecasts

Planning rechecked all 224 artifact hashes across 56 builds in
`target/empty-edges-after`, all 264 matching debug/release records, and the
committed [empty-edge snapshot](benchmarks/65816-empty-edges/after/tables.md).
The emitter revision is `3dd5cb2`, qualified at `192b6d8`; the subsequent roadmap
commit contains no compiler changes. Saved hashes and forecasts are in
[baseline.json](benchmarks/65816-single-word-edges/baseline.json).

Only optimized sum-loop and byte-sum contain single-word edges in the unchanged
14-pair / 66-vector corpus. Each has two static sites: immediate initialization
and a stack-source backedge. Optimized rotation has multi-word edges; raw corpus
cases have no nonempty edges. Constructed verified MIR is required to exercise
direct copies in raw mode.

The [expected counts](benchmarks/65816-single-word-edges/expected-copies.json)
are derived independently as `1 + n` from each loop's input vectors:

| Kernel | Vector inputs n | Expected direct copies |
| --- | --- | --- |
| Optimized sum_loop | 0, 1, 8, 13, 31 | 1, 2, 9, 14, 32 |
| Optimized byte_sum | 0, 1, 4, 8, 16 | 1, 2, 5, 9, 17 |

There are four static sites across two builds, ten selected vector records, and
92 executed direct copies per incoming interrupt-mask state. All other records
must select zero direct copies. Do not derive expected counts from the new
emitter or decoder.

Each selected edge removes four static bytes. Each execution removes two
instructions, ten qualified VM cycles, two stack-byte reads and two stack-byte
writes. Representative forecasts include entry guards and RTL:

| Optimized kernel / input | Bytes before / after | Cycles before / after | Stack reads before / after | Stack writes before / after | Unchanged stack peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| sum_loop(13) | 154 / 146 | 1,735 / 1,595 | 273 / 245 | 216 / 188 | 16 |
| byte_sum($12FFFC,16) | 252 / 244 | 4,646 / 4,476 | 624 / 590 | 523 / 489 | 22 |
| loop_rotation(13) | 186 / 186 | 1,314 / 1,314 | 201 / 201 | 178 / 178 | 26 |

These are predictions, not post-implementation measurements. Require exact
unchanged raw corpus artifacts and unchanged other optimized artifacts. Existing
fused-branch counts, total word-edge/word counts, DP traffic, source-memory traces,
frame maps and guard costs remain unchanged; relocated site addresses may move.

## Selection design

The change belongs in [`emit_word_edge`](../src/mir65816/emit/select.rs), after
the existing `word_edge` preflight has produced a complete checked `WordEdge`.
Keep the preflight and its diagnostic/fallback distinction intact:

- Resolve the target and check arity and exact two-byte argument/parameter widths.
  Do not use arithmetic's U8-to-word widening for edge arguments.
- Resolve source temps and authoritative mutable parameter homes. Sources are
  the existing supported U16 immediate or checked stack word; destination is a
  checked two-byte stack temporary.
- Validate both accessed bytes using the current transient S displacement.
- Continue validating the existing four-byte staging slot and its accessed word
  even though the selected direct path will not touch it. This keeps the current
  allocation contract and malformed-plan diagnostics unchanged.
- Preserve all-entry preflight for multi-value edges: an earlier unsupported
  operand must not hide a later malformed operand or emit a partial sequence.

After `a16()`, match exactly one checked move. Emit its existing source load,
then `STA destination,S`, followed by the existing typed JML. Otherwise execute
the unchanged two-phase word path. Keep this as a private selection decision;
no public MIR form, image metadata or CLI option is needed.

```asm
; Current stack-source copy      ; Direct stack-source copy
LDA source,S                    LDA source,S
STA staging,S                   STA destination,S
LDA staging,S                   JML successor
STA destination,S
JML successor
```

An immediate source uses `LDA #word` instead of `LDA source,S`. Known A16 emits
no mode instruction; known A8 or unknown local knowledge retains `REP #$20`.
With known A16, the complete stack-source transfer is eight bytes / 14 cycles,
and the immediate transfer is nine bytes / 12 cycles. REP adds two bytes and
three cycles in either case. Confirm these encodings and timings independently.

### Correctness argument

There is only one destination assignment. The complete source word is in A
before either destination byte is written, so no later source can be destroyed.
Exact self-copies remain valid; even physical overlap is safe for this one
load/store pair. Do not relax allocation rules to create overlapping homes.

At successor entry, A and N/Z match the source word just as after the old staging
reload. C/V and X/Y/S/D/DBR/I are preserved, with M restored to zero and the
existing X16 contract retained. All four staging bytes remain untouched by the
direct transfer. No DP access, helper, push or source-memory load is introduced.
Other live invocation values stay in their existing homes.

An interrupt between load and store now observes the only captured value in A.
The existing full CPU-state restoration contract must preserve it; task and IRQ
DP isolation remains unchanged. Ordinary calls do not occur inside the transfer,
and no value gains a register lifetime across a call.

## Regression and measurement support

The existing [`word_edge` decoder](../tools/native65816-runtime-tests/tests/support/word_edge.rs)
and bus-trace oracle assume every edge is staged. Update the test model to
distinguish a direct move from a staged move explicitly; an optional staging
location or private transfer-kind enum is sufficient. Preserve staged decoding
and its current rejection checks.

A bare `LDA; STA; JML` pattern is not proof of an edge. Ground direct decoding in
a test-only index of verified single-word MIR edges and their typed machine
fixups. Record routine-relative load/jump/target offsets and expected operands;
validate the full encoding, physical displacements, target and width at execution.
Do not count a staged copy's final assignment as a second direct edge.

Build the index from the prepared MIR and machine objects in focused fixtures.
For the external corpus, prepare each Action artifact once using its recorded
source, frontend mode and layout, require its generated image bytes to match the
saved artifact, and extract the index before dropping compiler objects. The
runner still executes the serialized artifact bytes. For o65 probes, carry
test-only routine-relative site offsets to the relocated routine addresses and
validate the resulting bytes and targets. This index must not change the public
image or o65 profile, nor serve as the expected result/count oracle.

Update the consumers in `word_edges.rs`, `support/comparison.rs`,
`preemption.rs`, `o65.rs` and `code_quality.rs`. Preserve semantic `word_edges`
and `edge_words` totals across both forms; add separate Action-only
`direct_word_edges` and `direct_word_edge_sites` metrics. Count once at the first
LDA, never again at REP, STA or JML. Keep vbcc measurement records unchanged.

For direct execution, expect source byte reads followed by destination byte
writes, or only destination writes for an immediate. Seed and verify all four
reserved staging bytes, surrounding canaries, other live homes and DP scratch.
Keep existing exact two-phase traces and upper-staging canaries for multi-word
edges.

## Regression matrix

| Area | Required evidence |
| --- | --- |
| Selection | Stack temp, U16 immediate, immutable incoming parameter and updated mutable parameter; known A8/A16/unknown mode; one typed JML; unchanged frame/maps. |
| Boundaries | Word displacement 254 accepted and 255 rejected, transient S movement and overflow; missing targets, homes or staging; wrong widths; errors leave code/fixups/mode unchanged. |
| Fallbacks | Empty edges unchanged; multi-word rotations, swaps, repeated sources and unused parameters still staged; byte/three-/four-byte and mixed edges unchanged; legal unsupported word sources retain fallback. |
| Execution | Goto, loop backedges, ordinary and fused branch arms, same target with different arguments, self-copy and independent live-ins. Test 0, 1, $00FF, $0100, $7FFF, $8000, $FFFF and distinct high-byte patterns. |
| Decoder | Truncation, wrong opcode/width/offset/target, out-of-range targets, missing or mismatched site evidence, ordinary load/store/jump sequences, and the suffix of staged copies are rejected as direct edges. |
| External effects | Reuse captured-word edge probes around volatile/aliased bank-crossing loads and direct/indirect clobbering callees; source access traces and call ordering stay exact. |
| Preemption | Explicit direct-copy probes in both task domains, including IRQ after LDA and before/after STA/JML, both conditional arms and live flags. Check complete restored CPU/frame state against an uninterrupted reference step. |
| Relocation | Serialize and relocate immediate/stack direct edges, both arms and a backedge at $100000 and $600000; execute both frontend modes and I states, checking full word results and relocated targets. |

Use constructed verifier-clean MIR in both frontend modes. The raw source corpus
does not by itself exercise this selection. Keep the existing multi-word IRQ
rotation coverage and seeded IRQ/NMI schedules; add direct-copy coverage rather
than replacing them. Verify independent ca65 encodings, including isolated
overlapping source/destination machine sequences, without weakening the allocator.

## Strict comparison accounting

The default [`delta.py`](../tools/compare65816/delta.py) requires identical stack
traffic. Its existing comparison-fusion exception removes one byte read/write
per fusion and is not appropriate for this change. Add a separate, mutually
exclusive `--direct-word-edge-counts` input using the committed expected-count
schema. For count C, require exactly:

- `stack_reads_after = stack_reads_before - 2*C`;
- `stack_writes_after = stack_writes_before - 2*C`;
- `instructions_after = instructions_before - 2*C`;
- `cycles_after = cycles_before - 10*C`;
- decoded direct copies equal C, with unchanged total word-edge/word and fusion
  counts, DP traffic, complete storage contracts, guard costs and correctness.

Unlisted records have C=0. Reject duplicate, stale, missing, negative, noninteger
or vbcc exceptions. Keep default and existing accounting modes strict and cover
the new mode with corruption tests. Allow newly added zero-valued direct metrics
when comparing unaffected Action records; do not hide changes in old fields.

Add a focused checker that verifies saved hashes and the complete instruction
streams: only the selected staging STA/LDA pairs disappear, with necessary
JSL/JML address relocation. Require four bytes saved per selected static site,
exact representative forecasts and byte-identical unaffected emitted artifacts.
Use file- and format-safe newline handling for generated text and verify LF/CRLF
through actual corpus builds.

## Implementation sequence and checks

1. Commit baseline semantic probes, independent encodings and edge-site indexing
   that can run against the current staged emitter. Preserve the baseline and
   independently declared counts in this plan. If build artifacts are missing,
   reconstruct them from an isolated `192b6d8` checkout.
2. Implement the private single-move selection, direct decoder/oracle and unit
   coverage together. Update the nonempty-edge paragraph of the
   [emission contract](MIR65816_EMISSION_CONTRACT.md) to distinguish single-word
   direct copies from multi-word staged copies. Commit after focused checks.
3. Complete direct-copy interruption/relocation coverage, strict count accounting
   and corpus measurements. Save source/tool/artifact hashes and results, update
   runner/tool documentation, and commit qualification. Preserve unrelated local
   changes in every commit.

Compiler checks:

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission \
  --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
```

Use the qualified VM runner for focused word/empty-edge, comparison, effects,
call/interop, allocation and guard targets affected by the change. Finish with
the complete native suite in both host modes, including new tests:

```sh
python3 tools/native65816-runtime-tests/qualify.py -- --nocapture
python3 tools/native65816-runtime-tests/qualify.py --release -- --nocapture
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/compare65816 -p 'test_*.py'
cargo build --release --bin actionc-65816
python3 tools/compare65816/build.py --output target/single-word-edges-after --verify-crlf
```

Run both external `code_quality` host commands from the
[comparison instructions](../tools/compare65816/README.md), pointing the manifest
and results at `target/single-word-edges-after`. Both must save all 264 records
(528 executions per host including paired I states), retaining the existing
optimized vbcc unlink failure. Do not treat report generation as proof that this
external failure passed. Generate the new snapshot and delta under
`docs/benchmarks/65816-single-word-edges` without replacing the baseline files.

This is an emission and test-tooling change. No semantic/NIR/allocator contract
change is planned, so a full root suite or NIR sweep is not required unless the
implementation expands into those boundaries. Check selected Rust formatting,
documentation links and diffs. Acceptance requires exact forecasts, unchanged
ABI/guards/maps, complete direct and staged execution coverage, matching host
results, and preserved local files. Board and Exec816 loader qualification remain
separate integration work.
