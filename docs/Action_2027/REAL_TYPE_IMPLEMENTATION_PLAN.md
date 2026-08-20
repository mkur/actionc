# Native REAL Implementation Plan

Implementation status: in progress. Slices 0 through 4 are complete: the Atari
oracle, exact decimal codec, modern-profile semantic contract, MIR storage-size
foundation, and address-based NIR are in place. MIR6502 FPP lowering remains
gated by an explicit diagnostic.

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
- real-to-integer conversion is explicit and diagnoses overflow according to a
  documented policy;
- `MOD`, shifts, and bitwise operators reject real operands;
- taking the address of a real value and using `REAL POINTER` are supported;
- by-value real parameters and real function results are deferred until their
  calling convention has been designed.

Arrays, indexed real lvalues, typed real constants, and library I/O are staged
after scalar storage and arithmetic. Unsupported forms receive explicit
diagnostics rather than being represented as six unrelated bytes.

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

- Add all relational predicates and real truth/nonzero testing.
- Add unary sign handling with a canonical zero policy.
- Add BYTE, CHAR, CARD, and INT to/from real conversions using confirmed Atari
  routines or small clean-room adapters.
- Cover mixed arithmetic, compound assignment, conditions, and loop tests.
- Define diagnostics for conversion overflow and non-integral real-to-integer
  results.

Exit criterion: native real participates consistently in the ordinary
expression and control-flow type system.

### Slice 7: Initializers, Arrays, and Indirect Storage

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

### Slice 8: Classic Backend Parity

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

### Slice 9: First-Party Library Surface

- Add clean-room, pointer-oriented procedures for text conversion and output,
  such as `ValR`, `StrR`, and printing support.
- Add wrappers for confirmed FPP functions such as exponent and logarithm
  operations.
- Keep the native type/compiler contract independent of this optional module.
- Document shared FR workspace, OS requirements, and interrupt/reentrancy
  constraints.

Exit criterion: ordinary programs can input, calculate, and display native real
values without importing the historical Toolkit source.

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
