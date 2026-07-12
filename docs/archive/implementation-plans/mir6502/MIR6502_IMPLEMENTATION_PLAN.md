# MIR6502 Implementation Plan

Snapshot date: 2026-06-01.

This note is the implementation plan for the MIR6502 layer. It is intended to be
used directly as a Codex execution plan.

Canonical contract: `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`.

The contract defines the pseudo-machine, phase model, value/address split,
verifier expectations, effects model, and deferred opcode families. This plan
must not drift from that contract. If implementation pressure reveals a contract
problem, update the contract first or in the same narrowly scoped commit.

Target pipeline:

```text
Action source -> AST -> semantic model -> SemIR -> NIR -> MIR6502 -> emission
```

## North Star

MIR6502 consumes verifier-clean NIR only. It must not inspect SemIR, parse
printed IR, or recover missing facts from source strings.

MIR6502 is a target-machine IR, not final emitted 6502 bytes. It should make
6502 decisions explicit enough to verify, print, test, and locally optimize
before the emission layer writes exact opcodes.

The first usable backend target is a scalar path:

```text
verifier-clean NIR
  -> pre-materialization MIR6502
  -> post-materialization MIR6502
  -> pre-emission MIR6502
  -> tracked emission
```

## Responsibility Boundary

Follow `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`.

NIR owns:

- source-language meaning;
- typed values and places;
- routine, block, temp, storage, static, and signature IDs;
- explicit loads, stores, casts, arithmetic, compares, calls, and branches;
- conservative call and machine-block effects.

MIR6502 owns:

- byte/word expansion;
- local A/X/Y/flags use;
- ABI argument and result homes;
- zero-page and scratch-slot decisions;
- 6502 addressing-form selection;
- runtime-helper selection;
- compare/test/branch fusion;
- target-specific peepholes after correctness.

Emission owns:

- exact opcode bytes;
- label binding and patching;
- branch-distance repair or diagnostics;
- load-file segment writing;
- source maps, listings, and proof hooks;
- tracked processor-state updates;
- raw data and machine-code barriers.

## Codex Execution Rule

Execute this plan as small, test-gated slices.

For every major milestone below:

1. make only the changes required for that milestone;
2. preserve existing tests unless the milestone intentionally changes an output
   contract;
3. add focused unit tests or fixtures for the new behavior;
4. run the required checks when possible;
5. commit before starting the next milestone;
6. use a commit message that names the layer and invariant improved.

Do not batch unrelated milestones. If a milestone is too large to keep the repo
compiling, split it into compiling sub-slices and commit each sub-slice.

Suggested commit message shape:

```text
mir6502: <short imperative summary>
```

Examples:

```text
mir6502: add MIR observation surface
mir6502: define value and address model
mir6502: verify structural invariants
mir6502: lower scalar loads and stores
```

## Required Checks

After touching MIR6502 code, CLI integration, fixtures, or test harnesses, run:

```sh
cargo test
```

Once MIR fixtures exist, also run:

```sh
cargo test mir6502_fixtures_match_snapshots
```

Once a MIR sweep command exists, also run:

```sh
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
```

When touching NIR-to-MIR lowering, also run the relevant NIR checks if present:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
```

If a check cannot be run in the current environment, say so in the commit or PR
notes and list the checks that still need manual execution.

## Red Lines

Do not implement MIR6502 in a way that violates the contract.

Do not allow MIR6502 to:

- inspect SemIR to recover missing facts;
- parse printed NIR or TAC;
- use source strings as executable storage identity;
- duplicate compare operands in branch conditions;
- treat memory locations as pure operation definition sites;
- lower calls without ABI and effect records;
- drop `clobbers`, `preserves`, stack, or memory effects silently;
- emit ordinary instructions without going through tracked emission;
- grow a one-variant-per-6502-opcode pseudo ISA in the first implementation;
- add target peepholes before the scalar path is correct and verified.

If a NIR input lacks facts needed by MIR, reject it with a precise diagnostic and
fix NIR in a separate NIR slice.

## Milestone 0: MIR Plan And Contract Alignment

Goal: keep the implementation plan aligned with the pseudo-machine contract.

Scope:

- Add this implementation plan.
- Keep `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md` as the canonical machine
  contract.
- Do not change compiler behavior.
- Do not update fixtures.

Acceptance criteria:

- This plan exists in `docs/MIR6502_IMPLEMENTATION_PLAN.md`.
- It references the contract explicitly.
- The plan is milestone-based and directly executable by Codex.

Suggested commit:

```text
mir6502: document implementation plan
```

## Milestone 1: Observation Surface And Module Scaffold

Goal: create an observable MIR6502 layer without real operation lowering.

Scope:

- Add `src/mir6502/`.
- Add initial modules:

```text
src/mir6502/mod.rs
src/mir6502/ir.rs
src/mir6502/lower.rs
src/mir6502/verify.rs
src/mir6502/printer.rs
src/mir6502/diagnostics.rs
```

- Expose a small public API:

```rust
pub fn lower_program(nir: &NirProgram) -> Result<MirProgram, Vec<MirDiagnostic>>;

pub fn verify_program(
    program: &MirProgram,
    phase: MirPhase,
) -> Result<(), Vec<MirDiagnostic>>;

pub fn format_program(program: &MirProgram) -> String;
```

- Add a CLI observation flag:

```text
--emit-mir6502
```

- Prefer `--emit-mir6502` over `--emit-mir` because this MIR is target-specific.
  A shorter alias can be added later if useful.
- Initial `lower_program` should create only program, routine, and block shells.
- Do not lower real operations yet.
- Do not emit 6502 bytes yet.

Acceptance criteria:

- `src/mir6502` compiles.
- `actionc-emit --emit-mir6502 <file.act>` works for a tiny input.
- MIR output is stable and readable.
- Existing tests still pass.

Suggested commit:

```text
mir6502: add MIR observation surface
```

## Milestone 2: Core IR Skeleton And Phase Model

Goal: implement the core program shape and phase model from the contract.

Scope:

- Define:

```rust
pub enum MirPhase {
    PreMaterialization,
    PostMaterialization,
    PreEmission,
}

pub struct MirProgram {
    pub statics: Vec<MirStatic>,
    pub globals: Vec<MirGlobal>,
    pub routines: Vec<MirRoutine>,
    pub runtime_helpers: Vec<MirRuntimeHelperDecl>,
}

pub struct MirRoutine {
    pub id: RoutineId,
    pub name: String,
    pub abi: MirRoutineAbi,
    pub frame: MirFrame,
    pub temps: Vec<MirTemp>,
    pub blocks: Vec<MirBlock>,
    pub effects: MirEffects,
}

pub struct MirBlock {
    pub id: MirBlockId,
    pub label: String,
    pub ops: Vec<MirOp>,
    pub terminator: MirTerminator,
}
```

- Define minimal ID newtypes needed by MIR if NIR IDs cannot be reused directly.
- Keep routine names and block labels as display metadata only.
- Define `MirTerminator` with:

```rust
Jump(MirBlockId)
Branch { cond: MirCond, then_block: MirBlockId, else_block: MirBlockId }
Return
Exit
Unreachable
```

- Initially, shell lowering may use `Return`, `Exit`, `Jump`, or `Unreachable`
  according to the NIR terminator shape.

Acceptance criteria:

- MIR program/routine/block/terminator shape exists.
- MIR phase is threaded through the verifier API.
- Printer output distinguishes routines and blocks by readable names while using
  stable IDs as executable identity.

Suggested commit:

```text
mir6502: define program and phase model
```

## Milestone 3: Structural Verifier Baseline

Goal: make MIR impossible to use without basic structural validation.

Scope:

- Implement verifier diagnostics.
- Check, for all phases:

```text
- unique routine IDs;
- unique block IDs within each routine;
- every block has exactly one terminator;
- terminator targets exist;
- labels/names are metadata only;
- no source syntax or SemIR handles appear in executable MIR.
```

- Add unit tests that construct invalid MIR by hand and verify diagnostics.

Acceptance criteria:

- Valid shell MIR verifies.
- Duplicate IDs are rejected.
- Missing branch/jump targets are rejected.
- Diagnostics name the routine/block where possible.

Suggested commit:

```text
mir6502: verify structural invariants
```

## Milestone 4: Fixture Harness And Sweep Skeleton

Goal: make MIR output an independently testable contract.

Scope:

- Add:

```text
fixtures/mir6502/
tests/mir6502_fixtures.rs
```

- Add one or two tiny fixtures that exercise the shell output.
- If useful, add an initial sweep binary:

```text
src/bin/actionc-mir6502-sweep.rs
```

- The sweep may initially only lower, verify, and format MIR.

Acceptance criteria:

- `cargo test mir6502_fixtures_match_snapshots` exists and passes.
- Fixture output is deterministic.
- Sweep, if added, distinguishes parse/SemIR/NIR/MIR-lowering/MIR-verifier
  failures.

Suggested commits:

```text
mir6502: add fixture snapshots
mir6502: add sweep validation command
```

## Milestone 5: Value, Definition, Memory, And Address Model

Goal: implement the reviewed MIR value/address split from the contract.

Scope:

- Define:

```rust
pub enum MirDef {
    VTemp(MirTempId),
    Reg(MirReg),
}

pub enum MirValue {
    ConstU8(u8),
    ConstU16(u16),
    Def(MirDef),
    Word { lo: Box<MirValue>, hi: Box<MirValue> },
    StaticAddr(StaticId),
    GlobalAddr(GlobalId),
    RoutineAddr(RoutineId),
}

pub enum MirMem {
    Absolute(u16),
    Static { id: StaticId, offset: u16 },
    Global { id: GlobalId, offset: u16 },
    Local { id: LocalId, offset: u16 },
    Param { id: ParamId, offset: u16 },
    Spill { id: MirSpillId, offset: u16 },
    ZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
}

pub enum MirAddr {
    Direct(MirMem),
    Label(MirLabel),
    ZeroPageIndexedX { base: MirZpSlot },
    AbsoluteIndexedX { base: MirMem },
    AbsoluteIndexedY { base: MirMem },
    IndirectIndexedY { zp: MirZpSlot },
}
```

- Implement `MirWidth`, `MirReg`, `MirFlag`, and minimal temp/storage ID types.
- The first lowering profile should support only `MirAddr::Direct`.
- Add verifier checks:

```text
- operation definitions target MirDef, not memory;
- memory destinations are written only through Store-like operations;
- address forms reference valid storage IDs;
- Direct addresses are valid in pre-materialization MIR.
```

Acceptance criteria:

- Printer can format values, defs, memory sites, and direct addresses.
- Verifier rejects memory-as-def patterns.
- Verifier rejects invalid storage references.

Suggested commit:

```text
mir6502: define value and address model
```

## Milestone 6: Frame And Storage Mapping

Goal: map NIR storage IDs to MIR memory homes without consulting SemIR.

Scope:

- Define `MirFrame`, `MirStorageSlot`, and `MirStorageBase` as specified in the
  contract.
- Lower NIR storage facts into MIR storage homes:

```text
NirParam  -> MirMem::Param
NirLocal  -> MirMem::Local
NirGlobal -> MirMem::Global
NirStatic -> MirMem::Static
Absolute  -> MirMem::Absolute
NirTemp   -> MirDef::VTemp
```

- Keep `MirZpSlot` virtual/abstract.
- Keep fixed ABI zero-page locations separate from allocatable zero-page slots.
- Do not assign real zero-page addresses yet.

Acceptance criteria:

- MIR frame records params, locals, spills, virtual zero-page slots, and fixed
  zero-page slots.
- MIR lowering never uses source names to resolve storage.
- Fixtures show stable storage references.

Suggested commit:

```text
mir6502: map NIR storage to MIR homes
```

## Milestone 7: Operation Families

Goal: add the initial operation families from the contract.

Scope:

- Define `MirOp` variants:

```text
LoadImm
Load
Store
Move
LeaAddr
Extend
Truncate
Unary
Binary
Compare
Call
RuntimeHelper
Barrier
MachineBlock
```

- Define:

```rust
MirUnaryOp::{Neg, BitNot}
MirBinaryOp::{Add, Sub, Mul, Div, Mod, Lsh, Rsh, And, Or, Xor}
MirCompareOp::{Eq, Ne, Lt, Le, Gt, Ge}
MirCondDest::{Temp, Flags}
MirCarryIn::{Clear, Set, FromPrevious}
MirCarryOut::{Ignore, Produce}
```

- Do not add `Identity`; use `Move`.
- Do not add `LogicalNot`; lower it through compare/branch logic if it appears.
- Do not add one variant per 6502 opcode.
- Add verifier checks for widths and destination categories.

Acceptance criteria:

- Every initial op shape prints deterministically.
- Verifier rejects missing/invalid widths.
- Verifier rejects `Binary Add/Sub` in post-materialization MIR when carry facts
  are missing.

Suggested commit:

```text
mir6502: define scalar operation families
```

## Milestone 8: Effects, Barriers, And Runtime Helper Declarations

Goal: preserve conservative scheduling facts from the first real MIR ops.

Scope:

- Define:

```rust
MirEffects
MirMemoryEffect
MirMemoryRegion
MirMemoryRegionKind
MirRegisterSet
MirRuntimeHelper
MirRuntimeHelperDecl
MirRuntimeHelperTarget
```

- Include `clobbers`, `preserves`, `sp`, and `stack_depth_delta` as specified in
  the contract.
- Calls, runtime helpers, barriers, and machine blocks must carry effects.
- Runtime helper targets may be `Deferred` before pre-emission MIR only.

Acceptance criteria:

- Verifier rejects call/helper/barrier/machine-block ops without effects.
- Pre-emission verifier rejects unresolved runtime helper targets.
- Machine blocks are opaque by default.

Suggested commit:

```text
mir6502: model effects and helper declarations
```

## Milestone 9: Scalar Load And Store Lowering

Goal: lower the first real NIR executable ops into pre-materialization MIR.

Scope:

- Lower scalar NIR `Load` and `Store` for direct places only:

```text
Param
Local
Global
Static
Absolute
```

- Lower NIR constants and temp values into `MirValue`.
- Produce `MirOp::Load` and `MirOp::Store` with `MirAddr::Direct`.
- Word-width loads/stores may remain word pseudo ops in pre-materialization MIR.
- Reject unsupported places with precise diagnostics:

```text
Deref
Field
Index
unsupported machine blocks
unresolved compatibility forms
```

Acceptance criteria:

- Byte scalar assignment fixture lowers to MIR.
- Word scalar assignment fixture lowers to MIR.
- Absolute store fixture lowers to MIR.
- Verifier passes `PreMaterialization`.

Suggested commit:

```text
mir6502: lower scalar loads and stores
```

## Milestone 10: Cast And Address Materialization Lowering

Goal: lower NIR `Cast` and `AddrOf` without smuggling semantics through `Move`.

Scope:

- Lower NIR `Cast`:

```text
same width -> Move
u8 -> u16  -> Extend signed=false
i8 -> i16  -> Extend signed=true
u16 -> u8  -> Truncate
```

- Lower NIR `AddrOf` to `LeaAddr` or direct address materialization.
- Lower address values:

```text
StaticAddr
GlobalAddr
RoutineAddr
```

- Verify address materialization has word width.

Acceptance criteria:

- Cast fixture prints `Move`, `Extend`, or `Truncate` as appropriate.
- Address-of fixture prints `LeaAddr` or address values.
- Invalid address materialization is rejected.

Suggested commit:

```text
mir6502: lower casts and address materialization
```

## Milestone 11: Scalar Unary And Binary Lowering

Goal: lower simple arithmetic and logic into pre-materialization MIR.

Scope:

- Lower:

```text
Unary Neg
Unary BitNot
Binary Add
Binary Sub
Binary And
Binary Or
Binary Xor
Binary Mul
Binary Div
Binary Mod
Binary Lsh
Binary Rsh
```

- Keep expensive operations as MIR pseudo ops for now.
- Do not select runtime helpers yet.
- Carry facts may be absent for pre-materialization word `Add`/`Sub`.
- Byte-width `Add`/`Sub` may include carry facts only when already expanded.

Acceptance criteria:

- Byte arithmetic fixtures lower to MIR.
- Word arithmetic fixtures lower to MIR pseudo ops.
- No target peepholes are added.

Suggested commits:

```text
mir6502: lower scalar unary ops
mir6502: lower scalar binary ops
```

## Milestone 12: Compare And Branch Lowering

Goal: lower NIR compare/branch without duplicating compare operands in branches.

Scope:

- Lower NIR `Compare` to:

```rust
MirOp::Compare { dst: MirCondDest::Temp(...), ... }
```

- Lower NIR `Branch` to:

```rust
MirTerminator::Branch { cond: MirCond::BoolValue(...), ... }
```

- Do not use `MirCond` to copy compare operands.
- Preserve `BlockId`-based control flow.
- Normalize any NIR fallthrough before MIR; MIR has no `Fallthrough` terminator.

Acceptance criteria:

- If/while scalar fixtures lower to MIR.
- Branch targets are stable block IDs.
- Verifier rejects duplicated compare conditions.
- Compare results used as ordinary values remain materializable as bool bytes.

Suggested commit:

```text
mir6502: lower compares and branches
```

## Milestone 13: Call Placeholder Lowering

Goal: preserve call ABI/effect facts before implementing full argument packing.

Scope:

- Lower direct user/runtime calls only when NIR signatures and effects are
  complete.
- Build conservative `MirCallAbi` placeholders.
- Preserve `clobbers`, `preserves`, memory effects, OS flags, opaque flags, and
  stack effects.
- Reject indirect calls until typed callable lowering is implemented.
- Reject calls without complete signature/effect facts.

Acceptance criteria:

- Direct call fixture prints MIR call with ABI/effects.
- Opaque/runtime/OS calls are barriers.
- Verifier rejects calls without ABI/effects.

Suggested commit:

```text
mir6502: preserve direct call ABI and effects
```

## Milestone 14: Pre-Materialization Fixture Coverage

Goal: make the scalar pre-materialization profile stable.

Scope:

Add fixtures for:

```text
empty_program.act
scalar_assignment_byte.act
scalar_assignment_word.act
absolute_store.act
byte_arithmetic.act
word_arithmetic.act
casts.act
address_of.act
if_compare.act
while_compare.act
return_scalar.act
call_placeholder.act
```

Acceptance criteria:

- `cargo test mir6502_fixtures_match_snapshots` passes.
- MIR snapshots are readable and stable.
- Fixtures cover every initial op family that has lowering.

Suggested commit:

```text
mir6502: expand scalar fixture coverage
```

## Milestone 15: Materialization Pass Runner

Goal: add the pass framework that moves from pre-materialization to
post-materialization MIR.

Scope:

- Add:

```text
src/mir6502/materialize.rs
src/mir6502/passes.rs
```

- Add:

```rust
pub fn materialize_program(
    program: MirProgram,
    config: &Mir6502Config,
) -> Result<MirProgram, Vec<MirDiagnostic>>;
```

- Run verification before and after materialization:

```text
verify PreMaterialization
materialize
verify PostMaterialization
```

- Initially, the materializer may be a no-op.

Acceptance criteria:

- No-op materialization preserves existing MIR fixtures.
- Verification hooks run before and after.
- Phase-specific diagnostics identify which phase failed.

Suggested commit:

```text
mir6502: add materialization pass runner
```

## Milestone 16: Word Load/Store And Logic Expansion

Goal: byte-expand simple word operations.

Scope:

- Materialize:

```text
load.u16
store.u16
move.u16
and.u16
or.u16
xor.u16
```

- Use explicit low/high byte values and definitions.
- Do not select registers globally.
- Keep materialization local and predictable.

Acceptance criteria:

- Word load/store fixtures show byte-lane operations after materialization.
- Word logic fixtures show low/high byte operations.
- Post-materialization verifier passes.

Suggested commits:

```text
mir6502: materialize word loads and stores
mir6502: materialize word logical ops
```

## Milestone 17: Word Add/Sub Carry Chains

Goal: expand word add/sub with explicit carry and borrow behavior.

Scope:

- Materialize word `Add` and `Sub` into byte-lane ops.
- Add `MirCarryIn` and `MirCarryOut` facts to byte-lane `Add`/`Sub`.
- Ensure no flag-clobbering operation can appear between carry-chain steps.
- Add verifier checks for carry-chain validity.

Example target shape:

```text
vt0:u8 = load.u8 local(a).lo
vt1:u8 = add.u8 vt0, #1 carry_in=Clear carry_out=Produce
store.u8 local(a).lo, vt1
vt2:u8 = load.u8 local(a).hi
vt3:u8 = add.u8 vt2, #0 carry_in=FromPrevious carry_out=Ignore
store.u8 local(a).hi, vt3
```

Acceptance criteria:

- Word add/sub fixtures materialize with explicit carry facts.
- Verifier rejects implicit carry in post-materialization MIR.
- Verifier rejects impossible carry chains.

Suggested commit:

```text
mir6502: materialize word add and sub carry chains
```

## Milestone 18: Runtime Helper Selection

Goal: lower expensive pseudo ops to known runtime helper calls.

Scope:

- Select helpers for:

```text
Mul
Div
Mod
word Lsh
word Rsh
```

- Produce `MirOp::RuntimeHelper` with:

```text
MirRuntimeHelper::Mul
MirRuntimeHelper::Div
MirRuntimeHelper::Mod
MirRuntimeHelper::Lsh
MirRuntimeHelper::Rsh
```

- Attach ABI and effects.
- Keep helper target addresses deferred until the runtime configuration is known.
- Pre-emission verifier must reject unresolved helper targets.

Acceptance criteria:

- Expensive ops no longer remain generic binary ops after materialization.
- Helper calls carry ABI and effects.
- Fixtures show selected helper names.

Suggested commit:

```text
mir6502: select helpers for wide operations
```

## Milestone 19: Compare And Branch Materialization

Goal: turn compare/branch MIR into flag-aware or explicit control-flow MIR.

Scope:

- Start with:

```text
u8 Eq/Ne
bool branch
constant bool branch, if still present
```

- Then add:

```text
u8 unsigned relational
u16 Eq/Ne
u16 unsigned relational
i16 signed relational
```

- Compare results used only by a branch may become `FlagTest` or `FusedCompare`.
- Compare results used as ordinary values must still produce a bool byte.
- Multi-step word and signed comparisons may lower to small MIR control-flow
  sequences rather than a single flag test.

Acceptance criteria:

- Byte equality and inequality branch fixtures materialize.
- Word equality branch fixtures materialize.
- Signed relational behavior has focused tests before it is enabled.

Suggested commits:

```text
mir6502: materialize byte equality branches
mir6502: materialize byte relational branches
mir6502: materialize word equality branches
mir6502: materialize word relational branches
```

## Milestone 20: Call ABI Planner

Goal: replace placeholder calls with an ABI-driven call plan.

Scope:

- Add:

```text
src/mir6502/abi.rs
src/mir6502/call_plan.rs
```

- Inputs:

```text
NirSignature
NirCallee
NirEffects
MIR values for args
```

- Outputs:

```text
MirCallAbi
Vec<MirArgHome>
Option<MirResultHome>
MirEffects
```

- Support in this order:

```text
1. direct user calls
2. runtime calls
3. builtin calls through explicit mapping
4. indirect calls
```

- Keep argument packing in one place.
- Preserve left-to-right evaluation where effects require it.

Acceptance criteria:

- Direct user call fixtures show concrete ABI homes.
- Runtime call fixtures show concrete ABI homes.
- Calls carry clobbers, preserves, stack effects, and memory effects.
- Verifier rejects calls without ABI/effects.

Suggested commits:

```text
mir6502: add call ABI planner
mir6502: lower direct user calls through ABI planner
mir6502: lower runtime calls through ABI planner
```

## Milestone 21: Pre-Emission Verifier Profile

Goal: define the subset of MIR that can feed tracked emission.

Scope:

- Strengthen `MirPhase::PreEmission` checks:

```text
- no unsupported pseudo ops;
- no unresolved storage or labels;
- no unassigned virtual temps;
- no unresolved runtime helper targets;
- no abstract zero-page slots unless emission explicitly owns final assignment;
- all raw data/machine-code boundaries represented as barriers;
- ordinary instruction work expressible through tracked emission helpers.
```

- Add tests for each rejected pre-emission violation.

Acceptance criteria:

- Pre-emission verifier rejects unresolved helpers.
- Pre-emission verifier rejects unassigned virtual temps.
- Pre-emission verifier rejects abstract compare conditions.

Suggested commit:

```text
mir6502: verify pre-emission invariants
```

## Milestone 22: MIR To Tracked Emission Bridge

Goal: emit the first scalar pre-emission MIR through the existing tracked emitter.

Scope:

- Add:

```text
src/mir6502/emit.rs
```

- Expose:

```rust
pub fn emit_program(
    mir: &MirProgram,
    emitter: &mut NativeTrackedEmitter,
) -> Result<(), Vec<MirDiagnostic>>;
```

- Start with byte-width direct operations only:

```text
LoadImm byte
Load byte Direct
Store byte Direct
Move byte
byte Add/Sub with explicit carry
byte And/Or/Xor
Jump
Branch FlagTest
Return
Exit
```

- Do not bypass tracked emission for ordinary instructions.
- Raw data and machine blocks remain barriers.

Acceptance criteria:

- Scalar MIR emits bytes through tracked emitter.
- Pre-emission verifier runs before emission.
- Existing emission state tracking remains centralized.

Suggested commits:

```text
mir6502: emit byte load and store operations
mir6502: emit byte arithmetic operations
mir6502: emit branches and labels
mir6502: emit returns and exits
```

## Milestone 23: End-To-End Backend Profile

Goal: add an end-to-end MIR6502 backend mode for scalar programs.

Scope:

- Add backend profile or CLI selection:

```text
--backend mir6502
```

- Pipeline:

```text
parse
semantic model
SemIR
NIR
verify NIR
optional safe NIR optimization
verify NIR
lower MIR6502
verify MIR PreMaterialization
materialize MIR6502
verify MIR PostMaterialization
prepare pre-emission MIR
verify MIR PreEmission
emit through tracked emitter
```

- Keep existing backend paths as comparison oracles.
- Integrate with `actionc-compare` later if needed.

Acceptance criteria:

- Tiny scalar programs compile through MIR6502 backend.
- Failures produce diagnostics, not panics.
- Existing backend behavior is not changed unless explicitly selected.

Suggested commit:

```text
mir6502: add scalar backend profile
```

## Later Expansion Order

After scalar end-to-end works, expand in this order:

```text
1. direct user calls with real ABI packing
2. runtime helper calls with concrete targets
3. static data and string addresses
4. absolute memory and SET-like behavior
5. arrays with constant indexes
6. arrays with dynamic indexes
7. pointer dereference
8. record fields
9. indirect calls
10. machine blocks
11. zero-page allocation
12. MIR peepholes
```

Do not start these before the scalar path is verified end-to-end unless a small
feature is required to unblock the scalar path.

## Deferred Work

The following are explicitly deferred by the MIR contract and should not be
implemented during the first scalar path:

- complete 6502 opcode-level pseudo ISA;
- full zero-page placement;
- general register allocation;
- dynamic indexed array addressing;
- pointer dereference address staging;
- signed relational compare sequences beyond focused tested slices;
- machine-block payload preservation;
- indirect calls;
- target peepholes;
- final opcode scheduling.

## Suggested First Codex Task

Use this as the first implementation task after this plan lands:

```text
Implement MIR6502 Milestone 1 only.

Goal:
- Add src/mir6502 with mod.rs, ir.rs, lower.rs, verify.rs, printer.rs, diagnostics.rs.
- Define MirProgram, MirRoutine, MirBlock, MirTerminator, MirPhase, and MirDiagnostic.
- Add lower_program that creates routine/block shells from verifier-clean NIR.
- Add verify_program with structural checks for unique routine/block IDs and valid terminator targets.
- Add format_program with stable, readable output.
- Add --emit-mir6502 CLI flag.
- Add one tiny MIR fixture and fixture test if the fixture harness can be added without broad changes.
- Do not implement operation lowering, storage mapping, materialization, arithmetic, calls, or emission.

Required checks:
- cargo test
- existing NIR fixture/sweep checks, if present
- new MIR fixture test, if introduced

Commit after this slice before starting another milestone.
Suggested commit message:
- mir6502: add MIR observation surface
```
