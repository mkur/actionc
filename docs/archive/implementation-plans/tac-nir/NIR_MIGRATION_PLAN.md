# TAC To NIR Migration Plan

Snapshot date: 2026-05-31.

This document describes how to migrate the current `tac` layer into NIR
(Normalized Intermediate Representation). The current `tac` module is the
implementation home for the migration. Do not start with a broad module rename;
first make the IR contract strict enough to support optimization and MIR6502
lowering.

The target pipeline is:

```text
Action source -> AST -> semantic model -> SemIR -> NIR -> MIR6502 -> emission
```

## Codex Execution Rule

Codex should execute this migration as small, test-gated slices.

For every major slice below, Codex must:

1. make only the changes required for that slice;
2. run the required checks for the touched area when possible;
3. update fixtures only when the IR contract or printer intentionally changes;
4. commit the slice before starting the next major slice;
5. use a commit message that names the milestone and the specific invariant
   improved.

Do not batch multiple major milestones into one commit. If a slice needs to be
split further to keep the repository compiling, split it and commit each
compiling sub-slice separately.

Suggested commit message shape:

```text
nir: <short imperative summary>
```

Examples:

```text
nir: document verifier-clean boundary
nir: add emit-nir alias
nir: introduce block ids alongside labels
nir: reject legacy operands in scalar stores
```

## North Star Invariant

Verifier-clean NIR must be consumable by MIR6502 without consulting SemIR and
without parsing printable strings.

That means verifier-clean NIR eventually contains no executable:

- `TacOperand`;
- expression-summary strings;
- raw source strings;
- unresolved names;
- `Symbol(String)` storage identity;
- field names instead of byte offsets;
- index syntax strings;
- indirect callee target strings;
- metadata operations inside executable blocks;
- unknown/open control-flow boundaries;
- string labels as long-term CFG identity.

NIR should preserve display names and source syntax only as debug, printer, or
source-map metadata.

## Responsibility Split

SemIR owns Action! meaning:

- name resolution;
- source-level type checking;
- lvalue legality;
- callable facts;
- array/routine disambiguation;
- record facts;
- pointer facts;
- source control-flow meaning.

NIR owns normalized typed computation:

- routines, basic blocks, and terminators;
- stable IDs for MIR-relevant entities;
- typed temps and typed values;
- explicit loads and stores;
- explicit casts, unary ops, binary ops, compares, and branches;
- explicit address-of and storage facts;
- static data references;
- call signatures and conservative effects;
- machine-block barriers and payloads when available.

MIR6502 owns target strategy:

- byte/word expansion;
- A/X/Y/flags use;
- 6502 addressing-mode selection;
- zero-page decisions;
- ABI homes;
- helper routine selection;
- compare/branch fusion;
- target-specific peepholes.

Emission owns final instruction/data writing, labels, patching, maps, listings,
and proof hooks.

## Required Checks

For slices touching TAC/NIR lowering, verifier, printer, or fixtures, run:

```sh
cargo test tac_fixtures_match_snapshots
cargo run --bin actionc-tac-sweep -- fixtures/tac
cargo test
```

When NIR-specific commands or fixtures are introduced, also run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
```

If a check cannot be run in the current environment, Codex should say so in the
commit/PR notes and list the checks that still need to be run manually.

## Milestone 0: Boundary Documentation And Guardrails

Goal: make the migration contract visible before changing behavior.

Scope:

- Add or update documentation that defines NIR, the SemIR/NIR/MIR6502 split, and
  the verifier-clean invariant.
- Keep `tac` as the implementation module name for now.
- Do not change compiler behavior.
- Do not update fixtures.

Suggested files:

- `AGENTS.md`
- `docs/NIR_MIGRATION_PLAN.md`
- optionally `docs/NIR_BOUNDARY.md`

Acceptance criteria:

- The repository documents that current `tac` is the transitional implementation
  home for future NIR.
- The docs state that optimizer passes may run only after NIR verification.
- The docs state that MIR6502 must not recover missing NIR facts by looking back
  into SemIR.

Codex commit checkpoint:

```text
Commit after documentation is added and before any compiler behavior changes.
Suggested message: nir: document TAC to NIR migration boundary
```

## Milestone 1: Observation Surface And Naming Bridge

Goal: introduce NIR terminology without destabilizing existing TAC tests.

Scope:

- Add `--emit-nir` as an alias for the current `--emit-tac` output.
- Keep `--emit-tac` working.
- For now, `--emit-nir` is backed by the transitional `tac` implementation and
  prints the same verifier-clean observation surface as `--emit-tac`.
- Keep the current printed format stable unless help text changes are needed.
- Add a focused test that proves `--emit-nir` and `--emit-tac` currently emit the
  same IR for at least one fixture.
- Do not rename `src/tac` yet.

Acceptance criteria:

- Existing TAC fixture tests still pass.
- `actionc-emit --emit-nir <file.act>` works.
- Documentation clearly says `--emit-nir` is currently backed by the transitional
  TAC implementation.

Codex commit checkpoint:

```text
Commit after the alias and tests pass.
Suggested message: nir: add emit-nir alias for transitional TAC output
```

## Milestone 2: Verifier Hardening Baseline

Goal: make the current verifier stricter without changing the broad IR shape.

Scope:

- Identify all legacy executable forms that are already intended to be rejected.
- Ensure verifier-clean TAC/NIR rejects executable metadata ops.
- Ensure verifier-clean TAC/NIR rejects legacy `Set`, legacy `Assign`, and legacy
  `ForStep` compound operations where replacements already exist.
- Preserve compatibility paths only when a replacement form is not implemented
  yet, and document each remaining compatibility path.
- Add verifier tests for rejected legacy forms.

Current compatibility path after this milestone: `TacOp::CompoundAssign` may
remain only for unstable places whose explicit load/binary/store lowering is not
implemented yet. `ForStep` compound assignments, executable metadata ops, legacy
`Set`/`Assign`, `Open`, and `Unknown` control-flow boundaries are verifier
errors.

Acceptance criteria:

- Removed legacy shapes cannot silently reappear.
- Verifier diagnostics are specific enough to guide the next migration slice.
- Existing fixtures remain green or are updated only for intentional verifier
  contract changes.

Codex commit checkpoint:

```text
Commit after each verifier-tightening group that leaves tests passing.
Suggested message: nir: harden verifier against legacy executable ops
```

## Milestone 3: Stable Block Identity

Goal: introduce block IDs while preserving readable labels.

Scope:

- Add `BlockId` to blocks if not already fully used in the executable boundary.
- Keep human-readable labels as metadata for printing.
- Build a block-ID lookup table per routine.
- Convert internal CFG validation to prefer block IDs.
- Initially allow string labels as a compatibility surface if needed, but isolate
  the conversion in one place.

Acceptance criteria:

- Printer output remains readable.
- Verifier validates block identity through a stable ID path.
- MIR/NIR consumers no longer need ad hoc string maps for basic CFG traversal
  once this milestone is complete.

Codex commit checkpoints:

```text
Commit 1 after BlockId is added alongside labels.
Suggested message: nir: add block ids alongside printable labels

Commit 2 after terminator validation uses block IDs or a centralized conversion.
Suggested message: nir: validate CFG targets through block ids
```

## Milestone 4: Temp Table And CFG-Aware Use Validation

Goal: make temps safe for cross-block optimization.

Scope:

- Add a routine-local temp table with ID, type, defining block/op, and optional
  source metadata.
- Build predecessor and successor sets from terminators.
- Add single-definition validation.
- Add cross-block use validation with dominance or a conservative dataflow rule.
- Keep the existing block-local validation until the new validation is proven.

Acceptance criteria:

- Verifier catches cross-block use-before-definition.
- Every temp use has a known type authority.
- Redundant type copies on temp values are either removed or verified against the
  temp table.
- Existing fixtures pass.

Codex commit checkpoints:

```text
Commit 1 after temp table scaffolding compiles.
Suggested message: nir: add routine temp table scaffolding

Commit 2 after CFG predecessor/successor construction compiles.
Suggested message: nir: build CFG facts for verifier

Commit 3 after cross-block temp validation is active.
Suggested message: nir: validate temp uses across blocks
```

## Milestone 5: Structured Symbol And Storage Facts

Goal: stop treating executable storage identity as a string.

Scope:

- Add fact records for params, locals, globals, statics, and routine storage where
  needed.
- Introduce stable IDs for MIR-relevant storage entities.
- Replace executable scalar `Symbol(String)` places with structured variants for
  params, locals, globals, and absolutes.
- Keep display names in fact records for printing.
- Keep a temporary name-to-ID resolution layer in the lowerer if needed.

Target place direction:

```rust
enum NirPlaceKind {
    Param(ParamId),
    Local(LocalId),
    Global(SymbolId),
    Absolute(u16),
    Deref { addr: NirValue, ty: NirType },
    Field { base: Box<NirPlace>, offset: u16, ty: NirType },
    Index { base_addr: NirValue, index: NirValue, elem_ty: NirType, elem_size: u8 },
}
```

Acceptance criteria:

- Scalar `Load`, `Store`, and `AddrOf` no longer require MIR consumers to resolve
  executable `Symbol(String)` places.
- Printer output still shows useful names.
- Verifier rejects executable `Symbol(String)` in the migrated scalar profile.

Codex commit checkpoints:

```text
Commit 1 after storage fact records and IDs are introduced.
Suggested message: nir: add structured storage facts

Commit 2 after scalar loads/stores use structured storage places.
Suggested message: nir: replace scalar symbol places with storage ids

Commit 3 after verifier rejects migrated executable Symbol(String) uses.
Suggested message: nir: reject string storage identity in scalar code
```

## Milestone 6: Address-Oriented Places

Goal: make dereference, indexing, and field access semantic rather than
source-syntax shaped.

Scope:

- Replace dereference places that contain legacy operands with value-addressed
  dereference places.
- Replace indexed places that contain legacy operands and syntax text with
  `base_addr`, `index`, `elem_ty`, and `elem_size`.
- Replace record field names with byte offsets and field type facts.
- Add address formation ops if needed to evaluate complex places once.
- Preserve source syntax only as metadata.

Acceptance criteria:

- Executable deref/index/field places contain no `TacOperand`.
- Executable field places contain offsets, not field names.
- Executable index places contain semantic element size, not syntax text.
- Record, array, pointer, and address-of fixtures pass or gain targeted expected
  NIR output changes.

Codex commit checkpoints:

```text
Commit 1 after deref places use typed address values.
Suggested message: nir: make dereference places value-addressed

Commit 2 after index places use base address and element facts.
Suggested message: nir: make index places address-oriented

Commit 3 after field places use byte offsets.
Suggested message: nir: lower record fields to offsets
```

## Milestone 7: Remove `TacOperand` From Executable Paths

Goal: make executable NIR use only structured values, places, metadata, or
explicit unsupported diagnostics.

Scope:

- Track all remaining `TacOperand` uses.
- Replace executable literal operands with typed constants.
- Replace executable temp operands with typed values or temp IDs verified by the
  temp table.
- Replace executable place/address operands with `NirPlace`, `NirValue`, or
  address ops.
- Move raw text and expression summaries into debug metadata or remove them.
- Add verifier checks that reject executable `TacOperand` for migrated profiles.
- Delete `TacOperand` only after all executable users are gone.

Acceptance criteria:

- Scalar and storage fixtures contain no executable `TacOperand`.
- Compatibility users are isolated and documented.
- The verifier prevents reintroduction into migrated executable ops.

Codex commit checkpoints:

```text
Commit after each family of TacOperand users is removed.
Suggested messages:
  nir: replace literal operands with typed values
  nir: replace temp operands with verified values
  nir: remove operands from executable storage paths
  nir: reject executable TacOperand in migrated code
```

## Milestone 8: Assignment, Compound Assignment, And FOR Normalization

Goal: expose assignment order as normal operations.

Scope:

- Ensure plain assignment is always represented as `Store`.
- Lower compound assignment to explicit `Load -> operation -> Store`.
- For unstable places, evaluate the address/place once before load/store.
- Replace `FOR` step compatibility forms with explicit arithmetic and branches.
- Encode unsupported loop-step semantics as explicit unsupported diagnostics, not
  string operations.

Acceptance criteria:

- No verifier-clean NIR contains executable legacy `Assign`.
- No verifier-clean NIR contains executable `CompoundAssign`.
- `FOR` fixture output contains normal loads, compares, binary ops, stores, and
  branches.

Codex commit checkpoints:

```text
Commit 1 after plain assignments are all stores.
Suggested message: nir: normalize assignments to stores

Commit 2 after compound assignments lower through load op store.
Suggested message: nir: normalize compound assignments

Commit 3 after FOR updates lower to explicit arithmetic.
Suggested message: nir: normalize FOR loop step updates
```

## Milestone 9: Condition Normalization

Goal: make every branch condition semantically explicit.

Scope:

- Decide and document the initial condition representation:
  - value-producing bool temps, or
  - explicit branch/test terminators.
- Prefer starting with value-producing bool temps because they are simpler to
  verify and optimize.
- Lower comparisons to bool/condition temps.
- Lower nonzero scalar/pointer tests explicitly.
- Lower bitwise expressions in conditions by materializing the bitwise result and
  testing it against zero.
- Lower short-circuit `AND` and `OR` through explicit CFG blocks.
- Add fixtures for nested logical conditions, negation, bitwise nonzero tests,
  pointer nonzero tests, and constant branches.

Acceptance criteria:

- `Branch` consumes only bool/condition values or a documented explicit test form.
- Verifier rejects non-bool branch values in migrated code.
- Bitwise conditions such as `WHILE skstat & $04 DO` test the bitwise result, not
  the unmasked source value.

Codex commit checkpoints:

```text
Commit 1 after nonzero tests are explicit.
Suggested message: nir: materialize nonzero branch conditions

Commit 2 after logical AND/OR lowering is explicit.
Suggested message: nir: lower short-circuit conditions to CFG

Commit 3 after verifier enforces branch condition types.
Suggested message: nir: enforce typed branch conditions
```

## Milestone 10: Calls, Signatures, And Effects

Goal: make calls optimizable and safe scheduling barriers.

Scope:

- Replace indirect callee target strings with typed values.
- Add callable signatures with parameter types, return type, routine kind, and ABI
  class where known.
- Verify call arity and argument width/type compatibility.
- Verify call result type against the callee signature.
- Replace read/write counts with structured memory effects:
  - none;
  - known regions;
  - unknown;
  - all.
- Preserve register clobber/preserve facts, OS-call flags, and opaque flags.
- Treat opaque calls and OS/runtime calls conservatively until precise effects are
  available.

Acceptance criteria:

- MIR6502 can lower calls without looking back into SemIR for callee expression
  shape.
- Verifier rejects arity/type mismatches.
- Verifier rejects untyped indirect callees.
- Calls and opaque effects remain ordering barriers for optimization.

Codex commit checkpoints:

```text
Commit 1 after call signatures are represented.
Suggested message: nir: add callable signature facts

Commit 2 after indirect callees use typed values.
Suggested message: nir: make indirect callees value-based

Commit 3 after call verifier checks arity and types.
Suggested message: nir: verify call signatures

Commit 4 after effects are structured.
Suggested message: nir: structure call memory effects
```

## Milestone 11: Static Data And Machine Blocks

Goal: make NIR self-contained for static allocation and inline machine-code
preservation.

Scope:

- Ensure static data uses byte payloads as authoritative data.
- Add alignment, mutability, and section facts if needed by MIR6502.
- Verify `StaticAddr` references valid static data IDs.
- Replace machine-block summaries with ordered machine items when available.
- Replace formatted machine effects with structured effects.
- Treat machine blocks as opaque barriers by default.

Acceptance criteria:

- MIR6502 can allocate static bytes from NIR alone.
- MIR6502 can preserve or emit inline machine blocks from NIR alone, or reject
  them with a precise unsupported diagnostic.
- No machine-block lowering depends on parsing formatted effect strings.

Codex commit checkpoints:

```text
Commit 1 after static data validation is complete.
Suggested message: nir: validate byte-exact static data

Commit 2 after machine block payloads are represented or explicitly rejected.
Suggested message: nir: structure machine block payloads

Commit 3 after machine effects are structured.
Suggested message: nir: structure machine block effects
```

## Milestone 12: NIR Fixture And Sweep Infrastructure

Goal: make NIR observable as its own contract.

Scope:

- Add `fixtures/nir` once output differs meaningfully from old TAC or once the
  migration needs a separate contract surface.
- Add `tests/nir_fixtures.rs` mirroring the TAC fixture strategy.
- Add `actionc-nir-sweep` if the existing TAC sweep cannot cleanly serve both
  roles.
- Keep TAC fixtures during the transition.
- Document whether `--emit-tac` is an alias, legacy observation mode, or a
  deprecated command.

Acceptance criteria:

- NIR fixtures are the optimizer contract.
- TAC fixtures remain either historical compatibility coverage or are retired in
  a documented step.
- Sweep output clearly distinguishes SemIR failures, NIR lowering failures, NIR
  verifier failures, and later MIR/codegen failures.

Codex commit checkpoints:

```text
Commit 1 after NIR fixtures are introduced.
Suggested message: nir: add fixture snapshots

Commit 2 after NIR sweep support is introduced.
Suggested message: nir: add sweep validation command
```

## Milestone 13: First Safe NIR Optimizations

Goal: introduce optimizer infrastructure only after verifier-clean NIR is strict.

Scope:

- Add an explicit pass runner that always verifies before and after optimization
  in debug/test paths.
- Start with safe local and CFG passes:
  - empty block cleanup;
  - unreachable block removal;
  - constant folding;
  - constant branch simplification;
  - copy propagation;
  - dead temp elimination.
- Avoid aggressive alias-sensitive optimization until storage identity, effects,
  and dominance are strong.
- Keep target-specific peepholes in MIR6502 or later, not NIR.

Acceptance criteria:

- Every optimization pass preserves verifier-clean NIR.
- Each pass has focused tests.
- Optimizations are disabled or conservative around calls, absolute memory,
  pointer dereferences, hardware registers, runtime/OS calls, and machine blocks.

Codex commit checkpoints:

```text
Commit 1 after pass runner and verify-before/after hooks are added.
Suggested message: nir: add optimization pass runner

Commit after each optimization pass lands with tests.
Suggested messages:
  nir: clean unreachable blocks
  nir: fold constants
  nir: simplify constant branches
  nir: propagate copies
  nir: eliminate dead temps
```

## Milestone 14: MIR6502 Consumer Boundary

Goal: make MIR6502 consume verifier-clean NIR rather than transitional TAC or
SemIR shapes.

Scope:

- Add a NIR-to-MIR6502 lowering entry point.
- Require NIR verification before lowering.
- Reject any remaining compatibility shape with precise diagnostics.
- Do not allow MIR6502 to inspect SemIR to recover missing types, storage facts,
  field offsets, call signatures, or branch semantics.
- Keep SemIR-native backend as a correctness runway and comparison oracle until
  the NIR path is proven.

Acceptance criteria:

- The scalar NIR profile lowers to MIR6502.
- MIR6502 tests cover at least scalar loads/stores, arithmetic, compares,
  branches, calls, absolute stores, and returns.
- Missing features fail at the NIR/MIR boundary with clear diagnostics.

Codex commit checkpoints:

```text
Commit 1 after the NIR-to-MIR6502 entry point exists.
Suggested message: mir6502: add NIR lowering entry point

Commit after each supported NIR operation family lands.
Suggested messages:
  mir6502: lower scalar loads and stores from NIR
  mir6502: lower arithmetic and compares from NIR
  mir6502: lower branches and returns from NIR
  mir6502: lower calls from NIR
```

## Final Rename Or Alias Decision

Only after the IR is strict and NIR fixtures exist should the repository decide
whether to rename code paths.

Options:

1. Conservative: keep `src/tac` as the implementation module and document it as
   historical naming.
2. Moderate: add `src/nir` and keep `src/tac` as a compatibility wrapper or
   re-export.
3. Aggressive: rename `src/tac` to `src/nir`, rename fixtures and tests, and keep
   CLI aliases for a transition period.

Recommended path: moderate. Avoid a large rename until behavior and verifier
contracts have stabilized.

Codex commit checkpoint:

```text
Commit only after the rename/alias policy is approved.
Suggested message: nir: rename transitional TAC module
```

## Red Lines

Do not call the migration complete while any of these are true:

- optimizer passes run on legacy/stringly TAC shapes;
- MIR6502 must consult SemIR to recover missing NIR facts;
- executable code uses `TacOperand`;
- executable storage identity depends on `Symbol(String)`;
- executable field/index forms preserve source syntax instead of semantic facts;
- call arity/type/effects are unverified;
- machine blocks lack payload/effect handling or an explicit unsupported barrier;
- cross-block temp use is not verified;
- branch conditions are not typed or explicitly tested;
- verifier-clean IR can contain unknown/open boundaries.

## Suggested First Codex Task

Use this as the first implementation task after this document exists:

```text
Implement Milestone 1 only.

Goal:
- Add `--emit-nir` as an alias for the current `--emit-tac` output.
- Keep `--emit-tac` working.
- Do not rename modules.
- Do not change printed IR except for help text if necessary.
- Add a focused test proving both flags produce identical output for one fixture.

Required checks:
- cargo test tac_fixtures_match_snapshots
- cargo run --bin actionc-tac-sweep -- fixtures/tac
- cargo test

Commit after this slice before starting another milestone.
Suggested commit message:
- nir: add emit-nir alias for transitional TAC output
```
