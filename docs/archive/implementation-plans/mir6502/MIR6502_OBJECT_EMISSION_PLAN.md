# MIR6502 Object Emission Plan

Snapshot date: 2026-06-02.

This note is the implementation plan for emitting object code from verified
MIR6502. It is intended to be used directly as a Codex execution plan after the
MIR6502 scalar and full-language MIR lowering work has produced verified
pre-emission MIR.

Canonical references:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md` defines the MIR6502 machine model,
  phase model, verifier expectations, value/address split, effects, and
  pre-emission requirements.
- `docs/MIR6502_IMPLEMENTATION_PLAN.md` defines the scalar-first MIR6502 plan.
- `docs/MIR6502_FULL_LANGUAGE_EXPANSION_PLAN.md` defines the post-scalar feature
  expansion plan.
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md` defines the MIR6502-to-tracked
  emission interface contract.
- `docs/SEMIR_NATIVE_EMISSION_PLAN.md` describes the existing tracked-emission
  layer built around `NativeTrackedEmitter`.

This plan must not weaken those contracts. If emission needs a semantic,
storage, ABI, helper, aliasing, or control-flow fact that is not present in
pre-emission MIR, fix MIR lowering/materialization/verification first. Do not
recover that fact in tracked emission.

## North Star

Object emission from MIR6502 should be boring and deterministic.

The pipeline is:

```text
verifier-clean NIR
  -> MIR6502 lowering
  -> MIR6502 materialization
  -> verify MIR PreEmission
  -> tracked emission helpers
  -> concrete bytes / load file / maps
```

MIR6502 decides target strategy. Tracked emission writes concrete bytes for
already-decided actions and records processor-state consequences.

Emission should ask:

```text
which bytes encode this already-decided MIR action?
how should tracked state change after those bytes?
```

Emission should not ask:

```text
what source construct produced this?
what storage kind is this really?
what ABI home should this argument use?
should this operation become a helper call?
can I ignore this barrier?
```

## Codex Execution Rule

Execute this plan as small, test-gated slices.

For every milestone below:

1. make only the changes required for that milestone;
2. require or assert `MirPhase::PreEmission` before writing bytes;
3. route ordinary instructions through tracked emission helpers;
4. preserve existing MIR fixtures and byte tests;
5. add focused byte-level tests for each newly emitted op family;
6. add diagnostics for unsupported or unresolved pre-emission forms;
7. commit before starting the next milestone;
8. use commit messages of the form `mir6502: <short imperative summary>`.

Do not mix object-layout changes, new op emission, peepholes, and feature
lowering in one commit. If a milestone is too large, split it into compiling
sub-slices.

## Required Checks

Run the relevant checks after each slice:

```sh
cargo test
cargo test mir6502_fixtures_match_snapshots
```

If a MIR sweep exists, run:

```sh
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
```

If end-to-end MIR backend support exists, also run representative object-output
checks, for example:

```sh
cargo run --bin actionc-emit -- --backend mir6502 --emit-code fixtures/mir6502/<fixture>.act
cargo run --bin actionc-emit -- --backend mir6502 --emit-load fixtures/mir6502/<fixture>.act
```

If a check cannot be run in the current environment, say so in commit or PR
notes and list the checks that still need manual execution.

## Red Lines

Emission must not:

- inspect SemIR;
- parse printed NIR, TAC, or MIR;
- look up source names;
- decide whether storage is local, global, static, absolute, indexed,
  dereferenced, descriptor-backed, or pointer-backed;
- decide whether an absolute-backed global alias is ordinary storage;
- decide ABI argument/result homes;
- infer call clobbers, preserves, memory effects, stack effects, or opacity;
- choose runtime helpers for arithmetic or shifts;
- decide compare/branch fusion;
- repair invalid carry chains;
- perform general register allocation;
- silently change indexed/indirect addressing strategy;
- bypass tracked emission for ordinary instructions.

If one of those decisions is missing, pre-emission MIR is not concrete enough.
Reject with a diagnostic and fix the MIR pipeline.

## Milestone E0: Hard Pre-Emission Gate

Goal: make it impossible to emit from the wrong MIR phase.

Scope:

- Require:

```rust
verify_program(mir, MirPhase::PreEmission)?;
```

before emitting any byte.

- Make the MIR-to-emission bridge return `Vec<MirDiagnostic>` on failure.
- Include routine/block/op context in diagnostics where possible.
- Do not let emission fallback to pre-materialization or post-materialization
  behavior.

Acceptance criteria:

- Non-pre-emission MIR is rejected before byte output.
- Diagnostics explain which pre-emission invariant failed.
- No emission helper tries to recover missing storage, ABI, helper, or effect
  facts.

Suggested commit:

```text
mir6502: require pre-emission MIR before emitting
```

## Milestone E1: Object Layout Context

Goal: replace ad hoc address assumptions with an explicit object-layout context.

Scope:

Add an emission context, for example:

```rust
pub struct MirEmitContext<'a> {
    pub origin: u16,
    pub layout: MirObjectLayout,
    pub labels: MirLabelTable,
    pub diagnostics: Vec<MirDiagnostic>,
    pub mir: &'a MirProgram,
}

pub struct MirObjectLayout {
    pub routine_labels: Vec<MirLabel>,
    pub block_labels: Vec<MirLabel>,
    pub globals: Vec<MirStoragePlacement>,
    pub statics: Vec<MirStaticPlacement>,
    pub helpers: Vec<MirHelperPlacement>,
}

pub enum MirStoragePlacement {
    Absolute { address: u16 },
    LabelRelative { label: MirLabel, offset: u16 },
    ZeroPage { address: u8 },
}
```

The exact Rust shape may differ. The invariant matters: emitted memory
references must resolve through a single object-layout layer.

The layout layer must resolve:

```text
MirMem::Absolute
MirMem::Global
MirMem::Static
MirMem::Local
MirMem::Param
MirMem::Spill
MirMem::ZeroPage
MirMem::FixedZeroPage
```

Rules:

- `MirMem::Absolute` resolves to the exact address.
- Absolute-backed globals resolve to their absolute address or to a global table
  entry that clearly records absolute backing.
- Ordinary globals/statics get deterministic object placement.
- Locals/params/spills must have concrete placement before emission or be
  rejected.
- Do not use a hardcoded global/static data base as the long-term object-layout
  policy.

Acceptance criteria:

- Every emitted memory reference resolves through `MirObjectLayout` or an
  equivalent context.
- Absolute-backed globals emit to the real absolute address.
- Unresolved placement produces a MIR diagnostic before bytes are written for the
  offending op.
- Existing scalar emission tests still pass after updating expected addresses if
  the layout policy intentionally changes.

Suggested commit:

```text
mir6502: add object layout context
```

## Milestone E2: Storage And Static Data Emission

Goal: emit byte-exact data/storage payloads before routine code depends on them.

Scope:

Support layout and emission for:

```text
ordinary global storage
absolute-backed aliases
static data bytes
string/static literals
initialized scalar data
zero-filled storage
descriptor/backing data already represented in MIR
```

Rules:

- Absolute aliases do not allocate object storage.
- Static/global initialized bytes come from MIR payloads, not source text.
- Zero-fill must be explicit in layout/map output.
- Descriptor/backing references must point to valid object placements.
- Emission must not reconstruct static bytes by parsing display strings.

Acceptance criteria:

- `BYTE COLOR=$02C8` does not allocate normal object storage.
- Ordinary `BYTE x` allocates one byte in the chosen data area.
- Static/string bytes appear exactly once.
- Storage maps list global/static placements.
- Emission diagnostics catch missing initialized payloads or invalid backing refs.

Suggested commits:

```text
mir6502: emit global storage bytes
mir6502: emit static data bytes
mir6502: preserve absolute aliases during layout
```

## Milestone E3: Routine And Block Label Emission

Goal: establish routine/block identity in emitted bytes.

Scope:

- Bind routine entry labels.
- Bind block labels.
- Preserve MIR block target identity.
- Record routine addresses and ranges in `CodegenMap` when possible.
- Report unresolved label diagnostics from emission finish/patching.

Rules:

- Emission may bind and patch labels.
- Emission must not change a branch or jump target.
- Block order in MIR is a layout hint, not semantic identity.
- Any block reordering should be owned by a MIR layout/materialization pass or a
  documented emission layout policy.

Acceptance criteria:

- Every emitted MIR block label is bound.
- Jumps and branches patch to the intended block.
- Routine address map entries are populated.
- Unresolved labels produce diagnostics, not panics.

Suggested commit:

```text
mir6502: emit routine and block labels
```

## Milestone E4: Direct Byte Operation Emission

Goal: support the first useful object-code subset.

Scope:

Emit byte-width direct operations:

```text
LoadImm byte
Load byte Direct
Store byte Direct
Move byte
byte Add/Sub with explicit carry
byte And/Or/Xor
Compare byte
Jump
Branch FlagTest
Return
Exit
Barrier
```

Rules:

- Emit through `NativeTrackedEmitter` or tracked helper facades.
- `MirMem::Absolute` stores must emit to the exact address.
- Global/static/local/param/spill direct stores must resolve through object
  layout.
- Byte `Add`/`Sub` must honor `MirCarryIn`.
- Unsupported operand forms produce diagnostics.

Acceptance criteria:

- Direct byte stores to absolute addresses emit correct `STA` targets.
- Direct byte loads/stores to placed globals/statics emit through layout.
- Byte `Add` with `carry_in=Clear` emits carry setup before addition.
- Byte `Sub` with `carry_in=Set` emits borrow setup before subtraction.
- Branches support at least `ZSet` and `ZClear` before adding other flags.

Suggested commits:

```text
mir6502: emit direct byte loads and stores
mir6502: emit direct byte arithmetic
mir6502: emit byte compares and flag branches
mir6502: emit returns and exits
```

## Milestone E5: Materialized Word Byte-Lane Emission

Goal: emit word operations only after MIR has already expanded them.

Scope:

Support byte-lane sequences produced by MIR materialization:

```text
word load/store as low/high byte ops
word add/sub as byte add/sub carry chains
word logic as low/high byte logic
word compare as MIR-selected compare/control-flow sequence
```

Rules:

- Pre-emission MIR should not contain generic word pseudo ops.
- Emission must not split a word op on its own.
- Carry chains must already be explicit and verified.
- Emission must not insert flag-clobbering work between carry-chain steps.

Acceptance criteria:

- Word store fixture emits low/high bytes in the correct order.
- Word add/sub fixture emits the materialized carry chain.
- Word logic fixture emits low/high byte logic.
- Verifier catches generic word pseudo ops before emission.

Suggested commit:

```text
mir6502: emit materialized word byte lanes
```

## Milestone E6: Direct Memory Form Coverage

Goal: support all direct memory forms that MIR has already placed.

Scope:

Resolve and emit:

```text
MirAddr::Direct(MirMem::Absolute)
MirAddr::Direct(MirMem::Global)
MirAddr::Direct(MirMem::Static)
MirAddr::Direct(MirMem::Local)
MirAddr::Direct(MirMem::Param)
MirAddr::Direct(MirMem::Spill)
MirAddr::Direct(MirMem::FixedZeroPage)
MirAddr::Direct(MirMem::ZeroPage)
```

Rules:

- `Global`, `Static`, `Local`, `Param`, and `Spill` resolve through
  `MirObjectLayout`.
- `FixedZeroPage` resolves through known fixed ABI addresses.
- Virtual `ZeroPage` resolves only after zero-page allocation or a documented
  emission-owned final assignment policy.
- `Absolute` remains exact.

Acceptance criteria:

- Absolute aliases emit exact addresses.
- Locals/params emit from assigned routine storage homes.
- Fixed zero-page homes use zero-page opcodes where MIR permits that encoding.
- Unresolved direct memory placement fails before writing bytes for the op.

Suggested commits:

```text
mir6502: resolve direct memory placements
mir6502: emit fixed zero-page direct memory
mir6502: reject unresolved direct memory placement
```

## Milestone E7: Indexed And Indirect Address Form Emission

Goal: support arrays, pointer deref, descriptor-backed indexing, and staged
addresses after MIR has selected the address strategy.

Scope:

Emit selected address forms:

```text
MirAddr::AbsoluteIndexedX
MirAddr::AbsoluteIndexedY
MirAddr::ZeroPageIndexedX
MirAddr::IndirectIndexedY
```

Rules:

- Emission must not change the selected address strategy.
- Emission may choose shorter direct zero-page encodings only when the operation
  is non-indexed and byte-equivalent.
- Emission must not silently replace absolute-indexed with zero-page-indexed.
- Zero-page indexed wraparound semantics must be guarded.

Acceptance criteria:

- Inline byte-array dynamic indexing emits the selected absolute-indexed form.
- Pointer/descriptor-backed dynamic indexing emits the selected indirect-indexed
  form.
- A zero-page-indexed wraparound guard test exists.
- Unsupported indexed forms fail with diagnostics.

Suggested commits:

```text
mir6502: emit absolute indexed address forms
mir6502: emit indirect indexed address forms
mir6502: guard zero-page indexed wraparound semantics
```

## Milestone E8: Calls And Runtime Helper Emission

Goal: write call byte-code from already-planned ABI homes and effects.

Scope:

Emit:

```text
direct user calls
runtime helper calls
builtin calls lowered to concrete call targets
OS calls
indirect calls after MIR has selected the strategy
```

Emission may:

- write `JSR` to a routine/helper/absolute target;
- write a concrete call sequence already selected by MIR;
- invalidate tracked state from `MirEffects`;
- record routine effects/map entries.

Emission must not:

- choose ABI homes;
- infer call signatures;
- infer clobbers or preserves;
- decide helper selection;
- decide indirect-call trampoline strategy.

Acceptance criteria:

- Direct procedure call emits `JSR` to the routine label/address.
- Runtime helper call emits `JSR` to resolved helper target.
- Builtin/OS calls carry and apply effects.
- Unresolved helper targets are rejected before emission.

Suggested commits:

```text
mir6502: emit direct routine calls
mir6502: emit runtime helper calls
mir6502: emit builtin and OS call targets
mir6502: apply call effects during emission
```

## Milestone E9: Machine Blocks And Raw Payload Emission

Goal: preserve raw machine-code/data payloads only when MIR has structured
payloads and effects.

Scope:

Emit structured machine-block items:

```text
raw byte
raw word
label definition
label reference
global/static/routine reference
zero fill
```

Rules:

- Machine blocks are barriers by default.
- Emission writes raw bytes and invalidates tracked state according to effects.
- Emission must not parse raw source text.
- Unsupported machine-block payloads must fail before emission.

Acceptance criteria:

- Structured machine block bytes are emitted.
- Label/global/static/routine references patch correctly.
- Raw payloads are reflected in maps/listings.
- Unsupported machine blocks fail before emission.

Suggested commits:

```text
mir6502: emit structured machine block payloads
mir6502: patch machine block references
mir6502: invalidate tracked state across machine blocks
```

## Milestone E10: CodegenOutput And Map Population

Goal: make MIR backend output comparable with existing backends.

Scope:

Populate `CodegenOutput` and `CodegenMap` fields progressively:

```text
origin
run_address
skipped_ranges
routine_addresses
routine_ranges
storage_symbols
source_ranges
routine_effects
machine_blocks
optimizations
proofs / proof_attempts, if applicable
```

Rules:

- Map entries should be tied to emitted bytes and MIR/source metadata where
  available.
- Empty map fields are acceptable only for facts that MIR/emission does not yet
  model.
- Keep existing compare tooling usable.

Acceptance criteria:

- `--emit-code` works for MIR backend.
- `--emit-load` works for MIR backend.
- `--emit-listing` has useful routine/block labels.
- `--emit-map` shows storage symbols and routine ranges.
- `actionc-compare` can compare MIR6502 output to existing profiles where
  supported.

Suggested commits:

```text
mir6502: populate routine codegen map entries
mir6502: populate storage codegen map entries
mir6502: populate source ranges for MIR emission
```

## Milestone E11: End-To-End Emitted-Byte Fixtures

Goal: test emitted bytes, not just printed MIR.

Add byte-level tests for:

```text
scalar_byte_store
absolute_alias_store
absolute_set_store
byte_add
byte_sub
byte_logic
word_store
word_add
if_eq_branch
if_ne_branch
while_loop
direct_call
runtime_helper_call
inline_byte_array_const_index
inline_byte_array_dynamic_index
pointer_deref_store
record_field_store
machine_block_bytes
```

Rules:

- For small programs, assert exact bytes.
- For larger programs, compare maps/ranges and use compare tooling.
- Unsupported forms should report diagnostics, not partial output.

Acceptance criteria:

- Emitted bytes are deterministic.
- The `COLOR=$02C8` regression emits a store to `$02C8`.
- Every newly supported emission op family has a focused byte test.
- Unsupported MIR never produces silent partial object code.

Suggested commit:

```text
mir6502: add emitted byte regression fixtures
```

## Milestone E12: Object Emission Sweep

Goal: make emission failures visible across fixture directories.

Add or extend:

```text
actionc-mir6502-sweep
```

Suggested modes:

```text
--emit-mir
--materialize
--pre-emission
--emit-code
--emit-load
```

Report categories separately:

```text
parse failure
semantic failure
NIR lowering failure
NIR verifier failure
MIR lowering failure
MIR verifier failure
materialization failure
pre-emission verifier failure
emission failure
finish/patch failure
```

Acceptance criteria:

- Sweep can run all MIR fixtures through object emission.
- Unsupported features are clearly counted.
- No panics on unsupported MIR.

Suggested commit:

```text
mir6502: sweep object emission fixtures
```

## Cross-Cutting Emission Verifier Work

As object emission expands, strengthen the verifier and diagnostics so malformed
MIR cannot reach tracked emission.

Verifier and bridge checks should eventually cover:

```text
pre-emission phase is required before emitting
all memory placements are resolved
absolute aliases are preserved or resolved to exact addresses
no generic word pseudo ops remain
byte add/sub carry chains are explicit and unbroken
no virtual temps remain unless intentionally owned by emission
runtime helper targets are resolved
call ABI/effects are present
machine blocks have structured payloads/effects or are rejected
indexed/indirect address strategy is selected before emission
raw data and machine-code boundaries are barriers
ordinary instructions are emitted through tracked helpers
```

If a new emission feature does not add or preserve a verifier rule that prevents
bad MIR from reaching byte output, the slice is incomplete.

## Suggested Initial Emission Commit Series

Use this sequence after verified pre-emission MIR is available:

```text
mir6502: require pre-emission MIR before emitting
mir6502: add object layout context
mir6502: emit global storage bytes
mir6502: emit static data bytes
mir6502: emit routine and block labels
mir6502: emit direct byte loads and stores
mir6502: emit direct byte arithmetic
mir6502: emit byte compares and flag branches
mir6502: emit materialized word byte lanes
mir6502: resolve direct memory placements
mir6502: emit absolute indexed address forms
mir6502: emit indirect indexed address forms
mir6502: emit direct routine calls
mir6502: emit runtime helper calls
mir6502: emit structured machine block payloads
mir6502: populate codegen map entries
mir6502: add emitted byte regression fixtures
mir6502: sweep object emission fixtures
```

## Suggested First Codex Task

Use this as the first object-emission task:

```text
Implement MIR6502 object emission Milestone E1 only.

Goal:
- Add an explicit MirEmitContext / MirObjectLayout layer.
- Stop resolving ordinary globals/statics through ad hoc hardcoded address math in emit.rs.
- Resolve MirMem through layout placement.
- Preserve MirMem::Absolute exactly.
- Add diagnostics for unresolved memory placement.
- Do not add new op emission support yet.
- Do not change MIR lowering or materialization except where needed to expose
  storage placement facts.

Required checks:
- cargo test
- cargo test mir6502_fixtures_match_snapshots
- existing MIR emission tests, if present

Commit after this slice before starting another milestone.
Suggested commit message:
- mir6502: add object layout context
```
