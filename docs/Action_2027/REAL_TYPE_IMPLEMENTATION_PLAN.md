# Native REAL Implementation Plan

Implementation status: complete. Slices 0 through 9 provide the Atari oracle,
exact decimal codec, modern-profile semantic contract, MIR storage-size
foundation, address-based NIR, both code-generation backends, aggregate and
indirect storage, and a clean-room first-party library surface.

## Goal

Add a native `REAL` value type to Action 2027. Its representation and observable
arithmetic behavior will follow the Atari floating-point package (FPP), rather
than introducing an unrelated binary or fixed-point format.

The initial implementation will use Atari's six-byte packed-decimal value
format and the Atari OS FPP as its arithmetic provider. This gives Action code
the same data interchange format as Atari BASIC, the OS floating-point
workspace, and the historical Action Toolkit `REAL.ACT` library.

The first useful milestone is:

```action
REAL x, y, result

x = 1.25
y = 2
result = x * y + 0.5
```

with assignments and expressions working under `--runtime cart` and
`--runtime standalone`. Standalone removes the Action cartridge dependency; it
does not promise to remove Atari OS dependencies selected by the program.

## Compatibility Contract

`REAL` remains an identifier token, not a newly reserved lexer keyword. In the
modern language profile, semantic analysis contributes a built-in type symbol
named `REAL`. An ordinary source declaration in a nearer scope may shadow that
symbol.

This rule is required for historical code such as:

```action
TYPE REAL=[CARD r1,r2,r3]
```

The source-defined `REAL` above remains a record type with fields `r1`, `r2`,
and `r3`; it must not silently become the native type. In particular,
`samples/toolkit/modern/REAL.ACT` must continue to compile without source
changes and with the same meaning.

The native type is initially an Action 2027 modern-profile feature. The parser
stays profile-neutral. The selected language profile is passed into semantic
analysis so the built-in symbol is only installed where the feature is
available. This avoids backend checks based on the spelling `REAL`.

The historical Toolkit source is useful as behavioral and ABI evidence, but
code under `corpora/` is not covered by the compiler's license. No Toolkit
implementation is to be copied into the compiler or an embedded runtime.

## Language Contract

The initial native type has these properties:

- storage size is exactly six bytes;
- assignment copies the value, not an address;
- global, local, and absolute variables are supported;
- a real literal is converted directly from its decimal source spelling;
- `+`, `-`, `*`, and `/` produce `REAL` when either operand is `REAL`;
- unary `+` and `-` are supported;
- comparisons produce the existing Boolean/condition result;
- integer-to-real conversion is implicit in mixed arithmetic and assignment;
- real-to-integer conversion is explicit; statically known non-integral and
  out-of-range values are diagnosed, while dynamic values use Atari FPI
  nearest-integer rounding followed by the target Action integer width;
- `MOD`, shifts, and bitwise operators reject real operands;
- taking the address of a real value and using `REAL POINTER` are supported;
- REAL arrays, indexed values, pointer dereferences, and REAL record fields are
  supported;
- `CONST REAL Name=1.25` defines an exact immutable value;
- by-value real parameters and real function results are deferred until their
  calling convention has been designed.

By-value call ABI forms and library I/O remain staged after storage and
arithmetic. Unsupported forms receive explicit diagnostics rather than being
represented as six unrelated bytes.

The implementation must define and test conversion details against Atari OS,
including exponent limits, rounding, underflow, overflow, signed zero, and
invalid input. These details must come from oracle results, not host floating
point behavior.

## Representation and Runtime ABI

Native `REAL` uses the Atari six-byte packed-decimal representation. Compiler
code represents a constant as an opaque `[u8; 6]` value, with a dedicated type;
it never routes through `f32` or `f64`.

Compile-time literal conversion parses the lexer-preserved decimal spelling and
produces the six bytes directly. Its test oracle is the Atari OS ASCII-to-FP
routine. This avoids a double-rounding dependency on the host and makes builds
reproducible.

The initial target provider uses the standard Atari OS FPP workspace and entry
points. The known interface includes FR0 at `$00D4`, FR1 at `$00E0`, and ROM
entry points used by the historical library, such as FADD, FSUB, FMULT, and
FDIV. Addresses and clobbers are to be confirmed in an executable oracle before
being made compiler constants.

Evaluation is destination-passing:

```text
materialize left  -> FR0
materialize right -> FR1
call FPP routine
copy FR0          -> destination
```

Nested expressions first materialize their children into compiler-owned
six-byte temporary storage. They must not leave a live intermediate only in
FR0 or FR1 because those workspaces are shared by every FPP operation.

FPP calls are initially modeled as conservative ordering barriers. They may
clobber processor state and the documented floating-point zero-page workspace,
and they are not assumed to be reentrant or interrupt-safe. Listings and maps
should identify the Atari OS FPP dependency separately from Action cartridge
runtime dependencies.

## Architecture

The feature follows the existing ownership boundaries:

```text
decimal source text
        |
        v
SemIR: native REAL type, conversions, operator meaning, addressability
        |
        v
NIR: typed six-byte places, literal statics, explicit REAL operations
        |
        v
MIR6502: FR0/FR1 transfers, FPP service calls, register/effect model
        |
        v
emission: bytes, relocations, maps, listings
```

SemIR owns whether a name denotes the built-in type or a source-defined record,
which conversions are legal, and which operator is selected. MIR6502 must not
look at the source name `REAL` or consult SemIR to recover those decisions.

### Semantic representation

`REAL` should be a distinct semantic value kind, not another member of the
existing integer `ScalarType`. The scalar implementation currently assumes a
one- or two-byte integer with signedness, bit width, wrapping casts, shifts, and
bitwise operations. Extending that abstraction to six-byte decimal arithmetic
would weaken those invariants.

The semantic model records:

- the stable symbol identity of the built-in type;
- a distinct real value type with storage width six;
- exact literal bytes derived from the original spelling;
- explicit integer/real conversions;
- resolved real arithmetic and comparison operations;
- unsupported ABI uses, with source spans.

### NIR representation

Verifier-clean NIR must not pretend a real value fits in the existing byte/word
temporary lane. Real values are addressable, typed places. Proposed operations
are structurally equivalent to:

Slice 2 establishes `NirTypeKind::Real` as a six-byte type fact so the SemIR/NIR
boundary does not erase the semantic type. It does not authorize that type in
existing executable scalar operations; those remain behind the code-generation
gate until Slice 4 adds and verifies the structured forms below.

```text
CopyValue   { type: REAL, destination, source }
RealUnary   { operation, destination, operand }
RealBinary  { operation, destination, left, right }
RealCompare { predicate, result_bool_temp, left, right }
RealConvert { kind, destination_or_result, operand }
```

Names are illustrative; the important contract is that operands use stable
storage/static identities and structured places, never source strings. Real
literals live in immutable six-byte NIR static data and may be deduplicated by
their canonical bytes.

The lowerer allocates hidden six-byte evaluation locals and normalizes complex
lvalues into places before emitting real operations. The verifier rejects a
real type in ordinary scalar `Load`, `Store`, `Binary`, `Cast`, or temporary
forms. Initial optimizers treat real operations as conservative memory/call
barriers. Algebraic folding is deferred until Atari rounding and effect rules
are represented strongly enough.

### MIR6502 representation

MIR currently couples a storage slot to `MirWidth::Byte` or `MirWidth::Word`.
Before native real lowering, storage allocation size must be separated from the
width of an individual machine transfer. A local may then reserve six bytes
while every emitted 6502 load/store remains byte- or word-sized.

Slice 3 establishes that separation with `MirStorageSlot::storage_size` and an
optional `scalar_width`. Parameters retain a byte/word scalar width because the
Action ABI requires one; inline arrays, records, and native `REAL` locals may be
address-only. Layout and initialization use `storage_size`, while ABI and
machine-transfer selection use `scalar_width`.

Atari FPP calls are target services, not Action runtime helpers. They should be
represented by a structured MIR operation or service identifier carrying the
selected routine and conservative effects. The cart and standalone runtime
linkers must not attempt to resolve them as embedded Action procedures.

The MIR materializer expands each real operation into explicit copies between
typed storage and FR0/FR1, a fixed Atari OS call, and a result copy. The emitter
continues to own instruction encoding and listing/map output.

## Implementation Slices

Each slice should be reviewable and independently green. No slice should add a
backend special case for an individual sample.

### Slice 0: Atari Oracle and Compatibility Baseline

- Add small original-compiler/VM probes that call the Atari FPP conversion and
  arithmetic routines directly.
- Capture byte vectors for ordinary values, exponent boundaries, rounding
  cases, zero, signed input, overflow, and underflow.
- Confirm ROM addresses, FR0/FR1 usage, register clobbers, and observable error
  behavior on the supported OS image.
- Add a regression proving a source-defined `TYPE REAL` shadows the planned
  built-in and retains record field behavior.
- Record the accepted literal grammar separately from the binary
  representation contract.

Exit criterion: the compiler has authoritative golden vectors and no design
decision depends on an assumed host or Atari floating-point behavior.

### Slice 1: Exact Packed-Decimal Codec

- Introduce an opaque `AtariReal`/`RealBytes` value containing exactly six
  bytes.
- Convert `NumberKind::Real` text with decimal integer arithmetic; do not use
  host `f32` or `f64`.
- Preserve the original literal text for diagnostics while storing canonical
  bytes in semantic facts.
- Test the codec against every Slice 0 oracle vector and malformed/excessive
  input.

Exit criterion: decimal literals have deterministic Atari bytes, but no source
program yet gains native real semantics.

### Slice 2: Semantic Type and Operator Contract

- Pass the resolved language profile into semantic analysis.
- Seed a built-in `REAL` type symbol only for the modern profile; do not add a
  lexer keyword.
- Add the distinct semantic real type and six-byte storage facts.
- Type real literals, declarations, assignments, arithmetic, comparisons, and
  pointer forms.
- Insert explicit integer-to-real conversions for supported mixed expressions.
- Diagnose bitwise operations, shifts, unsupported call ABI forms, and any
  deferred storage form.
- Prove with semantic tests that a source-defined `REAL` shadows the built-in.

Exit criterion: SemIR completely describes native real meaning without name
rediscovery by a backend.

### Slice 3: MIR Storage-Size Foundation

- Separate a MIR storage slot's allocation size from scalar transfer width.
- Permit address-only local storage larger than two bytes without representing
  it as a byte scalar.
- Update layout, overlap, initialization, and verifier logic.
- Add behavior-preserving tests for existing arrays, records, initializers, and
  byte/word temporaries.

Exit criterion: MIR can reserve a typed six-byte local with no changes to
existing generated programs.

### Slice 4: Address-Based REAL NIR

- Add the NIR real type with width six.
- Lower literals to immutable six-byte statics with stable IDs.
- Add typed destination-passing copy, unary, binary, comparison, and conversion
  forms as needed for the first executable milestone.
- Allocate hidden real evaluation locals while preserving source evaluation
  order.
- Extend the printer and boundary documentation.
- Tighten the verifier to reject real values in scalar byte/word operations or
  ordinary value temporaries.
- Add positive fixtures and negative verifier tests; keep real operations as
  optimizer barriers.

Exit criterion: verifier-clean NIR represents complete real expressions with
no raw names, expression strings, or implicit FR0/FR1 state.

### Slice 5: MIR6502 FPP Lowering

- Add structured Atari FPP target-service identities and effect descriptions.
- Materialize six-byte operands into FR0/FR1 and copy FR0 to the destination.
- Lower assignment and `+`, `-`, `*`, and `/`.
- Support global, local, and absolute real storage.
- Expose the OS dependency in maps/listings without adding an embedded runtime
  binding.
- Run generated programs under the VM in both cart and standalone modes.

Exit criterion: the core example in the Goal section runs correctly with the
MIR6502 backend under both runtime modes, including nested expressions and
aliased operands.

### Slice 6: Comparisons, Unary Operations, and Conversions

Status: complete.

- Add all relational predicates and real truth/nonzero testing.
- Add unary sign handling with a canonical zero policy.
- Add BYTE, CHAR, CARD, and INT to/from real conversions using confirmed Atari
  routines or small clean-room adapters.
- Cover mixed arithmetic, compound assignment, conditions, and loop tests.
- Define diagnostics for conversion overflow and non-integral real-to-integer
  results.

Exit criterion: native real participates consistently in the ordinary
expression and control-flow type system.

The implementation compares canonical six-byte representations directly, with
sign-aware ordering, so adjacent representable values remain distinguishable.
Unary negation canonicalizes zero. Atari IFP and FPI operate on unsigned
16-bit magnitudes; the MIR6502 adapters preserve signed Action `INT` semantics
and keep sign state in hidden frame storage across the opaque OS call. FPI
rounds dynamic nonnegative magnitudes to the nearest integer. Statically known
non-integral or out-of-range casts are rejected before lowering; dynamic casts
then apply the requested Action integer width to the FPI word.

### Slice 7: Initializers, Arrays, and Indirect Storage

Status: complete.

- Emit six-byte global/static initializers.
- Support arrays, indexed lvalues, pointer dereferences, fields containing
  native real values, and overlap-safe copies where required.
- Add typed `CONST REAL` only after its grammar and address/value behavior are
  explicit; represent it as immutable typed static data rather than a widened
  integer constant.
- Deduplicate identical literal statics without changing observable addresses
  of user-declared objects.

Exit criterion: every normal storage path either handles six-byte values or
produces a focused unsupported diagnostic.

Scalar REAL declarations initialized with a REAL literal and REAL array
initializer lists emit exact packed-decimal bytes. The contextual grammar
`CONST REAL Name=...` is recognized only when `REAL` is followed by a constant
name, so `CONST REAL=1` still declares an ordinary constant named `REAL`.
Initially a typed REAL constant accepts a signed REAL literal or an earlier
`CONST REAL`; it has value semantics and no source-level address. Uses lower to
immutable REAL `rodata`, deduplicated by canonical six-byte value across
routines. This cannot coalesce or move user-declared storage.

REAL copies now retain NIR's structured index, dereference, and field address
forms through MIR. Six source bytes are staged before any destination byte is
written. Dynamic indexes use the six-byte element-size fact; MIR address
advance supports nonzero byte scales beyond the scalar 1/2-byte cases. Record
layout counts a REAL field as six bytes.

### Slice 8: Classic Backend Parity

Status: complete.

- Extend the SemIR-to-classic bridge with structured native-real facts and
  hidden storage; do not make classic code generation infer semantics from the
  identifier spelling.
- Lower the same target-service operations and preserve source evaluation
  order.
- Keep backend selection orthogonal: requesting classic must not silently
  switch to MIR6502.
- Run the same cart/standalone VM matrix and compare results with MIR6502.

Exit criterion: native real behavior is backend-independent for the supported
language surface.

The SemIR-to-classic projection uses a compiler-only `NativeReal` type carrier
and structured expression facts containing exact literal bytes, resolved
integer conversions, operators, and lvalue shapes. The parser never constructs
the carrier, and classic code generation never decides native semantics from a
type name. A source-defined record named `REAL` therefore remains an ordinary
named record in this bridge.

Classic routines reserve hidden six-byte evaluation slots plus integer, sign,
and saved-address scratch selected from the resolved expression tree. Binary
operands are materialized left-to-right before FR0/FR1 service calls. Assignment
captures an indirect destination address before evaluating the right-hand side,
and all six source bytes are staged before stores, preserving evaluation order
and overlap behavior. Dynamic array and pointer indexes use their structured
six-byte element size.

Classic emission records the same Atari OS FPP service bindings in maps and
listings as MIR6502. `Optimized` remains the classic backend; it is not routed
through MIR6502. The core, overlap, control/conversion, and aggregate fixtures
run in the cart/standalone matrix under both backends and compare the same
observable packed-decimal bytes.

### Slice 9: First-Party Library Surface

Status: complete.

- Add clean-room, pointer-oriented procedures for text conversion and output,
  such as `ValR`, `StrR`, and printing support.
- Add wrappers for confirmed FPP functions such as exponent and logarithm
  operations.
- Keep the native type/compiler contract independent of this optional module.
- Document shared FR workspace, OS requirements, and interrupt/reentrancy
  constraints.

Exit criterion: ordinary programs can input, calculate, and display native real
values without importing the historical Toolkit source.

The clean-room library is split into a portable `MATH` facade, qualified `SYS`
conversion and I/O entry points, and the Atari-specific `ATARI.REAL` provider.
`MATH` exposes `Exp`, `Exp10`, `Ln`, `Log10`, `Power`, `Sqrt`/`Sqr`, `Sin`,
`Cos`, `Tan`, `Atan`/`Atn`, `Abs`, `Sgn`, `Floor`, and `Rnd`. `SYS` exposes
pointer-oriented `StrR`, `ValR`, output, and input helpers. The provider is
ordinary Action 2027 source in `embedded/modules/atari/real.act`; the
compiler's native type and lowering do not depend on it.

Executable AltirraOS oracle tests confirm FASC's high-bit-terminated output and
the four transcendental ROM entries. Square root and trigonometry are clean-room
Action implementations layered on those primitives and native REAL arithmetic,
so they remain available without the BASIC cartridge. The library preserves
packed-decimal computation rather than replacing it with host math. Its
procedures share FR0, CIX, INBUFF, and the wider FPP workspace, so they are not
reentrant or interrupt-safe. Both cart and standalone runtime modes still
require a compatible Atari OS when this module is used.

Trigonometric reduction is bounded: supported arguments use a nearest-period
calculation and split `2*pi` subtraction rather than an input-proportional
subtraction loop. `Sin`, `Cos`, and `Tan` define `|x| >= 1E6` as total loss and
return zero, ensuring that every native REAL input terminates even when the OS
FPP can no longer represent subtracting one period from the original value.

## Validation Matrix

The feature needs tests at every ownership boundary:

- lexer: unchanged real spelling recognition and preserved source text;
- parser: `REAL` remains an identifier and historical declarations still parse;
- semantic: built-in resolution, shadowing, conversions, invalid operators, and
  unsupported ABI forms;
- codec: exact six-byte vectors against Atari OS;
- NIR: fixtures, printer output, stable identities, and verifier rejection of
  scalar-lane real values;
- MIR6502: exact FR transfers, target calls, effects, and six-byte allocation;
- emission: expected `JSR` targets plus map/listing OS dependency annotations;
- VM: nested expressions, aliasing, zero, signs, exponent edges, rounding,
  comparison, and integer conversions;
- configuration: modern/MIR6502 and modern/classic, each with cart and
  standalone runtime;
- compatibility: unchanged Toolkit `REAL.ACT` semantics and original compiler
  survey stability.

After every NIR or MIR-affecting slice, run at least:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Before declaring parity complete, also run the original-compiler survey,
Toolkit comparison, TN stability checks, and the VM runtime matrix. Any NIR
fixture change must be identified as an intentional IR-contract change rather
than a printer accident.

## Deferred Work

The following are intentionally outside the first implementation:

- a software FPP for systems or configurations without Atari OS;
- IEEE-754 `SINGLE` or Mad Pascal-style Q24.8 fixed point;
- by-value real parameters and native real function results;
- host-float or algebraic constant folding;
- aggressive copy elimination across calls, absolute memory, pointer writes, or
  machine blocks;
- non-Atari targets.

An OS-free provider can be added later behind the same structured real-operation
contract. It must preserve the six-byte value representation and documented
rounding behavior so programs and stored data do not depend on the provider.

## Completion Criteria

The feature is complete when:

- native values have a documented, exact six-byte Atari representation;
- decimal literals are reproducible and match the Atari oracle without host
  floating-point conversion;
- nested and aliased expressions are correct despite shared FR0/FR1 state;
- SemIR owns all type and operator decisions;
- verifier-clean NIR cannot place real values in byte/word scalar lanes;
- MIR6502 models FPP calls and effects explicitly;
- both backends work under cart and standalone runtime modes;
- maps/listings disclose the Atari OS dependency;
- historical source-defined `REAL` types continue to work unchanged; and
- no historical Toolkit implementation has been copied into distributed code.
