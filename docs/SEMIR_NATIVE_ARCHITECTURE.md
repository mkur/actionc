# SemIR Native Architecture

SemIR-native is the modern backend path. Its purpose is not to recreate the old
AST code generator under a new name. It should consume semantic meaning from
SemIR and lower it through a small set of reusable backend primitives.

The guiding constraint is architectural cleanliness:

- prefer one general path over several shape-specific paths;
- make helpers reusable before adding a new special case;
- treat SemIR as the source of meaning, not original source text shape;
- keep compatibility facts and modern lowering choices separate;
- add focused tests for each supported semantic shape.

This matters because the old direct code generator grew through many local
rules. SemIR-native should avoid becoming another large collection of
case-by-case patches.

## Lowering Layers

SemIR-native should be organized around these layers:

1. semantic input: typed SemIR expressions, places, calls, declarations, and
   control-flow facts;
2. shape classification: value, place, address, callable, array decay, pointer
   dereference, indexed element, literal, storage slot, computed expression;
3. materialization: put a classified shape into a concrete ABI/register home;
4. emission: write tracked 6502 instructions through `NativeTrackedEmitter`.

The important rule is that code emission should normally happen after
classification. If a lowering path needs to know whether something is an array
parameter, local array, string literal, pointer dereference, or indexed element,
that knowledge should live in the classifier or materializer, not be repeated
inside every caller.

See `archive/implementation-plans/semir-native/SEMIR_NATIVE_LAYER_PLAN.md` for the ownership contract and near-term plan
for each layer. See `SEMIR_NATIVE_BACKEND_STATUS.md` for the current validation
snapshot and open runtime risks.

## Classification Is Not Typing

Typing answers what a value means in the Action language. Classification
answers how the backend can obtain or address that value.

Examples:

- `s(i)` may have type `BYTE`, but classification says whether this is an
  inline array read, descriptor-backed array read, array-parameter pointer read,
  or pointer-index read;
- `p^` may have type `BYTE`, but classification says it is a dereference that
  needs the pointer value in a zero-page pair and a `(zp),Y` load;
- `@name` and array decay are word-sized values, but classification says they
  are address values whose low/high bytes can be materialized without reading a
  storage slot;
- `Func()` may have a scalar return type, but classification says it is a call
  result that must be produced through the public or internal call ABI;
- `x + 1` may have type `BYTE`, but classification says it is computed and may
  clobber registers and flags.

The type layer should remain responsible for legality, width, signedness,
pointer compatibility, callable signatures, and lvalue/read-only distinctions.
The classification layer should consume those facts and choose a lowering
strategy:

- literal value;
- storage slot;
- address value;
- dereference;
- indexed element;
- call result;
- computed expression;
- unsupported semantic shape.

Classification should not duplicate semantic type checking. If a shape reaches
SemIR-native, the classifier may assume the semantic analyzer already accepted
the program, but it should still reject backend-unsupported shapes with precise
diagnostics.

## Core Helpers

The backend should converge on one helper family per purpose:

- byte value to `A`;
- word value to a target slot or ABI bytes;
- address value to `A/X`, target slot, or ABI bytes;
- lvalue effective address to a known zero-page pointer pair;
- condition operand to compare/branch inputs;
- call argument to packed ABI bytes.

These helpers should accept semantic shapes rather than source patterns. For
example, array decay should be handled once as an address value, regardless of
whether it came from a global array, local array, array parameter, explicit
cast, or call argument.

## Call Argument Planner

The next major cleanup target is call lowering.

The backend should not grow separate call paths for each current failure such
as `MovePage(@zx, allocp, 4)`, `Print(s)`, or `Putchar(s(i))`. Instead, call
lowering should use a planner:

1. inspect the callee signature and each SemIR argument;
2. flatten arguments into public Action ABI byte positions;
3. classify each byte source as one of:
   - constant byte;
   - address low/high byte;
   - storage byte;
   - return-slot byte;
   - computed byte;
   - unsupported;
4. emit the ABI byte homes in one place: `A`, `X`, `Y`, and `$A0+` for SArgs;
5. preserve left-to-right semantics where expression evaluation can have
   effects.

This planner should be used for all ordinary user calls. Special cases are
allowed only when they are ABI facts, not incidental source shapes.

## Address Values

Address-like values deserve a first-class lowering path. They include:

- `@symbol`;
- `@record.field`;
- array decay;
- local/global array base address;
- array parameter pointer value;
- string literal storage address;
- function or machine-block label address where explicitly allowed.

The backend should classify these as address values and then materialize the
low/high bytes through the same word/address machinery used by assignments,
returns, and calls.

## Value Reads

Value reads should also have one path per width:

- byte reads: literals, storage, deref, pointer index, array index, call result,
  byte arithmetic/logical expressions;
- word reads: literals, storage, deref, pointer index, array index, address
  values, call result, word arithmetic expressions.

Condition lowering, assignment lowering, return lowering, and call lowering
should consume these paths instead of each reimplementing support for dynamic
indexes or pointer dereferences.

## State Tracking

All emitted instructions should flow through `NativeTrackedEmitter`. If a new
lowering helper needs an opcode wrapper, add it to the tracked emitter first so
processor-state invalidation stays centralized.

Do not rely on optimizer state when emitting through untracked/raw paths. Raw
machine data and unresolved labels are barriers unless the helper explicitly
models their effect.

## Tests And Validation

Each new semantic shape should get a tiny focused SemIR-native test. The test
should name the semantic behavior, not a TN-specific workaround.

Good test names:

- `native_array_parameters_can_be_forwarded_to_array_parameters`
- `native_conditions_accept_dynamic_pointer_byte_array_left_operands`
- `native_local_array_decay_can_assign_base_address_to_word`

Avoid tests named after a one-off production routine unless the routine itself
is the public behavior under test.

Use TN and toolkit files as integration pressure, not as the design itself. A
TN blocker should usually become a small semantic test first, then a general
helper improvement.

## Current Direction

The immediate SemIR-native direction is:

1. introduce/generalize value and address classifiers;
2. rebuild call argument lowering around a single planner;
3. route assignments, returns, conditions, and calls through the same
   byte/word/address materializers;
4. keep unsupported diagnostics precise enough to identify the missing semantic
   shape;
5. resist adding routine-specific fixes even when a TN blocker is obvious.

The desired end state is simple: SemIR describes meaning, the planner
classifies value shape, and the emitter materializes it into the ABI.

## Classification Implementation Plan

The first implementation should be deliberately small. It should improve
structure without trying to solve register allocation or global optimization.

### Step 1: Add Shape Types

Create a native classification module or section with small enums that borrow
SemIR nodes:

- `NativeByteValue`;
- `NativeWordValue`;
- `NativeAddressValue`;
- `NativePlaceValue`;
- `NativeCallResult`;
- `NativeUnsupportedShape`.

The first version can stay private to `semir_native.rs` or a nearby module. Do
not expose it as a stable compiler-wide API until it has survived a few
lowering slices.

Suggested initial shape vocabulary:

- literal byte/word;
- storage byte/word;
- address low/high source;
- array decay address;
- string literal address;
- byte dereference;
- byte indexed element;
- word dereference;
- word indexed element;
- user call result;
- computed byte expression;
- computed word expression;
- unsupported.

### Step 2: Classify Address Values First

Address values are the most scattered today, and they block call lowering.
Build one helper that recognizes:

- explicit address-of;
- implicit record address-of;
- array decay;
- local/global array base address;
- array parameter pointer value;
- string literal address;
- routine or machine-block labels where syntax explicitly permits them.

Then replace ad hoc `array_address_expr` use in assignment and call code with
the classifier.

### Step 3: Build Byte And Word Materializers

Add materializers that consume classified shapes:

- byte value to `A`;
- byte value to `X`;
- byte value to `Y`;
- byte value to a target slot;
- word value to target slot;
- word/address value low/high byte to `A`, `X`, `Y`, or SArgs slot;
- lvalue effective address to a chosen zero-page pointer pair.

These materializers should be the only places that know the opcode sequence for
loading a literal, storage byte, dereference, dynamic index, or call result.

### Step 4: Rebuild Call Argument Lowering

Introduce a call argument planner:

1. read the callee signature;
2. flatten arguments into ABI byte positions;
3. classify each byte source;
4. materialize bytes into public Action ABI homes;
5. preserve left-to-right evaluation for effectful expressions.

The current TN blocker around `MovePage` should be solved through this planner,
not by a `MovePage`-specific rule.

### Step 5: Migrate Consumers One At A Time

Move existing paths onto the classifiers in this order:

1. call arguments;
2. assignments;
3. returns;
4. conditions;
5. compound assignments;
6. record and pointer/index helpers.

Each migration should delete at least one local ad hoc shape check. If it only
adds another special case, it is probably the wrong slice.

### Step 6: Keep Diagnostics Shape-Oriented

Unsupported diagnostics should name the missing semantic shape, not a generic
fallback helper. Prefer messages like:

- `unsupported call argument byte: dynamic word expression high byte`;
- `unsupported address value: record field address`;
- `unsupported byte value: word array element used as byte`.

This makes TN/toolkit failures useful as architecture feedback instead of
forcing a debugger session for every new shape.

### Step 7: Validate With Focused Tests And TN

For every newly classified shape, add a small SemIR-native test. Then rerun the
current TN compile frontier to see which semantic shape is next.

The success criterion for this phase is not "TN compiles immediately." It is
that each TN blocker maps to a reusable classifier/materializer improvement.
