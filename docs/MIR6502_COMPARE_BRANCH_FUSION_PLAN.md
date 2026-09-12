# MIR6502 comparison-to-branch fusion

Status: implemented, 2026-09-12. The baseline is `4b9147b`. The analyzed
pre-home rewrite, shared definition/use and machine-state proofs, configuration
gate, optimization reporting, and focused compiler/VM coverage are delivered.
Validation results and the measured inliner layout tradeoff are recorded below.

## Outcome and scope

Let the existing MIR6502 branch selectors consume a comparison directly when
its result reaches the branch through private BYTE copies. The first delivery
removes the unnecessary Boolean construction in Mandelbrot's radius check and
in generic programs with the same typed MIR shape.

This is a MIR6502 optimization. It changes neither Action! comparison semantics
nor the SemIR/NIR contract. Numeric comparisons still produce exactly BYTE 0
or 1 when a program stores, returns, passes, or otherwise uses their result.

Use the existing comparison and branch forms, shared analyses, and rewrite
driver. Do not introduce an emitter peephole, a new wide-comparison IR form,
or a source/library-specific matcher.

## Confirmed cause

This small program reproduces the problem on baseline `4b9147b`:

```action
LONGCARD input=$6E0
BYTE result=$600
PROC Main()
IF input >= LONGCARD($04000000) THEN
  result=17
ELSE
  result=29
FI
DO OD
RETURN
```

The relevant raw MIR is:

```text
v6 = cmp.b v5 ge #$04
v2 =.b v6
branch bool v2 ? b1 : b2
```

`lower/wide.rs` already narrows this aligned 32-bit comparison to the
significant byte. Its `Builder::lower_op` then copies the generated comparison
result into the original NIR destination. `expand_compare_branch_consumers`
does not recognize the comparison through that copy.

`expand_compare_value_consumers` consequently creates true/false blocks and a
Boolean merge. Later copy propagation cannot prevent that earlier expansion.
The resulting listing contains:

```asm
CMP #$04
BCS make_true
LDA #$00
JMP test_result
make_true:
LDA #$01
test_result:
CMP #$00
BNE then_body
```

The same construction appears in the saved baseline Mandelbrot listing,
`build/multiply-kernel/final.lst`. The independently compiled minimal probe,
raw/materialized MIR, and listing are under ignored `build/compare-branch-plan/`.
The source above makes the diagnosis reproducible without those local files.

## Implementation

### 1. Canonicalize comparison-result copies before branch selection

Recognize a block suffix consisting of one `MirOp::Compare` followed by one or
more contiguous BYTE `MirOp::Move` operations between full virtual temps. The
last destination must be the block's `MirCond::BoolValue` branch condition.

Rewrite the example to:

```text
v2 = cmp.b v5 ge #$04
branch bool v2 ? b1 : b2
```

Retarget the comparison to the final copy destination and remove the copies.
Keep the final result identity, comparison operands, predicate, signedness,
width, and terminator unchanged. This permits an ordinary operation rewrite;
it does not require a new CFG transformation or branch polarity calculation.

Support a contiguous copy chain by proving every link. Stop at the first
non-copy operation; do not follow aliases across blocks, loads, stores, casts,
Boolean arithmetic, or effect barriers. Accept only comparison representations
already supported by the existing branch selector: unsigned BYTE and signed
or unsigned word comparisons. Signed wide comparisons narrowed to a BYTE use
the existing sign-bit bias and unsigned comparison.

Place the analyzed rewrite in `run_prehome_canonicalization_group`, before
the existing compare-producer rewrites and branch/value expansion. Running it
early also exposes the final condition identity to existing producer selectors.
Gate it with `enable_peepholes`; do not move the broader late copy-propagation
pipeline earlier as part of this change.

### 2. Prove legality using the shared routine snapshot

All of these checks are mandatory in the first implementation:

- Every source has the exact reaching definition expected by the chain.
  Prove dominance and availability; MIR is not assumed to be globally SSA.
- Every removed intermediate definition is used only by the next copy.
  Account for full-temp and byte-lane uses throughout the routine, including
  successor blocks, loop backedges, and edge arguments.
- The final destination is used only as this branch's condition. Reject
  additional numeric uses, including uses on either outgoing edge. Moving its
  definition earlier within the suffix must not capture an older definition's
  uses or change replacement-operand dependencies.
- Sources and destinations are private virtual BYTE results. Reject physical
  registers, memory homes, lane projections, width-changing copies, self-copy
  cycles, ambiguous definitions, and unsupported comparison forms.
- The suffix contains only the comparison and copies. Operand evaluation and
  the comparison remain at their original positions. No call, machine block,
  volatile access, absolute access, or pointer write is crossed or removed.
- Initially require both branch edges to have no arguments. Existing branch
  helpers sometimes reconstruct plain edges from target IDs; do not expose an
  argument-carrying edge to that path without a separate preservation proof.
- Preserve machine effects and reject cases whose explicit register/flag uses
  cannot satisfy the existing rewrite and branch-selection contracts.

Use `PreHomeAnalysisSnapshot`, `PreHomeRewriteContext`, exact definition sites,
and the current transaction validator. If one missing query is needed for
"this definition has precisely these uses," add it to the shared context with
focused tests. Do not build a second suffix-only liveness analysis or weaken
the validator to accept the rewrite.

Apply through `MirPreHomeRewriteDriver` and `MirRewritePlan`, retaining
`MirEffectDelta::Unchanged` and the existing invalidation policy. Rebuild
analyses between rounds. The decreasing number of eligible copy operations
provides a termination measure; the rewrite must be idempotent.

### 3. Reuse the existing flag and branch lowering

After canonicalization, existing BYTE/word branch selection handles the
comparison. BYTE equality uses Z; unsigned ordering uses C, with the existing
two-flag or checked-threshold forms for inclusive predicates. Signed word
ordering must keep the existing signed lowering, including overflow handling.
Never replace signed ordering with a bare carry test.

Keep `compare_branch_plan` as the owner of threshold adjustment. In particular,
`<= $FF` and `> $FF` must not become comparisons with an overflowed immediate.
Keep the existing operand reversal and significant-lane rules unchanged.

If a numeric use or another proof blocker remains, keep the current Boolean
materialization path. Correct 0/1 values and preserved effects take precedence
over removing the comparison-to-zero sequence.

The plan does not require changing `lower/wide.rs`: recognizing the copy in
MIR also covers copies introduced by other lowering and inlining paths.
General 32-bit comparisons that remain multi-comparison AND/OR graphs are
regression cases here, not a new graph-fusion project. Cross-block conditions,
REAL comparisons, Boolean negation chains, and edge-argument support are
separate extensions.

## Implementation locations

| Location | Work |
| --- | --- |
| `src/mir6502/materialize/compare_branch.rs` | Recognize the comparison/copy suffix and construct its replacement. |
| `src/mir6502/rewrite/pilots.rs` | Discover candidates and obtain shared definition/use proofs. |
| `src/mir6502/rewrite/context.rs` | Add a narrowly scoped reusable proof query only if existing queries are insufficient. |
| `src/mir6502/materialize.rs` | Schedule the analyzed rewrite before comparison expansion and record its outcome. |
| `src/mir6502/materialize/tests.rs`, rewrite tests | Exercise candidate legality, transaction validation, phase ordering, and fallback. |
| `src/mir6502/mod.rs`, `fixtures/mir6502/` | Protect generated branch structure through compiler integration tests and focused fixtures. |
| `tools/vm-runtime-tests/tests/wide_integers.rs`, `comparison_values.rs` | Extend independent execution and observable-access coverage where needed. |

Use existing optimization reporting for candidates, applied rewrites, and
proof blockers. An applied counter such as `compare-branch-copy-elided` should
count removed copies consistently. Structural tests must confirm that branch
fusion actually follows; an incremented counter alone is insufficient.

Update `MIR6502_PSEUDO_MACHINE_CONTRACT.md` with the supported suffix, required
use proofs, retained numeric-value behavior, and conservative fallbacks when
the implementation lands. No new executable NIR form is expected.

## Delivery sequence

### Slice 1: freeze the contract and baseline

Turn the minimal reproduction into a generic integration fixture. Add direct
MIR cases for one copy, several copies, direct comparisons without copies,
and the mandatory rejection cases below. Record the current materialized
shape and emitted size/cycles before enabling the rewrite.

Existing runtime tests remain the arithmetic oracle. Do not replace the
Mandelbrot expression, change its threshold, or assert the inefficient
instruction sequence as a permanent fixture contract.

### Slice 2: implement the complete bounded rewrite

Deliver the matcher, proof checks, driver integration, reporting, and focused
positive/negative tests together. Use existing branch lowering after the
rewrite. Verify both default and optimized MIR configurations, plus a
peepholes-disabled control. Keep classic backends unchanged.

Review materialized snapshot differences individually. Expected differences
are optimization changes: fewer copies and Boolean merge blocks, with the
same source branches and effects. Raw NIR and raw MIR should remain unchanged
for existing fixtures because this is a materialization optimization.

### Slice 3: execution validation and measured acceptance

Run the relevant execution matrices and full checks, then measure the same
Mandelbrot recurrence before/after at identical origins and with identical
inliner settings. Record actual byte/cycle differences and any declined
sites. Finish by updating this plan's status and the MIR contract.

## Test matrix

| Area | Required cases |
| --- | --- |
| Predicates and widths | All six predicates for BYTE, CARD, INT, LONGCARD, and LONGINT source cases; both operand orders and both branch outcomes. Require fusion only where the supported MIR suffix is produced. |
| Boundaries | Zero, one, byte/word maxima, signed minimum/maximum, opposite signs, equal operands, and neighboring values around aligned 32-bit thresholds. Include non-aligned thresholds and wide equality as unchanged semantic controls. |
| Copy proofs | One/multiple copies; earlier definitions of the same temp; extra intermediate/final uses; full-temp and lane uses; successor/backedge uses; edge arguments; stale transaction generations; deterministic fixed-point behavior. |
| Numeric results | Store, return, call argument, arithmetic use, and a shared condition/value use. Results remain canonical 0/1 and their definitions survive. |
| Effects | Calls before operand capture execute once in order; calls/stores/machine blocks between comparison and branch prevent this suffix rewrite. Volatile wide input reads all four bytes in order, even when only the highest byte decides the branch. |
| Branch machinery | Inclusive predicates at maximum bounds, reversed operands, signed overflow cases, true/false fallthrough layouts, and a distant target exercising existing branch-distance handling. |
| Composition | Branches in loops, inlined wrappers, and ordinary helper callers. Retain existing multi-comparison wide predicates and short-circuit semantics. |

Reuse `aligned_wide_comparisons_preserve_all_predicates_and_operand_orders`
and `aligned_wide_comparison_preserves_every_volatile_input_byte`; extend them
only for missing coverage. Use bus access events for observable reads/writes,
not final RAM contents alone. Numeric comparison-value extensions run only in
the modern profiles that support them; source-compatible branch cases retain
the existing six mode/runtime combinations.

Focused shape assertions should check the predicate's CFG and def/use chain,
not ban all `LDA #0/#1` or `CMP #0` instructions from a routine. Real program
results may legitimately need those instructions. Require the minimal probe
to branch directly on its comparison without a temporary Boolean diamond.

## Validation and performance gates

During implementation, run focused matcher/rewrite tests first, then the
affected VM tests (`wide_integers`, `comparison_values`, and
`oscar64_mandelbrot`). Before completion run:

```sh
cargo test --locked nir_fixtures_match_snapshots
cargo run --locked --bin actionc-nir-sweep -- fixtures/nir
cargo test --locked mir6502_fixtures_match_snapshots
cargo run --locked --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test --locked
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml
```

Confirm the same source and numeric-value behavior with cartridge and
standalone runtimes. Include the complete Atari and VBXE Mandelbrot image
oracles already present in `oscar64_mandelbrot.rs`. Do not repeat exhaustive
multiply-helper sweeps unless implementation changes unexpectedly reach the
helper or arithmetic algorithms.

For performance, use the current multiplication kernel and INLINE behavior on
both sides. The earlier pre-inlining Oscar64 comparison is not the control.
Record compiler revisions/configurations, origins, emitted routine/program
bytes, both branch-path costs in focused probes, and recurrence cycles for the
640-point and original 16,000-point grids. Check every escape count against
the independent oracle and check listings against emitted bytes. Keep generated
artifacts under ignored `build/compare-branch-fusion/`.

Acceptance requires removal of the redundant Boolean path in the minimal
probe and both Mandelbrot renderers, no new helper or scratch ABI requirement,
and measured improvement without an unexplained size/cycle regression in the
affected probes. Report branch-layout/page-crossing effects rather than
promising a fixed saving from instruction counts. Preserve the existing
inliner legality/cost checks if changed layouts alter its selections.

## Reproduce the inspection

Save the minimal source above as `build/compare-branch-plan/wide-branch.act`.
The following commands use the inspection CLI's profile/backend options:

```sh
cargo run --quiet --bin actionc-emit -- --profile modern --backend mir6502 --runtime standalone --emit-mir6502 build/compare-branch-plan/wide-branch.act
cargo run --quiet --bin actionc-emit -- --profile modern --backend mir6502 --runtime standalone --emit-materialized-mir6502 build/compare-branch-plan/wide-branch.act
cargo run --quiet --bin actionc-emit -- --profile modern --backend mir6502 --runtime standalone --emit-listing build/compare-branch-plan/wide-branch.act
```

## Delivered validation and measurements

The new raw MIR fixture intentionally retains `Compare -> Move -> Branch`:
the improvement belongs to materialization. Existing NIR and raw MIR snapshots
are unchanged. Nine focused compiler tests cover supported predicates and copy
chains, idempotence, extra full/lane uses, successor/backedge uses, edge
arguments, reused definitions, operand dependencies, effects, unsupported
copy forms, live machine state, stale plans, configuration, and final branch
selection with both runtimes. The shared flow proof conservatively requires
one definition per temp ID throughout the routine.

Two new VM tests protect shared numeric/branch results, returned and passed
Booleans, call order/count, and distant branch targets. Existing aligned-wide
tests cover all predicates, signedness, operand orders, boundary values, and
every volatile input byte. The complete VM suite passes (303 tests), including
full Atari and VBXE Mandelbrot images. NIR and MIR snapshots pass, as do the
51-fixture NIR sweep and 168-fixture MIR sweep. The passing corpus expectation
increases from 347 to 348 for the new MIR fixture; the 12 known semantic
failures are unchanged.

The full compiler suite passes in an isolated checkout containing this patch
(3,119 passed, 24 ignored). The original working tree's full run encountered
an unrelated sample-loader failure: uncommitted wireframe support introduces
`samples/vbxe/shared/lines.act`, whose `SHARED.SCREEN` import is not resolved by
`parses_all_sample_programs`. That work is excluded from the isolated checkout
and left unchanged. Validation logs live under `build/compare-branch-fusion/`.

The following standalone measurements use the same source, origin, compiler
settings, pinned VM, and multiplication kernel before/after. Origins are
`$2000` for Atari and `$3000` for VBXE/the probe. Source bytes, thresholds,
recurrence, plotting, and palettes are unchanged. VBXE builds use
`--module-path samples/vbxe`. These current-source controls supersede historical
sample file sizes; the latest palette selection code is included on both sides.

| Measure | Before | After |
| --- | ---: | ---: |
| Minimal probe XEX, including headers | 55 bytes | 44 bytes |
| Probe false path, through result store to terminal loop | 38 cycles | 27 cycles |
| Probe true path, through result store to terminal loop | 34 cycles | 29 cycles |
| Atari XEX | 2,533 bytes | 2,377 bytes |
| Atari 640-point recurrence, 6,167 updates | 16,032,406 cycles | 16,047,747 cycles |
| Atari 16,000-point recurrence, 151,649 updates | 409,321,262 cycles | 409,695,879 cycles |
| VBXE XEX | 4,374 bytes | 4,366 bytes |
| VBXE 640-point recurrence, 6,167 updates | 16,106,410 cycles | 16,047,747 cycles |
| VBXE 16,000-point recurrence, 151,649 updates | 411,141,050 cycles | 409,695,879 cycles |

Both renderers now branch directly on the radius comparison. VBXE retains
the same inlining decisions and needs 1,445,171 fewer cycles on the original
grid (0.35%). Atari saves 156 bytes, but its changed layout makes the existing
inliner decline the single `MulFloor` site as `declined-non-improvement`, with
zero proven saved cycles. Its two `SqrWide` sites still expand. The extra call
overhead outweighs the branch improvement by 374,617 cycles (0.09%). This is a
measured speed/size tradeoff, not a claim that the Atari frame runs faster.

The inliner algorithm, growth limits, and cost proof are unchanged. Its former
test requiring all three standalone sites to expand was updated to require
both square sites while permitting the single wrapper's costed fallback;
retained-body equality remains checked. `INLINE` remains a preference.

All four recurrence profiles compare every escape count with the independent
integer oracle and check listing instructions against the actual executable.
Multiply helper work stays at 263,652,760 cycles for the original grid, with
480,613 calls. Coordinate mapping is unchanged. Timings count CPU instructions
without display DMA/interrupt contention and exclude plotting. The full-image
tests separately verify rendering correctness.

The recurrence profiler is `build/oscar64-mandelbrot/action-profile.rs`; the
minimal-path profiler is `build/compare-branch-fusion/probe-profile.rs`. Both
use Actionc VM `7ec0cc454ebf43b088b7bcd11515533085ea1964`. Acceptance tests use
`CARGO_PROFILE_TEST_OPT_LEVEL=1` to accelerate the Rust compiler/VM test hosts
while retaining test assertions; Action compilation settings are unchanged.
