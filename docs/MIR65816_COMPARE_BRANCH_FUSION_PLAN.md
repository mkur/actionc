# Native 65816 compare-to-branch fusion implementation plan

Status: completed on 2026-09-21. Baseline/use proof: `e4dbdd1`; fused emission:
`4687170`; qualification and measured results are in the containing commit. See
[results](MIR65816_COMPARE_BRANCH_FUSION.md). The original plan below was prepared
against main `4d54b81`, following the completed
[native word comparison slice](MIR65816_WORD_COMPARISONS.md).

## Objective and scope

For an eligible final word Compare whose sole use is the immediately following
MIR Branch, branch directly on CMP's C/Z flags. Omit Boolean materialization,
the byte store/reload and the second conditional dispatch. Preserve the existing
true/false edge transfers, including parallel copies and mode restoration.

Keep this an emission-only combination of two adjacent MIR operations. Do not
rewrite NIR, introduce a new public MIR form, carry flags across other operations
or blocks, move source loads, or change liveness/allocation policy. Retain the
Boolean's allocated home and all frame/storage metadata even when this use no
longer writes it. An eliminated branch-only value has no observable Boolean
consumer; its reserved home must not be described as containing a runtime 0/1.
Stored, returned, passed or otherwise reused Booleans still materialize normally.

Preserve physical ABI v1, image v3, the o65 profile, guards, call barriers,
argument/result layouts and Exec816's pin. No signed-ordering expansion, byte or
wide comparison fusion, nonadjacent matching, logical-not/cast chasing, branch
relaxation, empty-edge cleanup, frame shrinking or general mode optimization
belongs to this slice. Both raw and optimized target output use the same rule.

## Current code and measured baseline

[`routine`](../src/mir65816/emit/select.rs) emits every block operation, requests
A16, then emits its terminator. `word_compare` currently loads two eligible word
operands, performs CMP, branches to materialize 0/1, and stores a Boolean byte.
The Branch terminator changes to A8, reloads that byte, restores A16 and uses BNE
to select an edge trampoline. `edge` then performs any parallel copies in A8,
restores A16 and jumps to the target block. Even empty edges retain that sequence.

The [post-comparison snapshot](benchmarks/65816-word-comparisons/after/tables.md)
is the immutable baseline. Its emitter revision is `51e5bc7`; `4d54b81` adds
qualification without changing emission. During planning, all 224 saved file
hashes across 56 builds and all 264 matching debug/release records were verified.
Current MIR inspection finds one final word Compare followed by its byte-Temp
Branch in each of maximum, loop rotation, sum loop, recursive sum, byte sum and
forward copy, in both target modes. Their branch edges currently have no args;
nonempty edges need separate regression coverage.

Representative optimized Action measurements include guards and RTL:

| Kernel / input | Code bytes | VM cycles | Stack peak below entry S | Stack byte reads / writes |
| --- | ---: | ---: | ---: | ---: |
| maximum(13,41) | 146 | 149 | 6 | 16 / 7 |
| sum loop(13) | 213 | 2,465 | 16 | 287 / 230 |
| recursive sum(13) | 247 | 3,819 | 190 | 268 / 250 |

In the [maximum listing](benchmarks/65816-word-comparisons/after/maximum.optimized.actionc.lst),
the sequence from the operand LDA through the second conditional dispatch is
40 bytes. Fusion makes it 10 bytes while retaining both edge trampolines. The
corresponding stack/immediate sequence is 41 → 11 bytes. Removing the Boolean
path saves 31 cycles for a true comparison and 32 for false, plus one stack byte
read and one write each time. These estimates retain existing REP/SEP and JML
instructions on the edges.

| Kernel / input | Estimated bytes after | Estimated VM cycles after | Estimated stack reads / writes after |
| --- | ---: | ---: | ---: |
| maximum(13,41), either mode | 116 | 117 | 15 / 6 |
| sum loop(13), optimized | 183 | 2,030 | 273 / 216 |
| sum loop(13), raw | 188 | 2,161 | Baseline minus 14 / 14 |
| recursive sum(13), optimized | 217 | 3,372 | 254 / 236 |

These are listing-derived forecasts, not measurements of implemented fusion.
Set regression ceilings of 125 bytes / 125 cycles for maximum, 195 / 2,100 for
optimized sum loop and 200 / 2,250 for raw sum loop. Keep their exact frame/stack
peaks and guard costs. All three representative optimized kernels already have
zero DP traffic; fusion must retain that.

## Eligibility: adjacency and a routine-wide use proof

Match only this shape after ordinary MIR verification and allocation:

```text
block operations: ... Compare { dest: t, width: 2, ... }  // final operation
terminator: Branch { condition: Temp(t, ONE), then_edge, else_edge }
```

Require the existing native word comparison eligibility: U8/U16 immediates or
exact two-byte stack temps/parameters, an exact one-byte stack result home,
unsigned Eq/Ne/Lt/Le/Gt/Ge or signed Eq/Ne. Keep complete source/destination
displacement checks, including actual `Builder::delta`, even though the result
home is no longer written. Missing IDs, bad widths/homes and invalid extents
remain errors. Legal unsupported forms retain materialized comparison/branch
emission without a partial prefix or changed label/mode state.

Adjacency alone is insufficient. The Boolean must have no use anywhere else in
the routine, including a successor block, another Branch condition, Return,
call argument/indirect target, store value, pointer base/index, or either edge's
arguments. Count the two edge argument lists separately from the condition:

```text
t = Compare(x, y)
Branch(t, then: B(t), else: C())  // must not fuse: B needs the Boolean byte
```

[`liveness.rs`](../src/mir65816/emit/liveness.rs) already exhaustively visits
operation inputs, including addresses and indirect calls. Its `Uses.inputs` is
a set and its terminator set merges condition and edge arguments; that set
cannot establish a sole condition use. Add a small private routine-wide query
beside that walker, reusing operation input enumeration. One sufficient proof is:

1. Count Branch-condition occurrences per TempId across all blocks.
2. Collect every other temp input into a disqualifying set: operation inputs,
   all edge arguments and return values, using exhaustive enum matches.
3. Admit a candidate only when its count is one and it is absent from that set,
   and its defining Compare is the final operation of that very block.

This does not require a new dataflow optimization. Include unreachable blocks
conservatively and keep existing definition/use validation. Do not count a
destination or block-parameter definition as a use. Build the facts once per
routine; do not repeatedly scan the whole routine for each candidate. Keep
interference construction, allocation order and emitted storage maps unchanged.

## Shared checked condition and branch emission

Factor only the existing word comparison preflight and predicate selection into
a private checked condition description. Share it between materialized and fused
emission so operand swaps, signedness and error handling cannot diverge. Keep
normal materialization byte-for-byte unchanged in isolated selector tests.

The description contains checked left/right operands, the validated Boolean
home and one true-condition opcode:

| Predicate | A / CMP operand | Branch to true |
| --- | --- | --- |
| Eq, signed or unsigned | left / right | BEQ |
| Ne, signed or unsigned | left / right | BNE |
| Unsigned Lt | left / right | BCC |
| Unsigned Ge | left / right | BCS |
| Unsigned Gt | right / left | BCC |
| Unsigned Le | right / left | BCS |

In the block emitter, emit the ordinary prefix operations once. Preflight the
final Compare/Branch pair at its actual emission state. If selected, emit:

```text
a16(); LDA checked_left; CMP checked_right
Code::branch(true_predicate, yes_edge)
edge(else_edge)
mark(yes_edge)
edge(then_edge)
```

Consume flags before any edge copy, load or helper can overwrite them. Keep
`Code::branch`'s inverse short branch over JML and its typed fixups. Use the
existing `edge` implementation for both successors, even when their targets are
the same: their argument values can differ. Do not jump directly to a successor
and skip its parallel copies. Preserve false-first layout and the local true
edge trampoline; no branch distance assumption or relaxation is needed.

The selected path must skip both ordinary Compare emission and the generic
Boolean Branch terminator. Otherwise emit both through their original paths.
Every successor still receives A16/X16 state. `Code::mark` still invalidates
local mode knowledge; retain the edge's explicit A8/A16 transitions at each
entry. Only the immediately adjacent pair shares flags. All earlier source
loads, calls, stores, casts and memory effects stay in place. Keep nonempty-edge,
backedge and live-value behavior independent of operand stack-slot overlap.

## Commit 1: baseline, use-proof and semantic coverage

Record baseline provenance and hashes under
`docs/benchmarks/65816-compare-branch/`, reusing intact
`target/word-comparisons-after`. If reconstruction is necessary, build the
compiler and runner in an isolated `4d54b81` checkout. Do not regenerate the
historical snapshot with the new compiler or mislabel a supplied historical
binary as the runner's current revision.

Add the private use-proof query and focused tests without changing emission.
Cover condition-only use, a second condition in another block, successor use,
condition-plus-edge-argument use (either/both edges), returned/stored/call/address
uses, repeated uses within one operation, and unreferenced block parameters.
An exhaustive operation match must force future MIR forms to describe inputs.

Add `tools/native65816-runtime-tests/tests/compare_branch.rs` with baseline
semantic probes. Use independently calculated host expectations and runtime
inputs, batching cases by distinct behavior rather than recompiling each pair:

| Coverage | Required behavior |
| --- | --- |
| Relations | All six unsigned and signed Eq/Ne branch outcomes across 0, 1, `$00FF`, `$0100`, `$7FFF`, `$8000`, `$FFFF`; explicit true/false word results distinguish branch inversion mistakes. Keep signed ordering and byte/wide fallbacks covered. |
| Consumers and adjacency | A branch-only Boolean; stored/returned/passed/reused Booleans; two conditions using one temp; and an intervening operation/call. Assert eligibility from actual MIR, since optimization may remove source-level intermediates. |
| Control flow | Nested IF, loops/backedges, recursion, equal successor targets with different arguments, and parallel edge copies that swap/rotate live values. Construct verified MIR probes where source lowering cannot retain a required shape. |
| Effects and ABI | Captured volatile/aliased words, bank-crossing loads, comparisons after direct/indirect full A/X/Y/P/DP clobbers, exact original byte traces, guards and both incoming I states. |

Baseline semantic tests must pass before enabling fusion. Verify LF/CRLF through
any new newline-sensitive parsing/instrumentation path. Keep the existing word
comparison tests as coverage for materialized Booleans.

## Commit 2: adjacent word comparison/branch fusion

Implement shared checked condition preparation and block-pair selection in
`select.rs`. Add a focused private `branch_tests.rs` or extend the current
comparison tests without broad refactoring. Verify all gates, predicate swaps,
result-home validation, nonzero delta, full-word/byte boundaries and nonmutating
fallback/errors. Check that neither the Compare nor Branch is emitted twice and
that a failed eligibility check does not omit an operation.

Execute serialized bytes for both outcomes. Decode from real instruction
boundaries to prove one word CMP and one conditional dispatch; the fused interval
has no Boolean load/store, DP scratch or pushes. Source word reads remain exact.
Measure edge copies separately: they retain their required reads/writes and may
clobber flags after the decision. Assert A16 on both edge entries and successor
blocks, unchanged allocation/maps/guards, and the new whole-kernel budgets.
No new instruction opcode or production disassembler support is needed.

Update the scalar-selection section of the
[emission contract](MIR65816_EMISSION_CONTRACT.md): routine-wide single-use proof,
adjacent-pair flag lifetime, retained reservation but omitted Boolean write,
edge-copy guarantees and fallback. Do not weaken the current materialized
Boolean guarantees. Run focused compiler and native checks before committing.

## Commit 3: interruption, relocation and measured qualification

Teach test-side decoding to distinguish materialized and fused comparison
windows. The existing [window recognizer](../tools/native65816-runtime-tests/tests/support/comparison.rs)
requires the materialization suffix, so it cannot recognize fusion. Extend the
generic preemption coverage assertions to require actual fused windows instead
of silently accepting their disappearance. Retain the materialized comparison
probe and its 96 task/PC sites where the selected code remains unchanged.

Add a targeted fused-branch probe in both task domains. Inject IRQ at source
loads/CMP, immediately after CMP with each true/false C/Z outcome, at the
conditional instruction/JML and on each selected edge before copies. Cover
equality and carry predicates, stack/immediate forms and nonempty edges. Check
exact branch-dependent outputs, A/P restoration, parallel-copy results and
guards. Run both seeded IRQ/NMI schedules and retain existing arithmetic,
returns, pointer and context qualification. Incoming-I variation alone is not
preemption testing.

Add relocated o65 branch probes at both placements. The existing comparison
probe stores Boolean values and must remain as materialization coverage; add
actual conditional consumers to exercise fused JML fixups and edge behavior.
Run full native qualification in debug and release, recording actual test counts,
source/tool/VM hashes and identical saved artifacts through `qualify.py`.

Rebuild the unchanged 14-pair / 66-vector corpus with LF/CRLF equivalence and run
both target modes in both host builds. Save `docs/benchmarks/65816-compare-branch/after/`
and a delta against the unchanged post-comparison snapshot. Retain raw/optimized
maximum and sum-loop final listings and all vector measurements. Require no
Action correctness, code-size or cycle regression, unchanged frames/peaks/guards,
and identical vbcc measurements. Keep the known optimized vbcc unlink failure
visible; run both host comparison commands even if the first fails.

### Exact stack-traffic accounting

Each executed fusion removes exactly one Boolean byte write and one reload;
word input reads, edge-copy traffic and all unrelated accesses stay unchanged.
Before enabling fusion, record these predicted execution counts for each mode
and vector, after confirming the candidate's sole-use proof:

| Corpus kernel | Expected executed fusions |
| --- | --- |
| maximum | 1 |
| loop rotation | 9: eight successful loop tests and the final exit test |
| sum loop, recursive sum, byte sum | n + 1 |
| forward copy | count + 1, including the zero-length case |
| Other kernels | 0 |

Extend [`delta.py`](../tools/compare65816/delta.py) with an optional
`--fused-branch-counts` table keyed by case/mode/compiler/vector and a positive
integer count. Derive the allowed read and write deltas as exactly `-count`;
unlisted records retain equality. Reject duplicates, missing/unused keys,
nonpositive/noninteger counts and external-compiler entries. Make this option
mutually exclusive with the existing positive-only `--stack-read-deltas` option;
preserve that option and the strict default. This baseline already contains the
previous full-word read increases, so do not apply those exceptions again.

Verify predicted counts against reached, decoded fused sequences. Do not infer
an allowance by subtracting observed totals after a failure. Preserve every
other comparison invariant and test strict-default failure, exact reductions,
one-sided/wrong reductions, extra writes, stale keys, correctness regressions
and unchanged vbcc records. Report read/write reductions explicitly rather than
claiming unchanged stack traffic. Zero DP traffic and unchanged physical stack
depth remain separate assertions.

Commit a results document and machine-readable qualification record, update the
two runner READMEs and this plan's status, and report measurements against the
forecasts. Frame shrinking, broader flag use and signed ordering remain separate
follow-up decisions.

## Validation commands for implementation

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission \
  --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -m unittest discover -s tools/compare65816 -p 'test_delta.py'

python3 tools/native65816-runtime-tests/qualify.py \
  --test compare_branch --test word_comparisons --test arithmetic \
  --test word_arithmetic --test word_returns --test execution --test interop \
  --test indirect --test memory --test stack_allocation --test stack_faults

# Final qualification after focused failures are resolved:
python3 tools/native65816-runtime-tests/qualify.py -- --nocapture
python3 tools/native65816-runtime-tests/qualify.py --release -- --nocapture
```

Follow the [comparison runner](../tools/compare65816/README.md) for building,
executing both host modes and reporting, using distinct before/after directories.
Native tests must use `qualify.py` with its pinned CPU timing correction. Run
matching decoder checks if test-side decoding changes and the corpus generator's
`--check` if fixtures change. Check changed-file formatting, documentation links
and `git diff --check`. No full root suite or NIR sweep is required while this
stays within native emission/use inspection; broaden if those contracts change.
