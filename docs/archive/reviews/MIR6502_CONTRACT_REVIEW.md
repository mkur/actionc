# MIR6502 Pseudo-Machine Contract — Review

Snapshot date: 2026-06-01.

Review of `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md` against the stated
responsibilities of NIR, MIR6502, and emission layers. Cross-referenced with
`docs/NIR_TARGET_SHAPE.md` and `docs/NIR_MIGRATION_PLAN.md`.
`docs/TAC_BOUNDARY_FOR_6502_MIR.md` was read as historical context only.

Assumptions:

- NIR is assumed to be verifier-clean and implemented.
- MIR6502 should consume NIR only.
- MIR6502 must not inspect SemIR or parse printed IR to recover missing facts.
- NIR owns source-language meaning.
- MIR6502 owns 6502 lowering strategy.
- Emission owns final opcode bytes, label patching, load-file writing, source
  maps, and tracked processor-state updates.

---

## Must-Fix Issues

### MF-1 — `MirCond::Compare` duplicates NIR compare semantics

`MirCond::Compare` carries `op`, `left`, `right`, `width`, `signed` — this is
an exact copy of the NIR `Compare` operation. A branch that consumes a compare
result should reference the temp that holds it, not re-embed the compare.
Pre-materialization MIR should use `MirCond::BoolValue(temp)` where the temp
was produced by a `MirOp::Compare`; post-materialization should use
`MirCond::FlagTest`. The full `Compare` variant inside `MirCond` creates two
places that own the same fact and lets them diverge.

Patch suggestion: remove `MirCond::Compare`; keep only `BoolValue` (pre-mat)
and `FlagTest` (post-mat). If fusion needs to remember which compare fed a
branch, add a `MirCond::FusedCompare { compare_op_ref: MirOpRef, flag_test:
MirFlagTest }` that references the producing op rather than duplicating its
operands.

### MF-2 — Post-materialization example uses real 6502 mnemonics, not pseudo-ops

The post-materialization example shows `lda`, `clc`, `adc`, `sta`. This is
emission-level output, not MIR. If MIR actually contains `lda`/`sta` then:

- the `MirOp` enum needs a variant per 6502 instruction (premature full
  pseudo-ISA);
- emission has nothing left to decide, violating the stated emission
  responsibilities.

Post-materialization MIR should still use `MirOp::Load`, `MirOp::Binary { op:
Add }`, `MirOp::Store` — just with byte-lane-split locations and carry
semantics made explicit.

Patch suggestion: replace the post-mat example with something like:

```text
vt0:u8 = load local(a).lo
vt1:u8 = add.u8 vt0, #1          ; sets carry
store local(a).lo, vt1
vt2:u8 = load local(a).hi
vt3:u8 = adc.u8 vt2, #0          ; consumes carry
store local(a).hi, vt3
```

Add a note that `adc` is one of the few carry-aware pseudo-ops MIR needs,
distinct from plain `add`.

### MF-3 — No carry / borrow model anywhere

The 6502's multi-byte arithmetic is carry-chain-dependent. The document
mentions `MirWidth::Word` and byte-lane expansion but never defines how carry
flows between the low and high byte operations. Without this:

- the verifier cannot check that a word add was correctly expanded;
- post-materialization MIR is ambiguous about whether `Add` means `ADC` (with
  carry in) or `CLC; ADC`.

Patch suggestion: add a `MirCarry` model:

```rust
pub enum MirCarryIn {
    Clear,          // CLC before add
    Set,            // SEC before sub
    FromPrevious,   // consume carry from prior op
}
```

Thread it through `Binary { carry_in: Option<MirCarryIn> }` for `Add`/`Sub` at
byte width after materialization. Add a verifier rule: byte-width `Add`/`Sub`
in post-mat MIR must have an explicit `carry_in`.

### MF-4 — `MirEffects` is missing stack-pointer / stack-depth effects

`MirRegisterSet` models A, X, Y, flags — but 6502 `JSR`/`RTS` and any
push/pull use the stack. A call clobbers the stack pointer implicitly; nested
calls and interrupt-like patterns (OS calls) can clobber stack contents. The
effects model cannot express "this call pushes N bytes" or "stack depth must be
balanced."

Patch suggestion: add `sp: bool` to `MirRegisterSet` and add an optional
`stack_depth_delta: Option<i8>` to `MirEffects` for calls. Even if the first
slice treats all calls as opaque, the field should exist so the verifier can
assert stack-depth balance across a routine when it matters.

### MF-5 — No `Cast` operation in `MirOp`

NIR has `Cast`. The MIR op enum has no cast. NIR casts like `u8 → u16`
(zero-extend) and `i8 → i16` (sign-extend) are real 6502 work (the high byte
is `#0` or the sign-extended pattern). If MIR silently handles this inside
`Move`, the verifier can't distinguish a plain copy from a width-changing cast
and can't check that sign-extension was done.

Patch suggestion: add `MirOp::Extend { dst, src, from_width, to_width, signed:
bool }` and `MirOp::Truncate { dst, src }`. Keep the set minimal — these two
cover all Action! cast paths.

### MF-6 — `MirOp::Store` takes `MirValue` as `src` but `MirOp::Load` returns to `MirLoc`

`Load.dst` is `MirLoc`, but `MirLoc` includes `Reg`, `VTemp`, `Spill`,
`ZeroPage`, `Absolute`, and various storage byte variants. This means a load's
destination could be `MirLoc::Absolute(0xD40A)` — a hardware register — which
is semantically a *store* side-effect, not a load destination. The
location/value asymmetry is under-specified.

Patch suggestion: split `MirLoc` into `MirDef` (things that can receive a
result: `VTemp`, `Reg`) and `MirMem` (memory locations that can be loaded from
or stored to). A load from memory into a temp is `Load { dst: MirDef, src:
MirMem }`; a store is `Store { dst: MirMem, src: MirValue }`. Reg-to-reg and
reg-to-mem are `Move`.

---

## Should-Fix Issues

### SF-1 — `MirCondDest` is referenced but never defined

`MirOp::Compare` uses `dst: MirCondDest` but the type is never defined. It is
unclear whether a compare result goes to a virtual temp, a flag set, or both.
Define it:

```rust
pub enum MirCondDest {
    Temp(MirTempId),
    Flags,            // result lives only in processor flags
}
```

### SF-2 — `MirAddr` and `MirLoc` overlap heavily

`MirAddr::Absolute(u16)` vs `MirLoc::Absolute(u16)`, `MirAddr::ZeroPage` vs
`MirLoc::ZeroPage`, etc. The document never explains when to use which. This
will cause inconsistent usage across the codebase.

Suggestion: define `MirAddr` as "addressing mode for a load/store instruction"
and `MirLoc` as "where a value currently lives." Add a one-paragraph rule and
remove the storage-byte variants (`StaticByte`, `GlobalByte`, `ParamByte`,
`LocalByte`) from `MirLoc` since those are address-mode concerns, not
value-residence concerns.

### SF-3 — `MirUnaryOp::LogicalNot` is source-level, not target-level

Logical-not is a source concept that NIR should have already lowered to
`compare temp, #0 → bool`. Keeping it in MIR means MIR must decide how to
lower it (xor? compare-and-branch?) — duplicating NIR's responsibility.

Suggestion: remove `LogicalNot` from `MirUnaryOp`. If NIR delivers a
`LogicalNot`, the NIR-to-MIR lowering should expand it inline.

### SF-4 — `MirUnaryOp::Identity` is a no-op that shouldn't be a pseudo-op

If identity is needed, a `Move` with the same width suffices. A separate
`Identity` unary op adds an op the verifier must handle but that does no work.

### SF-5 — No `AddrOf` in `MirOp`

NIR has `AddrOf`. The MIR op set doesn't. Address-of is real 6502 work:
materializing a 16-bit address into a register pair or zero-page location.
Without an explicit op, the lowering must smuggle it through `LoadImm` with a
symbolic address, losing the distinction between "load a constant" and
"materialize the address of a storage location whose final address isn't known
yet."

Suggestion: add `MirOp::LeaAddr { dst: MirLoc, target: MirAddr, width:
MirWidth }` (load effective address).

### SF-6 — No `MirMemoryRegion` definition

`MirMemoryEffect::Regions(Vec<MirMemoryRegion>)` references `MirMemoryRegion`
which is never defined. Even a minimal definition is needed:

```rust
pub struct MirMemoryRegion {
    pub kind: MirMemoryRegionKind,
    pub offset: u16,
    pub size: u16,
}
pub enum MirMemoryRegionKind {
    Local(LocalId),
    Global(GlobalId),
    Static(StaticId),
    AbsoluteRange,
    ZeroPage,
}
```

### SF-7 — `MirFrame` is referenced but never defined

`MirRoutine` contains `frame: MirFrame` but the document never defines
`MirFrame`. This is the structure that would hold local slot layout, spill area
size, and total frame size — all critical for ABI and call lowering.

### SF-8 — `Fallthrough` terminator from TAC is missing from MIR terminators

NIR_TARGET_SHAPE says `Fallthrough` should become `Return(None)` or a
documented terminator. The MIR contract has `Return` but doesn't mention how
`Fallthrough` maps. Add a note: "NIR `Fallthrough` must be normalized to
`Return` before MIR lowering."

---

## Open Design Questions

### OQ-1 — When does register allocation happen?

The document defers "general register allocation" but doesn't say whether
post-materialization MIR is pre- or post-regalloc. The `MirLoc::Reg` variant
suggests post-mat MIR can contain physical register assignments, but the
verifier section doesn't require them. Clarify: is there a post-regalloc phase
between post-materialization and pre-emission, or does materialization include
register allocation?

### OQ-2 — How do word-width compare/branch sequences work on 6502?

A 16-bit signed comparison on the 6502 is a multi-instruction sequence (compare
high bytes, branch, compare low bytes). The document says post-mat branches
should prefer flag tests, but a single `MirFlagTest` can't represent the
multi-step nature of a 16-bit signed compare. Does this become a runtime
helper? A macro-op? A sequence of MIR ops with a compound flag condition?

### OQ-3 — Where does zero-page allocation live?

`MirLoc::ZeroPage(MirZpSlot)` and `MirAddr::ZeroPage(MirZpSlot)` assume slots
are already assigned. But zero-page is a global scarce resource. Is `MirZpSlot`
an abstract virtual slot (allocated later) or a concrete ZP address? If
concrete, who assigns it and when? This needs a policy note.

### OQ-4 — How does `MirRuntimeHelperDecl` relate to `MirOp::RuntimeHelper`?

`MirProgram` has `runtime_helpers: Vec<MirRuntimeHelperDecl>` and
`MirOp::RuntimeHelper` references `MirRuntimeHelper`. Neither type is defined.
Clarify: is `MirRuntimeHelper` an ID into the `runtime_helpers` table, or a
standalone enum of known helpers (multiply, divide, etc.)?

### OQ-5 — Should MIR blocks be ordered or is layout an emission concern?

The document doesn't say whether MIR block order is significant. On 6502,
block layout directly affects branch distances (branches are ±127 bytes). Is
block ordering a post-materialization MIR concern or purely emission?

---

## Wording / Documentation Improvements

### WD-1 — Add a "Definitions" section

Terms like "materialization," "home," "barrier," "pre-emission" are used
throughout but never formally defined. A short glossary at the top would
prevent misreading.

### WD-2 — The "Two MIR Phases" section describes two phases but the verifier section describes three

The body describes pre-mat and post-mat. The verifier section adds
"Pre-emission MIR." Either make the phases section say three, or explain that
pre-emission is a sub-phase of post-mat.

### WD-3 — `MirRoutine.name` is `String`, not `Option<String>`

The contract says "MIR6502 may keep display names only as diagnostics" but
`name` is non-optional. Make `name` optional or add a note that it's always
populated but never used for executable dispatch.

### WD-4 — The initial acceptance profile should cross-reference NIR_TARGET_SHAPE

The profile lists NIR inputs but doesn't reference the NIR document's operation
definitions. Add: "See NIR_TARGET_SHAPE.md §Operations for the canonical
definition of each NIR op consumed here."

### WD-5 — Missing linkage to NIR effects model

NIR has `NirEffects` with `preserves: NirRegisterSet`. MIR has `MirEffects`
without `preserves`. Document whether MIR intentionally drops `preserves`
(because it's been consumed during ABI lowering) or whether it's an omission.

---

## Summary

| ID | Severity | One-liner |
|---|---|---|
| MF-1 | must-fix | Remove `MirCond::Compare`; use `BoolValue` or `FusedCompare` ref |
| MF-2 | must-fix | Replace post-mat example with pseudo-op notation, not `lda`/`sta` |
| MF-3 | must-fix | Add `MirCarryIn` model for byte-width `Add`/`Sub` |
| MF-4 | must-fix | Add `sp` to `MirRegisterSet`; add `stack_depth_delta` to effects |
| MF-5 | must-fix | Add `MirOp::Extend` and `MirOp::Truncate` |
| MF-6 | must-fix | Split `MirLoc` into def-sites vs memory-sites |
| SF-1 | should-fix | Define `MirCondDest` |
| SF-2 | should-fix | Clarify `MirAddr` vs `MirLoc` split rule |
| SF-3 | should-fix | Remove `LogicalNot` from MIR |
| SF-4 | should-fix | Remove `Identity` unary op |
| SF-5 | should-fix | Add `LeaAddr` op for address materialization |
| SF-6 | should-fix | Define `MirMemoryRegion` |
| SF-7 | should-fix | Define `MirFrame` |
| SF-8 | should-fix | Document `Fallthrough` → `Return` normalization requirement |
