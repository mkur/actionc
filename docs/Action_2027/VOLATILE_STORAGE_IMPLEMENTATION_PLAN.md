# Volatile Storage Implementation Plan

Implementation status: implemented on the `Action-2027` branch. Syntax,
semantic facts, the executable NIR contract, both public classic modes, and
MIR6502 are covered. MIR6502 lowers volatile accesses through full-memory
compiler barriers and preserves them through final emission.

## Goal

Add a source-level storage qualifier for hardware registers and memory that can
change outside the current Action! routine:

```action
VOLATILE BYTE WSYNC=$D40A,
              VCOUNT=$D40B,
              COLBAK=$D01A

VOLATILE CARD RTCLOK=$0012
VOLATILE BYTE ARRAY POKEY(16)=$D200
```

`VOLATILE` is an actionc extension. It is accepted by both compiler profiles
and must have the same observable behavior in the classic and MIR6502
backends. The original Action! cartridge compiler does not accept it.

## Language Contract

The first grammar is:

```text
variable-declaration := [ VOLATILE ] type [ ARRAY ] declaration-entry
```

`VOLATILE` is recognized contextually before a variable type and applies to
every comma-separated entry in that declaration. It qualifies storage rather
than the scalar type.

The first implementation supports global and routine-local scalar and array
storage. It rejects volatile constants, parameters, type or record fields, and
pointer declarations with a focused diagnostic. Pointer syntax is deferred
because a volatile pointer cell and a pointer to volatile data need distinct
source meanings.

For each volatile source access:

- a read performs exactly one real memory read;
- a write performs exactly one real memory write;
- the compiler does not remove, combine, cache, duplicate, hoist, sink, or
  reorder the access;
- volatile accesses retain source order with respect to other volatile
  accesses, calls, machine blocks, and inline assembler;
- the first implementation treats the access as a conservative compiler
  memory barrier.

Volatility adds no runtime fence instruction. A volatile word access is still
two 6502 byte accesses and is not atomic. Compound assignments retain their
source read and write; they must not be selected as an NMOS 6502
read/modify/write instruction when the extra bus write would be observable.

An alias explicitly declared from volatile storage inherits volatility. A
separate unqualified declaration of the same numeric address does not acquire
the source qualifier implicitly. Existing conservative treatment of unknown
absolute memory remains in place for legacy programs.

## Architecture

SemIR owns qualifier legality and derives whether an lvalue access is
volatile. The semantic model records the qualifier against stable storage
identity rather than a name or formatted address.

NIR owns normalized volatile access semantics. Executable accesses use
explicit operation forms:

```text
Load | VolatileLoad | Store | VolatileStore
```

Volatility must not survive only as declaration metadata or debug text. This
is important because absolute-backed declarations can become absolute places
and indexed arrays become address computations before optimization.

MIR6502 consumes the NIR operation without consulting SemIR. Volatile accesses
are surrounded by zero-byte full-memory barriers, which preserve each access
through materialization while leaving the normal 6502 load/store encoding
unchanged. The qualifier changes legal transformations, not instruction
syntax.

The classic backend keeps the same property on its structured storage slots.
The declaration fact replaces the current name/zero-page heuristic used by
expression-effect proofs.

## Implementation Slices

### Slice 1: Syntax and semantic facts

- Add structured variable-declaration qualifiers to the AST.
- Recognize contextual, case-insensitive `VOLATILE` before a variable type.
- Preserve the qualifier through constant materialization and SemIR lowering.
- Record volatility by stable symbol/storage identity.
- Derive volatile lvalue access through array indexing and explicit aliases.
- Diagnose unsupported declaration contexts and pointer ambiguity.

Suggested commit: `language: add volatile storage declarations`.

### Slice 2: NIR contract and optimization safety

- Add an explicit access kind to executable NIR loads and stores.
- Lower semantic volatile accesses without address or name heuristics.
- Print the access kind readably and tighten the verifier.
- Keep unused volatile loads and every volatile store.
- Exclude volatile storage from forwarding, promotion, dead-store removal,
  home elision, and induction-address caching.
- Treat volatile operations as conservative ordering barriers.

Suggested commit: `nir: preserve volatile memory accesses`.

### Slice 3: Classic backend

- Carry volatility on classic `StorageSlot` values and aliases.
- Make expression-effect proofs use the declaration fact.
- Prevent cached/redundant load and store elimination.
- Invalidate tracked state conservatively at volatile accesses.
- Avoid unsafe direct read/modify/write selection.
- Cover compatibility and optimized classic modes.

Suggested commit: `codegen: honor volatile storage in classic backend`.

### Slice 4: MIR6502 backend

- Carry NIR access semantics into MIR loads and stores.
- Make centralized effects report volatile memory interaction.
- Prevent copy propagation, SSA-lite, consumer fusion, peepholes, and
  scheduling from removing or moving volatile accesses.
- Remove the assumption that explicitly volatile zero-page storage is safe to
  cache or reorder.
- Preserve ordinary storage optimization behavior.

Suggested commit: `mir6502: honor volatile memory accesses`.

### Slice 5: Documentation and sample adoption

- Document syntax, scope, ordering, non-atomic word access, and pointer limits.
- Document the SemIR/NIR/MIR ownership boundary.
- Mark the hardware-register declarations in the Action 2027 plasma sample.
- Add cross-backend object/listing checks where exact instruction counts are
  useful.

Suggested commit: `docs: document volatile storage semantics`.

## Validation

Focused regressions must cover:

- global and routine-local volatile declarations;
- grouped scalars and fixed/inline arrays;
- repeated reads and repeated same-value writes;
- unused reads and self-assignment;
- compound assignment without unsafe read/modify/write selection;
- volatile zero-page and hardware-register addresses;
- constant and dynamic array indexes;
- ordering around calls, machine blocks, and inline assembler;
- alias inheritance;
- diagnostics for parameters, fields, constants, and pointers;
- unchanged optimization of equivalent ordinary storage;
- compatibility, optimized classic, and MIR6502 modes.

After semantic or NIR changes run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

## Main Risks

The highest risk is losing the qualifier while converting named
absolute-backed storage to an absolute place or while materializing an indexed
address. The executable access therefore carries volatility explicitly.

The classic backend already has a narrow volatility heuristic, while MIR6502
allows some zero-page accesses to be cached or reordered. The implementation
must replace those assumptions with the declaration fact without weakening
the existing conservative policy for unqualified absolute memory.

The first implementation deliberately favors correctness over optimization.
Effect precision can be narrowed later only with tests proving that every
source-visible hardware access remains present and ordered.
