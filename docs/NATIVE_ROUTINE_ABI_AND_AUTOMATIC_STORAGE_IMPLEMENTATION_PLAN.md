# Native Routine ABI and Automatic Storage Implementation Plan

Snapshot date: 2026-09-03.

Status: in progress. Slice 0 was implemented and verified on 2026-09-03.

## Objective

Add a native Action! routine ABI in which every routine invocation owns its
parameters and automatic locals. Recursive and concurrently re-entered calls
must therefore observe distinct storage. Preserve the classic Atari Action!
ABI, where parameters and locals occupy fixed per-routine storage, without an
output or behavior change.

The work is a shared prerequisite for:

- Action! programs on 68000 Amiga systems and later other 68k platforms;
- an Exec-inspired, preemptively multitasking system written in Action! for
  the 65816;
- future native targets that cannot treat routine storage as one statically
  allocated global block.

The intended boundary is:

```text
source routine
    -> SemIR: source meaning, activation model, storage duration, signature
    -> NIR: invocation-scoped objects, abstract calls, ABI class, effects
    -> MIR68K: D/A registers, stack frame, call sequence, prologue/epilogue
    -> MIR65816: M/X state, hardware/software frame, near/far call sequence
```

NIR describes an automatic object but never a stack slot. A MIR backend may
keep an object in registers, give it a frame slot, split it into lanes, or
eliminate it when doing so preserves addressability and observable behavior.

## Current Baseline

The compiler currently has several useful pieces of this boundary:

- SemIR distinguishes parameters, routine locals, aliases, absolute storage,
  arrays, records, and callable signatures.
- verifier-clean NIR uses `ParamId` and `LocalId` in executable places.
- NIR calls carry typed arguments, a typed result, a signature, and
  conservative effects.
- MIR6502 has explicit call-home planning and a structured physical Action!
  ABI.
- MIR65816 and MIR68K independently consume verified NIR and already recognize
  parameters and locals as abstract address bases.
- target profiles distinguish the Atari compatibility ABI, 65816 native and
  small ABIs, and a 68k native ABI.

The missing contract is significant:

- parameters and locals are documented and laid out as fixed routine storage;
- `NirLocal` has a storage shape but no storage duration;
- `NirRoutine` has no structured activation model or routine signature;
- `NirCallableSignature.abi` is the executable string `"action"`;
- native MIR canaries use the combined address mode `FrameOrStatic` and have
  no frame-object or call-home plans;
- a local initializer is a load-image initializer, not an entry-time action;
- local array backing, aliases, relocations, inline machine code, and memory
  effects assume that a routine local can have one stable link-time address.

The benchmark suite deliberately omits recursive programs for this reason.

## Language and Compatibility Contract

### Classic activation

The Atari compatibility ABI keeps the existing behavior:

- every parameter and ordinary local denotes one fixed cell per routine;
- initializer data is present when the program image is loaded and is not
  implicitly restored on each call;
- taking and retaining a local address preserves its historical fixed-address
  behavior;
- current-location routines, cartridge/runtime entry points, local-address
  relocations, and inline machine blocks retain their observable layout;
- recursion remains unsupported by the storage model and must not be made to
  appear safe by an optimizer.

The initial migration does not change classic diagnostics. A later, separate
change may diagnose a proven recursive call group under the classic ABI.

### Native activation

The native ABI gives every invocation a distinct activation:

- value parameters are invocation values;
- assigning to a parameter changes only the current invocation's copy;
- every ordinary local is automatic and exists from routine entry through
  routine return;
- recursive and mutually recursive invocations receive distinct objects;
- an automatic object's address remains valid only while its invocation is
  active;
- absolute declarations and aliases of globals or absolute storage do not
  become automatic objects;
- an alias of an automatic local shares that invocation's lifetime and frame
  object;
- an uninitialized automatic object receives no implicit zero initialization;
- a declared initializer is evaluated or copied at every routine entry;
- all return and fallthrough paths perform the backend-required epilogue.

Returning or globally retaining a pointer to an automatic object is a lifetime
error in the program. The first implementation may conservatively permit it
while documenting the invalid lifetime, but optimizers must never extend the
object to static storage or assume that an escaped address is unobservable.
A later escape diagnostic must use SemIR/NIR identity and flow, not source-name
matching.

### Persistent native state

The first native milestone requires persistent state to be declared at module
or global scope. It does not add local `STATIC` syntax in the same migration.
If real programs show that scoped persistent state is necessary, add a
contextual `STATIC` storage modifier in a separate language-design slice. NIR
must nevertheless use an explicit per-local storage-duration field now so that
such a feature will not require another IR redesign.

### Initialization distinctions

Action! declaration syntax currently serves several different purposes. SemIR
must continue to resolve these before NIR:

- an absolute declaration or storage alias selects existing storage and does
  not become an entry-time assignment;
- a pointer/array backing declaration determines object representation;
- an ordinary native local initializer becomes entry-time initialization;
- an aggregate initializer may use an immutable static template followed by a
  per-entry copy;
- a descriptor for automatic array backing is constructed from the current
  activation's backing address, not a link-time relocation;
- a classic local initializer remains part of fixed routine storage.

No MIR backend may infer which interpretation was intended from initializer
syntax.

## ABI Contract

### ABI identities

Do not overload the target-wide `AbiId` with per-call decisions. Replace the
string ABI in `NirCallableSignature` with a structured, target-independent
calling-convention identity. The precise Rust shape may change, but the model
should distinguish at least:

```rust
pub enum NirCallConvention {
    TargetInternal,
    TargetPublic,
    Runtime,
    External(ExternalAbiId),
}
```

- `TargetInternal` is the ordinary Action!-to-Action! convention selected by
  the target ABI. Its physical placement may evolve with the backend.
- `TargetPublic` is a stable Action! entry used when a routine address escapes
  or the routine is exported across an independently compiled boundary.
- `Runtime` selects a target/runtime binding whose signature and effects are
  already verified.
- `External` reserves a stable identity for platform ABIs such as Amiga
  library calls; their register maps do not enter NIR.

Use stable IDs and a fact table if the set becomes open-ended. Do not retain an
executable ABI name as a string.

### Internal and observable entries

Ordinary direct calls may use `TargetInternal`. A routine requires a stable
entry convention when any of the following holds:

- its callable address is stored or passed indirectly;
- it is exported for separately compiled Action! code;
- a selected runtime or platform calls it;
- it is an interrupt, hook, startup, or other foreign entry;
- compatibility placement makes its entry or parameter homes observable.

The first implementation may use one native Action convention for both
internal and public calls. Keeping the identities separate prevents later
interprocedural optimization from silently changing an observable boundary.
Adapters or trampolines between Action!, C, Amiga-library, hook, interrupt, and
startup conventions belong to the relevant MIR/platform layer.

### Initial physical policies

The common migration does not standardize one physical ABI across 68k and
65816.

MIR68K should begin with a correctness-first call plan:

- explicit incoming argument locations and result location;
- an even-aligned frame extent suitable for an original 68000;
- a defined caller/callee-save set;
- addressable frame objects for mutated or address-taken parameters;
- one epilogue reached by every normal return;
- no dependence on an Amiga library base register in the internal ABI.

A stack-first implementation is acceptable as the first internal convention.
Register arguments can be selected later without changing NIR. Do not freeze
the first internal convention as the public ABI until Amiga callbacks,
separate compilation, and external-object interoperability have been designed.

MIR65816 must independently decide:

- native versus small-model pointer and call widths;
- `JSR`/`RTS` versus `JSL`/`RTL` boundaries;
- M/X width state at routine boundaries;
- hardware-stack, direct-page, or software-frame placement;
- access strategy when a frame exceeds stack-relative displacement range;
- which state a task context switch must preserve.

Those are MIR65816 decisions. NIR exposes sizes, alignments, liveness,
address-taking, call signatures, and storage duration, but no `S`, `D`, `DBR`,
`PBR`, M, or X concepts.

## Proposed SemIR and NIR Shape

The names below are illustrative. The invariant matters more than the exact
spelling.

```rust
pub enum SemActivationModel {
    ClassicStatic,
    NativeReentrant,
}

pub enum NirActivationModel {
    ClassicStatic,
    NativeReentrant,
}

pub enum NirStorageDuration {
    Automatic,
    RoutineStatic,
    External,
}

pub struct NirObjectLayout {
    pub size: ByteSize,
    pub alignment: ByteSize,
}

pub struct NirRoutine {
    pub id: RoutineId,
    pub signature: NirCallableSignature,
    pub call_convention: NirCallConvention,
    pub activation: NirActivationModel,
    // existing params, locals, temps, entry facts, and blocks
}

pub struct NirLocal {
    pub id: LocalId,
    pub duration: NirStorageDuration,
    pub layout: NirObjectLayout,
    // existing type, backing, initializer facts, and debug metadata
}
```

Parameters are inherently invocation-scoped under `NativeReentrant`.
Backends should receive enough facts to decide whether a parameter needs an
addressable home; NIR must not eagerly force every parameter into memory.

For arrays and records, NIR must carry the final target-selected cell extent,
alignment, backing extent, descriptor extent, and element stride. MIR must not
walk SemIR declarations to reconstruct a frame layout.

### Entry initialization

Automatic initialization is executable behavior and should be explicit before
MIR lowering:

- scalar initializers become ordinary typed NIR stores;
- address initializers become `AddrOf` plus a typed store;
- aggregate initializers use a verified static template and `CopyBytes`;
- automatic array descriptors receive the activation-local backing address and
  their declared size metadata on entry;
- generated initialization operations are ordered before the source routine
  body and retain ordinary volatile/effect rules.

Do not encode entry initialization as a special MIR prologue side channel. A
small NIR entry-initialization region or ordinary operations in the entry block
keeps the semantics visible to verification and optimization. Classic static
initializers remain data images and generate no entry stores.

### Storage identity and effects

An automatic `LocalId` names a lexical object, while each invocation owns a
different dynamic instance. Analyses must therefore pair storage identity with
duration:

- a direct nested call cannot access its caller's non-escaped automatic object
  merely because the callee has the same lexical `LocalId` in a recursive
  invocation;
- a pointer passed to a call may expose the pointed-to automatic object;
- a local address stored to global or unknown memory is escaped and must remain
  observable until routine exit;
- calls and foreign code remain conservative barriers until structured effects
  prove otherwise;
- automatic objects cannot be targets of load-time data relocations;
- aliases of automatic objects use the target object's dynamic identity.

Initially prefer conservative escape and call handling. The feature must be
correct before using non-escape facts to promote frame objects or remove
stores.

## Implementation Slices

Each slice is a vertical, verifier-clean change and should be committed
separately. Do not mix instruction-selection optimizations with the ABI
migration.

### Slice 0: contract and compatibility baselines

Status: complete. The target-shape contract now distinguishes classic static
and native reentrant activations and links to this plan. A compile-only fixture
captures parameters, fixed and initialized locals, aliases, local aggregates,
an escaped local address, an indirect call, and deliberately unsafe classic
recursion. Object, NIR, MIR6502, and map hashes are recorded in
`NIR_ATARI_BASELINES.md`, with focused tests proving the current fixed-frame
shape.

1. Add this contract to `NIR_TARGET_SHAPE.md` and cross-link it from the target
   independence plan.
2. Capture byte hashes, sizes, maps, and relevant NIR/MIR6502 snapshots for:
   parameters, scalar locals, initialized locals, local arrays, records,
   aliases, address-taking, current-location routines, indirect calls, cart
   runtime, and standalone runtime.
3. Add source-semantic tests demonstrating existing classic persistence and
   the known non-reentrant recursion limitation.
4. State that native automatic initialization happens on each entry while
   classic initialization remains load-time state.

Completion gate: no compiler behavior changes; the compatibility surface is
measurable.

Suggested commit:

```text
docs: define native routine activation and ABI contract
```

### Slice 1: structured call-convention and routine identity

1. Introduce `RoutineId` everywhere NIR currently uses a raw routine `u32`.
2. Replace `NirCallableSignature.abi: String` with a structured convention or
   stable convention ID.
3. Put a signature, call convention, and structured entry classification on
   every `NirRoutine`.
4. Include the convention in callable-signature interning, callable-pointer
   compatibility, and indirect-call verification.
5. Make direct, indirect, runtime, and external calls carry compatible
   convention identities.
6. Tighten the verifier to reject missing conventions, call/callee convention
   mismatches, and executable string ABI identities.
7. Preserve readable convention names only in the printer.

Completion gate: all existing calls are still classified as the classic
Action convention, and MIR6502 output is unchanged.

Suggested commit:

```text
nir: replace string call ABI with structured routine conventions
```

### Slice 2: activation and storage-duration facts

1. Add the selected activation model to the target ABI/profile facts.
2. Resolve it in semantic layout before NIR lowering.
3. Classify every stored declaration as automatic, routine-static, or
   external/aliased storage.
4. Add explicit final size and alignment facts for parameters, local cells,
   aggregate backings, and descriptors.
5. Make aliases inherit or reference the target duration rather than acquiring
   an independent duration.
6. Print the facts in focused fixtures and statistics.

Completion gate: Atari NIR says `classic-static`; 65816-native,
65816-small, and 68k NIR say `native-reentrant`, with no stack offsets or
register names in NIR.

Suggested commit:

```text
semir: resolve native activation and local storage duration
```

### Slice 3: verifier-clean automatic objects

1. Teach NIR storage analysis that automatic identity is invocation-relative.
2. Verify size, alignment, alias-duration, and activation-model consistency.
3. Reject load-time relocations whose target is automatic storage.
4. Verify that absolute and global aliases never acquire frame storage.
5. Preserve addressability when an automatic parameter or local is used by
   `AddrOf`, `CopyBytes`, a volatile operation, or foreign-code metadata.
6. Keep calls and opaque foreign code conservative for escaped automatic
   storage.

Completion gate: malformed mixed-duration NIR cannot pass
`backend::VerifiedNir`.

Suggested commit:

```text
nir: verify invocation-scoped automatic storage
```

### Slice 4: entry-time initialization

1. Split classic load-image initialization from native entry initialization in
   SemIR-to-NIR lowering.
2. Lower native scalar and pointer initializers to ordinary entry-block
   operations.
3. Lower aggregate initialization through immutable templates and
   `CopyBytes`.
4. Construct sized-array descriptors from their activation-local backing
   addresses on each entry.
5. Ensure uninitialized automatic objects remain uninitialized.
6. Cover early returns, empty routines, nested lexical declarations, aliases,
   and initializer evaluation order.

Completion gate: calling a native routine twice recreates its declared initial
state; calling a classic routine twice retains the historical fixed state.

Suggested commit:

```text
nir: lower native local initialization at routine entry
```

### Slice 5: promotion, effects, and escape safety

1. Audit NIR promotion, home elision, copy propagation, dead-store removal,
   and storage propagation for automatic duration.
2. Treat automatic objects whose address never escapes as private to the
   current activation.
3. Treat passed, returned, globally stored, or opaque-code-visible addresses
   conservatively.
4. Ensure recursive calls refer to a fresh callee activation rather than the
   caller's lexical storage object.
5. Add verifier or optimizer assertions that no pass changes an automatic
   object to static duration.
6. Defer aggressive interprocedural escape analysis.

Completion gate: optimization preserves recursive and address-taken fixtures
under both optimized and unoptimized NIR paths.

Suggested commit:

```text
nir: make storage optimization activation-aware
```

### Slice 6: preserve the classic Atari ABI

1. Map `ClassicStatic` NIR to the existing MIR6502 routine-storage layout.
2. Keep parameter homes, local labels, current-location entries, `RUNAD`,
   machine-block visibility, initializer relocations, and standalone/cart
   runtime binding unchanged.
3. Make MIR6502 explicitly reject `NativeReentrant` until a separate native
   6502 ABI is intentionally designed.
4. Run byte comparisons for every Slice 0 baseline.
5. Classify any NIR fixture changes as an intentional contract/printer change,
   not a generated-code change.

Completion gate: representative Atari binaries and all existing MIR6502
fixtures remain byte-identical except for explicitly approved metadata output.

Suggested commit:

```text
mir6502: preserve classic static activation explicitly
```

### Slice 7: MIR68K frame and call plans

1. Split `FrameOrStatic` into explicit static, automatic-frame, parameter, and
   external address forms.
2. Add MIR68K frame-object IDs with size, alignment, mutability,
   addressability, and source-owner facts.
3. Add a MIR68K ABI planner that maps typed arguments and results to abstract
   physical homes.
4. Lay out automatic objects, saved registers, spills, and outgoing call space
   with checked offsets and even 68000 alignment.
5. Generate structured prologue, epilogue, call-sequence, and return plans.
6. Force mutated/address-taken parameters into homes; permit immutable
   non-address-taken parameters to remain values.
7. Verify balanced stack effects over every CFG exit and recursive call.
8. Add canary snapshots for direct recursion, mutual recursion, an
   address-taken local, a record, a local array, an indirect call, and a call
   with enough arguments to require stack placement.

Completion gate: MIR68K represents a complete, internally consistent native
activation and call plan without consulting SemIR. Instruction encoding and
Hunk emission are not required by this slice.

Suggested commit:

```text
mir68k: plan native calls and automatic frames
```

### Slice 8: MIR65816 frame and call plans

1. Split static, automatic-frame, parameter, and external address forms in
   MIR65816.
2. Add independent MIR65816 frame objects and ABI homes; do not reuse MIR68K
   or MIR6502 machine types.
3. Specify native and small-model call boundaries, including code-pointer
   width and near/far return forms.
4. Specify M/X state on entry and exit and include it in call verification.
5. Select a correctness-first automatic-frame strategy and diagnose frame
   shapes the initial strategy cannot address.
6. Verify frame extent, stack-relative displacement, pointer width, bank
   constraints, and balanced return forms.
7. Add the same recursive, aggregate, address-taking, indirect-call, and large
   argument fixtures used for MIR68K.
8. Record the complete processor state that a future task switch must save;
   keep that state out of NIR.

Completion gate: both 65816 memory models independently consume the same
automatic-storage NIR contract and produce verified native call/frame plans.

Suggested commit:

```text
mir65816: plan native calls and automatic frames
```

### Slice 9: foreign code and observable boundaries

1. Diagnose native opaque machine code that refers to an automatic object as
   if it had a fixed link-time address.
2. Permit explicit address passing to foreign code only through structured
   operands/effects that the selected backend understands.
3. Preserve automatic homes required by public entries, callable pointers,
   runtime callbacks, and external ABI adapters.
4. Reject target-incompatible current-location and absolute-entry constructs
   with a focused diagnostic rather than silently making them static.
5. Document how future Amiga hooks, interrupts, and startup adapters select a
   non-internal ABI.

Completion gate: no native executable path can observe a fictitious fixed
address for automatic storage.

Suggested commit:

```text
compiler: guard native automatic storage at foreign boundaries
```

### Slice 10: recursion and reentrancy acceptance suite

1. Restore or add recursive permutation and queens kernels as native-only
   acceptance programs.
2. Add scalar recursion, mutual recursion, nested calls with address-taken
   locals, and repeated initialization tests.
3. Add an abstract re-entry test proving two live activations have different
   addresses for the same lexical local.
4. Verify public/indirect calls retain their selected convention.
5. When executable native emitters become available, run the same corpus in a
   68k emulator and a 65816 emulator and check stack balance after completion.
6. Document stack consumption per routine in maps/listings once physical frame
   layout is emitted.

Completion gate: the compiler core and both native MIRs prove recursion-safe
storage. Native binary execution becomes the matching completion gate for each
full backend, rather than weakening this common NIR contract in the meantime.

Suggested commit:

```text
tests: prove native recursive activation semantics
```

## Verification Matrix

After every slice that changes semantics, NIR, verification, or backend
lowering, run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Also run targeted checks for:

- semantic and SemIR fixtures covering storage duration and initialization;
- NIR verifier rejection of mixed or missing activation facts;
- MIR6502 fixture snapshots and representative byte comparisons;
- MIR68K and MIR65816 canary tests;
- classic cart and standalone runtime builds;
- optimized and unoptimized NIR paths.

Required behavioral cases:

| Case | Classic Atari ABI | Native ABI |
| --- | --- | --- |
| second call after modifying local | observes fixed routine cell | receives a new automatic object |
| initialized local | initialized in image, then persistent | initialized on every entry |
| direct recursion | non-reentrant storage model | distinct activation per depth |
| address-taken local | stable fixed address | stable only during activation |
| local alias | fixed target cell | current activation's target object |
| absolute/global alias | external/static storage | external/static storage |
| local aggregate backing | fixed routine data | activation-local backing |
| indirect/public call | classic observable Action ABI | stable native public ABI |
| opaque machine reference to local | fixed relocation permitted | fixed relocation rejected |

## Non-goals

- Do not introduce 32-bit Action! integer types, `ADDRESS`, or `SIZE` in this
  migration; the representation must remain ready for them.
- Do not implement the Amiga A6/LVO library ABI, C interoperability, Hunk
  output, or Workbench/CLI startup here.
- Do not implement a 65816 scheduler, interrupt veneer, context switch, or
  complete bank allocator here.
- Do not force one physical calling convention onto MIR6502, MIR65816, and
  MIR68K.
- Do not add closures, nested routines, destructors, exceptions, variable
  length arrays, dynamic stack allocation, or tail calls.
- Do not make classic Atari locals automatic or silently make native locals
  static because their address is taken.
- Do not optimize frame layout before the activation, effects, and ABI
  contracts verify cleanly.

## Risks and Decisions to Preserve

### Semantic compatibility

Automatic initialization and lifetime are intentional native-ABI semantics,
not an optimization. They must be selected before NIR optimization and printed
in diagnostics/maps. The classic ABI remains available for source that relies
on persistent local cells.

### Address escape

The compiler cannot generally prove that a pointer retained by unknown code is
not used after return. Begin conservatively. A future diagnostic or ownership
feature may improve safety, but silently promoting an escaped automatic object
to static storage would break recursion and hide bugs.

### Arrays and aggregate cost

Large automatic arrays can consume substantial stack or software-frame space.
Backends should report frame sizes and may diagnose target limits. They must
not silently allocate a large automatic local statically. Source can move
deliberately persistent or oversized objects to global storage.

### 65816 stack constraints

The native hardware stack is confined to bank zero and stack-relative
displacements are limited. The MIR65816 ABI must make its initial limits
explicit and leave room for a software frame or task-local direct-page design.
Changing that machine strategy must not require a NIR change.

### External stability

Internal ABI optimization is allowed only behind non-observable direct calls.
An exported, address-taken, callback, startup, interrupt, or independently
compiled entry needs a stable convention or an adapter. The verifier must make
that distinction structural.

## Definition of Done

The common migration is complete when:

- every verifier-clean routine has a structured identity, signature, calling
  convention, and activation model;
- every stored local has explicit duration, size, and alignment;
- native ordinary locals and parameters are invocation-scoped while classic
  Atari routine storage is unchanged;
- native local initialization is explicit entry-time NIR behavior;
- NIR effects and optimization distinguish lexical identity from dynamic
  activation identity;
- no executable ABI decision depends on the string `"action"`;
- MIR6502 preserves classic output and rejects unsupported activation models;
- MIR68K and MIR65816 independently produce verified frame and call plans for
  recursion, indirect calls, address-taken storage, arrays, and records;
- automatic storage never receives a load-time address relocation;
- opaque foreign code cannot silently observe automatic storage as a fixed
  symbol;
- the required NIR sweep and complete Rust test suite pass.

At that point, full MIR68K and MIR65816 emitters can implement their physical
prologues, epilogues, call sequences, and frame accesses without recovering
source-language facts or revising NIR.
