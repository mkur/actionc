# INLINE routine declarations

Status: in progress, 2026-09-11. Slice 1 implements contextual declaration
syntax and typed preference propagation through AST, SemIR, NIR and MIR6502.
The new lowered/optimized NIR fixtures add the `inline prefer` metadata contract;
existing fixture output is unchanged. Slice 2 implements requested BYTE-leaf
priority, separate cumulative budgets and applied/declined site reports.
Focused execution tests confirm the larger body gate changes selection while
recursion, storage and non-improvement retain calls. Slices 3–6 remain pending.

## Objective

Add an explicit routine declaration modifier that requests inlining, and extend
the existing MIR6502 inliner far enough to expand ordinary Q4.12 arithmetic
wrappers into their callers. Deliver and validate each slice separately, with
one implementation commit per slice.

```action
INLINE BYTE FUNC Mix(BYTE left,right)
RETURN((left LSH 1) XOR right)

PUBLIC INLINE INT FUNC MulFloor(INT left,right)
  LONGINT product,wideRight
  product=LONGINT(left)
  wideRight=LONGINT(right)
  product=product*wideRight
RETURN(INT(LONGCARD(product) RSH 12))
```

The release acceptance case is the existing Mandelbrot implementation calling
`MATH.Q4_12.MulFloor` and `SqrWide`. Recognition must depend on typed operations
and effects, never module names, function names or sample constants.

## Language contract

- Syntax is `[PUBLIC] INLINE PROC Name(...)` or
  `[PUBLIC] INLINE <result-type> FUNC Name(...)`. `PUBLIC` retains its existing
  named-module scope rules. `INLINE` also works in unnamed source modules.
- Treat `INLINE` as a case-insensitive contextual modifier. Existing identifiers
  named `INLINE`, including routine names and qualified members, remain legal
  outside modifier position. Declaration lookahead must include routine-body
  boundaries, not just top-level parsing.
- It is a strong optimization preference, not mandatory expansion. It changes
  no arithmetic, evaluation order, visibility, calling convention, parameter
  persistence, return-slot behavior or callable identity.
- Automatic inlining continues without the modifier. Requested candidates get
  priority and a larger bounded growth allowance, but retain legality and
  profitability checks. Unknown cost is a reason to retain the call.
- Accept the hint in all actionc profiles so annotated embedded libraries still
  compile in compatibility, optimized-classic and MIR6502 modes. Only modern
  MIR6502 acts on it initially; other backends retain ordinary calls. This is an
  actionc extension, not source compatibility with the original Action! compiler.
- Invalid placement, duplicate modifiers and `INLINE EXTERNAL` declarations
  receive source diagnostics. A body with unsupported operations, an observable
  entry or recursion is a valid request that can be declined by optimization.
- Report requested/applied/declined outcomes and reasons through existing
  optimization reporting. Do not warn on every ordinary build. Document that a
  retained call is permitted. No `;@actionc inline`, `ALWAYSINLINE`, `NOINLINE`,
  call-site modifier or public command-line override is part of this release.

## Current implementation and required extensions

The existing inliner is implemented in
`src/mir6502/analysis/leaf_routines.rs` and
`src/mir6502/materialize/inlining.rs`, with emitted-code costing in
`src/mir6502/materialize/inlining_cost.rs`.

It currently admits at most two byte parameters, four acyclic blocks and twelve
MIR operations, with no local storage or calls/helpers. It is enabled by
`Mir6502Config::optimized()`, which the CLI uses for `--mode mir6502`.
Selection materializes and emits baseline/trial programs before accepting an
expansion. Preserve this transaction and the existing automatic-byte behavior.

Three representation details matter:

1. Action parameters and ordinary locals have persistent storage. `INLINE` does
   not make them automatic variables. Omitted arguments and escaped storage can
   make parameter capture observable across calls.
2. NIR LONG values lower into two MIR word lanes. A user call currently defines
   its low result through `MirOp::Call`, followed by a separate load from the
   upper return slot at `$A2`. Wide inlining needs an explicit complete result
   representation, rather than guessing ownership of an adjacent memory load.
3. The cost walker currently represents the expanded leaf by a cycle bound. A
   wrapper with helpers also has an ordered sequence of nested calls; that
   sequence must survive expansion and participate in path matching.

Related contracts are documented in
[the existing inliner plan](MIR6502_SMALL_LEAF_INLINER_PLAN.md) and
[the MIR6502 contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md).

## Ownership and invariants

AST stores the modifier and its source span. SemIR validates declaration use
and preserves a typed preference, including through module import/flattening
and compatibility projections. NIR carries that preference as routine metadata
associated with its stable routine ID. Use an enum such as `Auto` / `Prefer`,
not annotation text, an executable operation or an expression summary.

MIR6502 receives the metadata from verified NIR and owns eligibility, expansion,
growth limits, costing and helper handling. Keep expansion before parameter
prologues, block-argument lowering and home allocation. Emission writes the
selected result and supplies measurement/provenance; it does not interpret
`INLINE`. No new NIR inliner or SemIR lookback is needed.

All accepted transformations must preserve these guarantees:

- Evaluate actual arguments exactly once in the original order and capture
  their already evaluated values at the original call point.
- Use fresh block/temp/storage identities where appropriate, remapping all
  supported references simultaneously and rebuilding dependent analyses.
- Preserve all declared return lanes and canonical public return-slot writes,
  including when the source discards a result. Eliding those writes is a
  separate optimization requiring its own observability proof.
- Prove all calls supply the full argument list and that candidate parameter
  storage, local storage and routine addresses do not escape. Keep the existing
  conservative whole-candidate rejection when any call violates that proof.
- Module `PUBLIC` visibility alone is not an address escape. An imported public
  source routine with a known body can qualify under the same whole-program
  proof. Fixed/current-location, external and program entries remain excluded.
- Never assume that local storage resets on entry. Only promote scalar scratch
  whose reads are dominated by definitions in the current invocation and whose
  identity is unobservable. Persistent counters/initializers remain observable.
- Keep retained helpers as ordering barriers with their declared memory,
  register, flag, stack and fault effects. Do not move work across them based
  solely on the inline preference.
- Retain the original callable body initially and give the cost model no
  dead-callee deletion credit. Do not recursively expand newly cloned routines
  in the same pass. Detect recursive call-graph components and decline them.
- Verify candidate MIR before costing and after accepted changes. NIR metadata
  additions and MIR call-form changes must tighten their respective verifiers.

## Slice 1: declaration syntax and typed metadata

Implement contextual modifier parsing, including named/unnamed modules,
`PUBLIC INLINE`, declaration boundaries and targeted invalid-placement errors.
Carry the preference through AST, semantic lowering, module projections, NIR
and MIR. Do not attach it to the callable type: it changes no type identity.

Print non-default preferences readably in SemIR/NIR/MIR; leave default routine
output unchanged. Update syntax and ownership documentation. Classic codegen
accepts the metadata and emits its normal calls.

Acceptance: parser tests cover identifiers named INLINE, consecutive routines,
typed functions, procedures, aliases/imports, duplicates and non-routine use.
Fixture tests prove metadata survives lowering and optimization. All existing
mode/runtime combinations still accept unannotated source.

Suggested commit: `frontend: carry INLINE routine preferences through typed IR`.

## Slice 2: requested inlining policy and explanations

Activate `Prefer` for the existing byte-leaf subset. Separate the preference
from eligibility; never use it to bypass an escape, effects or verifier check.
Prioritize requested candidates deterministically before automatic candidates.

Start with explicit requested growth ceilings of 128 bytes/site, 512/caller and
1024/program, with at most 16 requested trials and eight sites/group. Keep
automatic candidates on their existing ceilings and trial budget; charge all
accepted growth cumulatively against a combined 1280-byte program ceiling.
Validate these constants against emitted results before rollout. They are
resource limits, not benchmark-specific selection rules.

Keep the automatic four-block/twelve-operation eligibility limits unchanged.
Requested candidates may contain at most eight acyclic blocks and 128
pre-materialization MIR operations, with at most two logical scalar parameters.
Widening this size gate does not admit new operations before their supporting
slices are complete.

Require the existing conservative cycle-saving proof. Report source routine,
caller/site, preference, outcome, bytes and estimated cycles, with distinct
reasons for unsupported shape, observable storage, recursion, cost uncertainty,
non-improvement and exhausted budget. Suppress speculative-trial reports.

Acceptance: a supported BYTE example demonstrates the hint's selection effect;
unsafe and unprofitable candidates retain calls and report reasons. Existing
AES automatic inlining and default/off configuration behavior remain covered.

Suggested commit: `mir6502: honor INLINE preferences with bounded costed trials`.

## Slice 3: word and LONG argument/result support

Generalize captures and continuation arguments from fixed bytes to typed lanes.
Support scalar BYTE, CHAR, INT, CARD, LONGINT and LONGCARD arguments/results,
initially with the existing two logical-parameter limit. Map parameter byte
offsets to lane captures explicitly; a LONG parameter occupies two word lanes.

First add structured additional result lanes to ordinary MIR calls, reusing the
runtime-helper result model where practical. Lower user LONG results as one
call defining both word lanes, replacing the current call-plus-high-slot-load
shape. Update ABI planning, effects/use-def visitors, remappers, materialization,
printer and verifier together. Verify unique result definitions, widths and
non-overlapping ABI homes, including discarded and partially consumed values.
Keep actual machine calling conventions unchanged.

Use complete return-lane definitions for inline continuations, preserving
`$A0..$A3` writes as applicable. Do not reconstruct wide call results from
source names or opportunistic neighboring loads. Keep this slice restricted to
call-free arithmetic and conversions.

Acceptance: execute inline-on/off cases for signed extension, truncation,
carry/borrow across lanes, multiple returns, repeated calls, mixed-width
parameters and results consumed after another call. Add negative verifier
tests for malformed multi-result calls.

Suggested commit: `mir6502: inline scalar word and LONG routines with explicit result lanes`.

## Slice 4: proven private scalar scratch

Reuse current NIR scalar promotion first. Where residual scalar homes block a
candidate, add only the missing bounded, verified proof/promotion for scratch
defined before every read in the current invocation. Preserve explicit typed
storage identities and the existing call/memory barriers.

Support BYTE/word/LONG scalar scratch; do not clone persistent storage into a
fresh per-call frame merely because it is spelled as a local. Reject addressed,
aliased, initialized persistent, volatile, aggregate and read-before-definition
homes. Parameter mutation remains outside the initial substitution contract.
This slice can be limited to regression coverage if existing promotion already
eliminates every needed home; avoid a redundant optimizer pass.

Acceptance: branch-defined scratch and wide temporary relays agree with calls;
persistent counters, partial writes, omitted arguments and escaped addresses
retain their original behavior. Explain any changed NIR snapshots as deliberate
promotion changes, not printer cleanup.

Suggested commit: `mir6502: admit inline scratch backed by verified storage proofs`.

## Slice 5: arithmetic wrappers containing retained helpers

Permit a closed initial set of compiler-owned arithmetic helpers in requested
candidates: multiplication, shifts, division and remainder with structured
effects and balanced normal-return stack behavior. Preserve division faults
and all helper calls; this slice does not inline helper machine code or replace
arithmetic algorithms. Ordinary nested user calls, OS calls, indirect calls and
opaque machine blocks remain excluded.

Clone helper arguments and every result lane together. Extend costing from a
leaf cycle bound to path summaries containing ordered helper-call events. Pair
events only when target/ABI/effects and captured input-value correspondence are
preserved. Same target names alone are insufficient. Account for wrapper
prologues, return handling, spills, branch layout and call overhead. Never
pretend a data-dependent helper costs zero; cancel equivalent helper work on
both sides, retaining conservative treatment of relocation/layout changes.
Unknown path correspondence retains the wrapper call.

Acceptance: generic signed multiply-and-shift wrappers, wide squares and
division wrappers execute identically inline-on/off, including faults. Results
remain live across multiple helpers, and helper ordering/clobbers are unchanged.
The cost test must actually select a supported wrapper, not merely demonstrate
that cloning is structurally legal.

Suggested commit: `mir6502: inline arithmetic wrappers while preserving helper calls`.

## Slice 6: Q4.12 rollout and measured validation

Mark `MulFloor` and `SqrWide` as `PUBLIC INLINE` without changing their formulas,
rounding, wrapping or public ABI. Other Q4.12 and Q8.8 functions are follow-up
opt-in candidates after measurement; do not annotate the entire library blindly.

Compare inline-enabled and disabled builds from the same compiler revision.
Require the two `SqrWide` calls and the `MulFloor` call in the Mandelbrot
recurrence to expand under the general rules. If the cost proof rejects them,
resolve and document the general limitation before declaring this rollout
complete; do not add a sample-specific exception or bypass profitability.

Use the existing fixed-point VM infrastructure. Cover every signed 16-bit input
for squaring and boundary/deterministic pair sets for `MulFloor`, especially
negative fractional products and INT minimum. Preserve floor versus
truncation-toward-zero behavior. Check matched Mandelbrot coordinates, iteration
counts and full Atari/VBXE framebuffer results.

Record code bytes, kernel CPU cycles, helper/wrapper call counts and build-time
impact, including retained bodies. The current comparison baseline is
20,769,965 cycles for the 640-point grid with 6,167 updates, and 532,548,330 for
the original 16,000-point grid with 151,649 updates. Rebuild the no-inline control
alongside the candidate; do not mix resolutions or rely only on old totals.
See [the Oscar64 comparison](OSCAR64_MANDELBROT_CODEGEN_COMPARISON.md).

Keep permanent regression cases small and focused. Save generated listings and
profiling artifacts under ignored `build/`, not as new copies throughout the VM
test tree. Update the syntax reference, inliner contract, fixed-point docs and
this plan's status with actual supported limits and results.

Suggested commit: `math: request Q4.12 wrapper inlining and validate Mandelbrot codegen`.

## Checks and completion criteria

After every slice touching semantic lowering, NIR, its verifier/printer or
related representations, run the repository-required checks before committing:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

For MIR representation/expansion slices also run:

```sh
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
```

Run matching new focused VM tests during the relevant slice. For the final
library rollout run the existing VM suite, which includes fixed-point and
Oscar64 ports:

```sh
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml
```

Every fixture delta must identify an intentional IR contract change, printer
change or optimization/bug fix. The release is complete when declaration
metadata is preserved, fallback behavior is documented and observable in
reports, safety rejections are covered, Q4.12 call sites expand, results match,
and measured code generation improves with bounded growth.

Assembly Q4.12 kernels, power-of-two division lowering, general nested inlining,
recursive expansion, aggregate/pointer inlining, automatic removal of retained
callees and broader zero-page allocation are separate work.
