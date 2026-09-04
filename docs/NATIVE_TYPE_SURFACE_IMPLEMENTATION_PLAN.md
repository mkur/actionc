# Native Type Surface Implementation Plan

Snapshot date: 2026-09-04.

Status: active. Slice 0 records the source, semantic, NIR, and compatibility
contract. Each implementation slice must be committed and verified before the
next begins.

## Objective

Expose the value types needed by the first Motorola 68000 and WDC 65816
backends without changing the established meaning of Atari Action! programs.
The completed NIR target-layout and native-routine work already supports
target-width pointers, automatic storage, and typed call signatures. This plan
adds the missing source surface:

1. fixed-width signed `LONG` and unsigned `ULONG`, both 32 bits;
2. function results for every register-sized value type;
3. callable-pointer declarations with complete parameter and result types;
4. target-sized `ADDRESS` and `SIZE` integer types.

The intended boundary remains:

```text
source spelling and lookup
    -> SemIR value type and source conversion rules
    -> NIR integer width/role, typed call signature, and explicit cast
    -> target MIR result homes, lanes, and instructions
```

SemIR owns source typing and conversion legality. NIR owns normalized typed
computation. MIR68K and MIR65816 own physical register pairs, stack homes, and
multi-instruction implementation. No backend may infer source types from a
name or summary string.

## Source Contract

### Fixed-width integers

`LONG` is a signed 32-bit two's-complement integer. `ULONG` is an unsigned
32-bit integer. Their widths do not vary by target. `BYTE`, `CHAR`, `CARD`, and
`INT` retain their current widths, signedness, promotions, and overflow
behavior.

The names match the classic Amiga `exec/types.h` interface. They are
compiler-provided contextual type symbols rather than lexer keywords. Existing
programs may continue to declare values or fields named `long` or `ulong`.
Normal type lookup applies in type position.

Wide decimal literals infer `LONG` when they no longer fit the existing
16-bit literal classes and fit 32 bits. Wide hexadecimal literals infer
`ULONG`. Explicit casts remain available when the programmer needs a different
interpretation. Constant evaluation retains a value wide enough to diagnose
overflow before applying the destination type's wrapping rule.

### Function results

A function result is a complete value type rather than a `FundType` embedded
in `RoutineKind`. The initial returnable set is:

- `BYTE`, `CHAR`, `CARD`, `INT`, `LONG`, and `ULONG`;
- `ADDRESS` and `SIZE`;
- data pointers;
- callable pointers.

Existing declarations such as `BYTE FUNC Next()` retain their source and ABI
meaning. A pointer-returning declaration uses the ordinary Action!-style type
before `FUNC`:

```action
BYTE POINTER FUNC Allocate(SIZE bytes)
```

Record and `REAL` results are not part of this plan. They require an abstract
caller-owned result place and a separately specified indirect-result ABI;
silently treating either as a register value would weaken NIR.

### Callable-pointer signatures

An optional parameter list on a callable-pointer declaration supplies its
complete prototype:

```action
PROC POINTER notify(BYTE event, ADDRESS context)
ULONG FUNC POINTER checksum(BYTE POINTER data, SIZE length)
BYTE POINTER FUNC POINTER allocator(SIZE bytes)
```

An existing declaration without a parameter list remains an exact
zero-parameter callable, which matches the currently supported uses. Routine
address assignment and indirect calls require matching parameter types, result
type, variadic facts, and public calling convention. Parameter names in a
callable-pointer prototype are documentation only and do not introduce storage.

### Address and size integers

`ADDRESS` is an unsigned integer capable of representing an architectural
address. It is not a pointer and carries no data/code address-space identity.
Pointer conversions remain explicit; converting back to a pointer supplies the
destination pointee and address space.

`SIZE` is the unsigned size/index type of the selected data model. The target
layout records its width explicitly rather than asking semantic lowering to
guess it from a pointer width.

| Target | `LONG` / `ULONG` | `ADDRESS` | `SIZE` |
| --- | ---: | ---: | ---: |
| Atari 6502 | 32 | 16 | 16 |
| WDC 65816 small | 32 | 24 | 16 |
| WDC 65816 native | 32 | 24 | 24 |
| Motorola 68000 | 32 | 32 | 32 |

The canonical compiler-owned type identities are `SYS.LONG`, `SYS.ULONG`,
`SYS.ADDRESS`, and `SYS.SIZE`. The normal unqualified spellings are contextual,
shadowable prelude aliases. In particular, `CARD size` and a record field named
`address` remain legal source.

Pointer/address conversions obey these rules:

- `ADDRESS(pointer)` explicitly converts a data or callable pointer to its
  numeric representation;
- a typed pointer cast explicitly converts `ADDRESS` back to that pointer
  class;
- widening to `ADDRESS` is permitted when every source bit is represented;
- a narrowing `ADDRESS`-to-pointer conversion is permitted for a constant that
  fits, but a dynamic narrowing requires a visibly narrower intermediate
  integer conversion;
- ordinary integers and pointers do not become assignment-compatible merely
  because they have the same physical width.

Address arithmetic is deliberately narrow:

```text
ADDRESS + SIZE       -> ADDRESS
ADDRESS - SIZE       -> ADDRESS
ADDRESS - ADDRESS    -> SIZE
```

Equality, ordering, masks, and shifts are valid. Multiplication of an
`ADDRESS` is a diagnostic. Typed pointer offsets remain the preferred source
operation when a pointee type is known.

`SIZE` supports ordinary unsigned arithmetic and comparisons. `SYS.SIZEOF`,
`SYS.ELEMENTS`, `SYS.ALIGNOF`, and `SYS.OFFSETOF` produce `SIZE`; their
evaluation remains arbitrary precision until conversion to the selected
target type.

## Compatibility Contract

- No existing type spelling becomes a new lexer keyword.
- `BYTE`, `CHAR`, `CARD`, and `INT` are never widened by target selection.
- Existing untyped literals at or below `$FFFF` retain their current inferred
  type and behavior.
- Existing `PROC POINTER` and fundamental `FUNC POINTER` declarations retain
  their exact zero-parameter meaning.
- The Atari compatibility and MIR6502 objects recorded in
  [`NIR_ATARI_BASELINES.md`](NIR_ATARI_BASELINES.md) must remain byte-identical
  for all existing sources.
- The classic and MIR6502 emitters may initially reject runtime 32-bit and
  24-bit integer values with a focused unsupported diagnostic. They must never
  truncate them.
- Successful changes to NIR representation should preserve readable legacy
  printer output when no source contract changed. Snapshot changes must be
  classified as contract, printer-only, or bug-fix changes.

## Implementation Slices

### Slice 0: contract and baselines

Status: complete when this document is committed. The existing Atari baseline
matrix already covers lower-width arithmetic, direct function results, data
pointers, callable-pointer storage, indirect calls, records, arrays, and both
runtime/backend combinations. Later slices must reproduce it.

1. Record the syntax, widths, promotions, conversions, target matrix, and
   non-goals.
2. Reuse the byte-exact rows in `NIR_ATARI_BASELINES.md` as the compatibility
   oracle.
3. Require parser/semantic regression coverage before contextual type aliases
   are exposed so common identifier spellings remain source compatible.

Suggested commit:

```text
docs: define native scalar and callable type surface
```

### Slice 1: width-aware integer foundation

Status: complete. Numeric and semantic constants retain `u64` bits, while NIR
integer types and constants carry explicit width, signedness, and semantic
role. Existing fixture spelling is preserved for the legacy 8/16-bit surface.

1. Widen numeric-literal and semantic constant storage from `u16` to `u64`.
2. Make masks, wrapping, shifts, signed interpretation, and formatting consume
   explicit integer widths.
3. Give NIR integer types structured bit width, storage width, signedness, and
   semantic role (`ordinary`, `address`, or `size`).
4. Give integer constants an explicit NIR type rather than relying on an
   untyped `ConstU8`/`ConstU16` distinction.
5. Migrate verification, optimization, printing, signature hashing, data
   fragments, and target canaries without exposing new source syntax.
6. Preserve existing printed NIR and Atari objects where practical.

Suggested commit:

```text
nir: generalize fixed-width integer representation
```

### Slice 2: `LONG` and `ULONG`

Status: complete. Both contextual scalar aliases lower to fixed 32-bit NIR
integers, wide literals and constants retain all bits, native MIRs accept
32-bit values, and MIR6502 rejects runtime wide integers explicitly.

1. Register contextual `SYS.LONG` and `SYS.ULONG` type identities and their
   unqualified aliases without adding lexer keywords.
2. Support declarations, parameters, locals, fields, arrays, constants, casts,
   arithmetic, comparisons, and initializers.
3. Extend promotions conservatively: `ULONG` dominates `LONG`; `LONG`
   dominates the established 8/16-bit integers; pointer conversion remains
   explicit.
4. Infer wide decimal and hexadecimal literals according to the source
   contract and diagnose values beyond 32 bits.
5. Add SemIR/NIR fixtures for signed boundaries, wrapping, shifts, multiply,
   divide, and comparisons.
6. Let MIR68K and MIR65816 consume the typed 32-bit operations. Make unsupported
   Atari emission fail explicitly rather than truncate.

Suggested commit:

```text
language: add long and ulong scalar types
```

### Slice 3: general function result types

Status: complete. Function declarations now retain a complete result `TypeRef`,
SemIR and NIR signatures retain the resolved `ValueType`, native result homes
distinguish integer and address-class values, unsupported aggregate/`REAL`
results are diagnosed, and callable categories are structured NIR facts.

1. Make `RoutineKind` identify procedure versus function only.
2. Store a function result as a complete `TypeRef` in the AST and a complete
   `ValueType` in SemIR/NIR signatures.
3. Parse every returnable type before `FUNC`, including pointer results.
4. Preserve old declarations and diagnostics.
5. Type-check every `RETURN` against the complete result type.
6. Replace callable-kind strings used as executable NIR facts with a
   structured identity.
7. Plan native result homes as D0 for 68k integers, A0 for 68k addresses, A for
   65816 values up to 16 bits, and an explicit A:X pair for 24/32-bit values.
8. Diagnose record and `REAL` results until their indirect-result contract is
   implemented separately.

Suggested commit:

```text
language: generalize function result types
```

### Slice 4: complete callable-pointer signatures

Status: complete. Callable declarations now retain prototype parameter types,
including array-parameter decay, and propagate them through semantic and NIR
signature identity. Routine-address assignment requires the same structured
signature, while indirect calls enforce their prototype's arity and types.
Plain, array, record-field, and routine-parameter storage are covered.

1. Extend callable type syntax and the AST with prototype parameters and a
   complete result type.
2. Resolve prototype parameter types without allocating parameter objects.
3. Include parameter types, result, variadic facts, and convention in stable
   signature identity.
4. Require exact routine-address assignment compatibility.
5. Check indirect-call arity and argument types from the callable value.
6. Cover callable pointers stored in globals, locals, arrays, records, and
   passed as routine parameters.

Suggested commit:

```text
language: add complete callable pointer signatures
```

### Slice 5: `ADDRESS`

1. Add an explicit architectural-address integer layout to every target.
2. Register the contextual `SYS.ADDRESS` type and unqualified alias.
3. Add explicit data-pointer, code-pointer, and integer conversion classes.
4. Implement the address arithmetic matrix and reject unrelated arithmetic.
5. Preserve data/code address-space identity on conversion back to a pointer.
6. Serialize static address integers using target width and byte order.
7. Cover 16-, 24-, and 32-bit address constants and dynamic conversions.

Suggested commit:

```text
language: add target-sized address values
```

### Slice 6: `SIZE` and wide layout quantities

1. Add an explicit size-integer layout to every target.
2. Register the contextual `SYS.SIZE` type and unqualified alias.
3. Return `SIZE` from all four layout queries while retaining wide compile-time
   evaluation.
4. Widen semantic array lengths, record sizes, field offsets, alignments, and
   strides away from `u16`; convert to checked `ByteSize`/`ByteOffset` facts at
   the NIR boundary.
5. Support `SIZE` arithmetic, comparisons, loop bounds, parameters, and
   function results.
6. Accept target-representable objects larger than 64 KiB and diagnose objects
   outside the selected target/model limits.

Suggested commit:

```text
language: add target-sized size values
```

### Slice 7: acceptance and backend contract

1. Add a native-only source corpus combining 32-bit arithmetic, pointer
   results, typed callbacks, `ADDRESS`, `SIZE`, and layout queries.
2. Verify lowered and optimized NIR for 68k, 65816 native, and 65816 small.
3. Verify result homes, argument homes, frame extents, and indirect-call
   signatures in both native MIRs.
4. Exercise 24-bit and 32-bit address round trips and a 32-bit callback result.
5. Reproduce every Atari object baseline and record whether any fixture change
   is contractual, printer-only, or a bug fix.
6. Mark this plan complete and link it from the active documentation index.

Suggested commit:

```text
tests: cover native scalar and callable type surface
```

## Required Verification

After every semantic, NIR, or backend slice:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Also run focused parser, semantic, callable, target-layout, MIR68K, MIR65816,
and Atari object-baseline checks introduced by each slice.

## Non-goals

- Do not change `CARD` or `INT` width on native targets.
- Do not add implicit pointer/integer assignment compatibility.
- Do not implement 32-bit MIR6502 arithmetic in this migration.
- Do not add record or `REAL` return ABIs.
- Do not add 64-bit integers, unions, enums, bit fields, local `STATIC`, or
  near/far pointer qualifiers.
- Do not implement Amiga library adapters, C ABI interoperation, interrupt
  veneers, or object formats here.
- Do not add `PACKED`; it remains the next separate representation feature.

## Definition of Done

- All four types are ordinary resolved source types rather than name-based
  backend special cases.
- Width and signedness are explicit through SemIR and verifier-clean NIR.
- Function and callable-pointer signatures carry complete structured types.
- Pointer/address conversions are explicit and address-space safe.
- Native MIRs consume 16-, 24-, and 32-bit values without consulting SemIR.
- Existing Atari sources, NIR fixtures, maps, and load-format objects remain
  compatible.
- The required NIR sweep and complete Rust test suite pass.
