# MIR6502 Static-Array Affine Index Plan

Status: implemented. Created and completed 2026-09-01.

This plan continues
[`MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md`](MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md)
with the general compiler gaps exposed by comparing the array Flame kernels
produced by actionc and Mad Pascal. The benchmark is an observation workload,
not an optimization contract: no selector may depend on a Flame routine name,
a particular absolute address, the four neighbour constants, or source
spelling.

The implementation order is:

```text
affine static indexing
    -> direct indexed accumulation
    -> register-carried full-range loop

symbolic fixed-array backing addresses (independent source-parity slice)
```

Each slice is independently useful, tested, and committed before the next
slice begins.

## Baseline

The Action! and Mad Pascal sources perform the same computation:

- one inclusive 256-element ascending byte loop;
- three rows updated in the same order;
- four byte inputs at index displacements 30, 31, 32, and 63;
- byte-width addition followed by two right shifts;
- one descending 32-element random seed loop.

The relative charset, display, row, and seed-buffer layout is also the same.
The important difference is target materialization.

Mad Pascal folds every constant displacement into an absolute indexed base and
keeps the induction value in Y:

```asm
lda row1+30,y
clc
adc row1+31,y
clc
adc row1+32,y
clc
adc row1+63,y
lsr
lsr
sta row1,y
```

The current actionc MIR6502 path preserves the required word semantics of
`CARD(index)+constant`, but separately extends the byte index, adds each
constant, constructs each 16-bit address in a pointer pair, and spills loaded
values and partial sums.

Measured baselines using Atari800 in NTSC mode over the same 250-frame window
are:

| Measurement | actionc MIR6502 | attached Mad Pascal binary |
|---|---:|---:|
| completed outer iterations | 56 | 203 |
| 256-element hot-loop code | about 789 bytes | 63 bytes |

The benchmark numbers are validation signals, not test assertions. Exact code
size and iteration count may change for unrelated reasons.

## Architectural boundary

SemIR owns compile-time constant resolution and the fact that a declaration has
an exact backing address. NIR owns typed extension, arithmetic, indexed places,
storage identity, and source-independent control flow. MIR6502 owns affine
address decomposition, 6502 indexed-address selection, A/X/Y placement,
machine flags, and compact loop latches. Emission writes only already-selected
operations.

The first three slices therefore change MIR6502 only. They must not introduce
an absolute-indexed NIR operation or make MIR6502 consult SemIR. The fourth
slice improves the semantic storage fact and its NIR projection so both classic
and MIR6502 consume the same resolved address.

Calls, machine blocks, volatile accesses, unresolved aliasing, pointer writes,
and unknown effects remain conservative barriers unless existing structured
facts prove the exact transformation safe.

## Slice 1: affine static byte indexes

### Goal

Recognize the typed MIR definition graph corresponding to:

```text
root:u8 -> unsigned extend to u16 -> add constant:u16
```

when it indexes a byte element of directly addressable storage. Convert:

```text
base[(u16)root + C]
```

to the existing structured address shape:

```text
base=(base + C), index=root, element_size=1
```

This lets materialization select `AbsoluteIndexedY` without constructing a
pointer pair for every access. `MirAddr::ComputedIndex.offset` and structured
`MirMem` offsets already carry the required information; no persistent NIR
operation is needed.

### Implementation

Add a small affine-index analysis beside the existing delayed byte-index logic
in `src/mir6502/materialize/indexes.rs`. Its result should contain:

- the byte-valued root;
- a checked `u16` displacement;
- the extend/add producer operations;
- referenced memory whose stability must be proved;
- whether the expression has word or byte-wrapping semantics.

Initially accept:

- unsigned byte-to-word extension;
- addition of one or more compile-time word constants;
- the commuted form `constant + extended_byte`;
- byte-element reads and writes;
- global, static, and absolute-backed direct storage;
- structured bases whose final layout supports absolute indexed access.

Use the analyzed rewrite workflow to remove producer operations only when
routine-wide use/def proves them dead. Keep stable storage IDs and offsets until
layout/emission; do not replace them with early numeric addresses.

### Safety rules

`CARD(index)+30` deliberately carries out of the low byte. Folding its constant
into the absolute base is exact because absolute indexed addressing performs
that page carry. It does not require an upper-bound proof for the byte root.

By contrast, `index+30` is byte arithmetic and wraps before indexing. It must
retain the existing wrapping path unless range analysis proves that no wrap can
occur. The affine analysis must distinguish these cases rather than infer
semantics from the final numeric values.

Reject initially:

- signed extension;
- subtraction or a runtime displacement;
- element sizes other than one;
- pointer-backed or descriptor-backed bases;
- shared producer chains that cannot be deleted safely;
- an unrepresentable base-plus-displacement;
- any case whose selected direct address is not valid after final layout.

### Acceptance criteria

- Static `BYTE` reads and writes using `CARD(byte_index)+constant` emit
  absolute-indexed accesses.
- Page crossings at indexes 226 through 255 remain correct.
- The materialized path contains no pointer-pair construction for accepted
  accesses.
- Byte-wrapping, signed, dynamic, and pointer-backed negative cases retain the
  general path.
- No executable NIR shape changes.

Commit:

```text
mir6502: fold widened byte indexes into static bases
```

## Slice 2: direct indexed byte accumulation

### Goal

Keep a byte arithmetic chain in A and consume an immediately preceding direct
indexed load as a 6502 memory operand. Transform:

```text
load temp, base+C,Y
sum = add A, temp
```

into:

```asm
clc
adc base+C,y
```

This removes both the loaded-value home and the partial-sum spill while
preserving the original evaluation order.

### Implementation

Add a narrow post-home target operation, for example
`BinaryDirectIndexedByte`, with:

- A as the implicit left operand and destination;
- X or Y as an explicit proven index carrier;
- a structured `MirMem` source;
- explicit binary operation and carry behavior;
- byte width only in the first implementation.

Add verifier, printer, effects, liveness, rewrite, spill-census, standalone,
and emission handling. Selection should use the analyzed post-home rewrite
driver rather than an emitter peephole.

Start with addition whose carry input is absent or explicitly clear and whose
carry output is ignored. Other operations may be added later only through the
same proof and cost model.

Require:

- one load and exactly one arithmetic use;
- adjacency or an intervening range proven effect-free;
- unchanged index-register identity;
- no call, barrier, machine block, aliasing write, or register clobber;
- no volatile access movement, repetition, or deletion;
- an A result that feeds the ordinary next consumer.

### Acceptance criteria

- A four-input byte sum emits one indexed `LDA` and three indexed `ADC`
  instructions.
- The shifts and following indexed store consume the value directly from A.
- No private loaded-value or partial-sum spill remains.
- Each source is read exactly once and in source order.
- Barrier, volatile, multi-use, and clobbering negative cases retain the
  general path.

Suggested telemetry:

```text
direct-indexed-byte-binary-selected
```

Commit:

```text
mir6502: accumulate direct indexed byte loads in A
```

## Slice 3: full-range byte induction in Y

### Goal

After Slices 1 and 2, first measure whether the existing counted-loop and
register-carried induction selectors already choose Y and remove repeated index
loads. Extend those selectors only for facts they do not yet cover.

For a proven inclusive unsigned loop from 0 through 255 with step +1, select:

```asm
ldy #0
loop:
    ; body uses absolute,Y
    iny
    bne loop
```

### Proof obligations

- Counted-loop analysis identifies one canonical preheader, header, body,
  latch, and normal exit.
- Width is byte, direction is ascending, step is one, start is zero, and bound
  is 255.
- The loop is proven to enter and wrap is used only as the selected terminal
  condition.
- Y has no conflicting body demand or clobber.
- The induction home is nonvolatile, not address-taken, and not ambiguously
  aliased by a body store.
- Calls, machine blocks, unknown effects, and noncanonical exits reject the
  candidate.
- Every `EXIT` path preserves the source-visible current induction value.

If the final induction value is dead or immediately overwritten, omit a normal
exit writeback. If Action! semantics make the final 255 observable, restore and
write back 255 at the canonical exit. If that cannot be proved exact, retain
the general guarded loop.

The existing machine-value analysis should remove repeated `LDY induction`
loads within the body and keep the exact Y fact across the backedge. Do not add
a second allocator or a loop-shaped emitter peephole.

### Acceptance criteria

- A safe full-range loop emits one Y initialization and an `INY/BNE` latch.
- Accepted loop bodies contain no induction-home reloads or memory increment.
- Live post-loop values and `EXIT` paths receive correct writeback.
- Volatile, address-taken, aliasing, call-containing, and conflicting-register
  loops retain the existing representation.
- The existing overflow-safe general ascending loop remains the fallback.

Commit:

```text
mir6502: carry full-range byte induction in Y
```

## Slice 4: symbolic fixed-array backing addresses

### Goal

Allow both maintained compiler paths to classify a qualified compile-time
address expression as exact fixed backing storage:

```action
BYTE ARRAY row0(256)=FLAMES_DISPLAY.FIRE_ANCHOR-31
```

This removes the need for raw numeric addresses in the Action! Flame source.
It does not affect hot-loop performance and remains a separate semantic/storage
slice.

### Implementation

- Resolve qualified constants and supported arithmetic through semantic
  constant evaluation.
- Record the exact fixed-array backing address as a structured storage fact.
- Project the same fact into NIR and the classic codegen input.
- Keep the display name and expression only as source/debug metadata.
- Reject runtime-dependent expressions, unresolved names, invalid types, and
  address overflow.
- Restore the Flame declarations to symbolic addresses only after all public
  modes pass focused coverage.

If this changes an IR boundary, update `NIR_TARGET_SHAPE.md` with the owner and
verifier guarantee. Do not add qualified-name parsing or SemIR lookup to
MIR6502.

### Acceptance criteria

- Literal and qualified-constant fixed arrays have identical storage facts in
  compatibility, optimized classic, and MIR6502 modes.
- Arithmetic over imported `CARD` constants is accepted when fully constant.
- Runtime and overflowing expressions receive an explicit diagnostic.
- Printed IR remains readable while executable consumers use the resolved
  storage identity and address.

Commit:

```text
semir: resolve fixed-array backing constant expressions
```

## Regression coverage

Add a generic fixture, not named after Flame, with a fixed byte region large
enough to exercise accesses at `CARD(index)+30`, `+31`, `+32`, and `+63`.
Cover indexes 0, 225, 226, 254, and 255 so page carry is observable.

Focused positive coverage must prove:

- runtime agreement in compatibility, optimized classic, and MIR6502 modes;
- direct absolute-indexed reads and writes in materialized MIR6502;
- direct indexed arithmetic with no value/address spills;
- one shared index carrier in the loop;
- correct final induction value where observable.

Focused negative coverage must include:

- byte-wrapping addition without a widening cast;
- signed extension;
- dynamic displacement;
- word elements;
- pointer and descriptor bases;
- volatile inputs;
- an aliasing store;
- a call or machine block in the loop;
- a live post-loop induction value and an `EXIT` edge.

Prefer structural MIR assertions and selected opcode subsequences. Do not make
the Flame routine size, addresses, or score a regression-test contract.

## Validation

Run after every MIR6502 slice:

```sh
cargo test --test mir6502_materialization_gap
cargo test --test mir6502_loop_consumer
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml
cargo test
```

Slice 4 changes semantic/storage facts and must additionally run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

After each material performance change, compile the benchmark suite in all
three public modes. For MIR6502, record:

- the hot-loop byte count;
- remaining spills and indirect-address preparations;
- `register-carried-induction-selected` and new selector telemetry;
- Atari800 NTSC iterations over 250 frames;
- visible screen-buffer correctness.

The final qualitative target is the Mad Pascal machine shape: one index
register, twelve absolute-indexed reads, three indexed stores, accumulation in
A, and an `INY/BNE` latch. Exact binary or score parity is not required.
