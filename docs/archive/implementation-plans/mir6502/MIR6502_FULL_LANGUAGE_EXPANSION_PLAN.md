# MIR6502 Full-Language Expansion Plan

Snapshot date: 2026-06-01.

This note is the post-scalar expansion plan for MIR6502. It is intended to be
used directly as a Codex execution plan once the scalar MIR6502 path from
`docs/MIR6502_IMPLEMENTATION_PLAN.md` is working end-to-end.

Canonical references:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md` defines the MIR6502 machine model,
  phase model, verifier expectations, value/address split, effects, and deferred
  opcode families.
- `docs/MIR6502_IMPLEMENTATION_PLAN.md` defines the scalar-first implementation
  plan.
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md` defines what MIR6502 must decide
  before tracked emission.
- `docs/ACTION_STORAGE_LAYOUT.md` records target-relevant Action! storage and ABI
  layout facts.
- `docs/SEMIR_NATIVE_ARCHITECTURE.md` records the classifier/materializer/emitter
  split that MIR6502 should preserve at the NIR-to-target layer.

This plan must not weaken those contracts. If a feature requires a new MIR form,
first decide whether it belongs in NIR, MIR6502, materialization, or tracked
emission. Do not push semantic recovery into the tracker.

## North Star

MIR6502 becomes full-language-capable by preserving every target-relevant fact
from verifier-clean NIR, selecting target strategy, materializing that strategy
into pre-emission MIR, and then handing concrete actions to tracked emission.

The expansion path is:

```text
scalar-correct MIR6502
  -> complete storage/backing facts
  -> value/place/address classifier
  -> address/static/call support
  -> arrays/pointers/records
  -> machine blocks and builtins
  -> indirect calls and aggregate initialization
  -> zero-page allocation
  -> verified MIR peepholes
```

Do not chase source syntax one construct at a time. Add general MIR capabilities
that cover families of language features.

## Preconditions

Start this plan only after the scalar path is verified end-to-end:

```text
NIR scalar profile
  -> pre-materialization MIR6502
  -> post-materialization MIR6502
  -> pre-emission MIR6502
  -> tracked emission
```

Required scalar support before full-language expansion:

- `BYTE`, `CHAR`, `CARD`, and `INT` scalar loads/stores;
- direct global/local/param/static/absolute storage;
- absolute-backed aliases such as `BYTE COLOR=$02C8`;
- constants, temps, `Move`, `Extend`, `Truncate`, and `LeaAddr`;
- byte/word `Add`, `Sub`, `And`, `Or`, and `Xor`;
- helper selection for `Mul`, `Div`, `Mod`, `Lsh`, and `Rsh`;
- compares, branches, returns, and `EXIT`;
- pre-emission verifier coverage;
- tracked emission for the scalar pre-emission subset.

If scalar MIR can still lose an absolute address, call effect, word width,
branch target, or storage backing fact, fix that before starting this plan.

## Codex Execution Rule

Execute this plan as small, test-gated slices.

For every milestone below:

1. make only the changes required for that milestone;
2. keep existing scalar fixtures green;
3. add focused fixtures for each newly supported shape;
4. tighten the verifier when a new shape becomes legal;
5. reject unsupported shapes with precise diagnostics;
6. commit before starting the next milestone;
7. use commit messages of the form `mir6502: <short imperative summary>`.

Do not mix feature support, peepholes, broad refactors, and emission changes in
one commit. If a milestone is too large, split it into compiling sub-slices.

## Required Checks

Run the relevant checks after each slice:

```sh
cargo test
cargo test mir6502_fixtures_match_snapshots
```

If NIR lowering or fixtures are touched, also run the NIR checks if present:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
```

If MIR sweep support exists, run:

```sh
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
```

If end-to-end MIR backend support exists, include representative compile checks
for newly supported fixtures. If a check cannot be run, say so in the commit or
PR notes.

## Red Lines

Do not allow full-language support to make MIR6502 weaker.

MIR6502 must not:

- inspect SemIR to recover missing facts;
- parse printed NIR/TAC/MIR;
- use source names or source syntax as executable identity;
- hide target-relevant storage backing facts from the printer/verifier;
- ask tracked emission to decide whether storage is global, local, absolute,
  indexed, dereferenced, or descriptor-backed;
- ask tracked emission to choose ABI homes, helper calls, or semantic lowering
  strategy;
- lower machine blocks without structured payload/effect facts or a precise
  unsupported diagnostic;
- add target peepholes before the represented feature is correct and verified.

## Milestone 0: Scalar Baseline Closure

Goal: make sure the scalar path is complete enough to be a foundation.

Scope:

- Preserve absolute-backed globals and aliases.
- Print storage backing facts clearly.
- Verify that absolute aliases do not silently become ordinary allocated globals.
- Complete scalar pre-materialization, materialization, pre-emission verification,
  and tracked emission.

Required fixtures:

```text
scalar_byte_store.act
scalar_word_store.act
absolute_alias_store.act
absolute_set_store.act
byte_arithmetic.act
word_arithmetic.act
cast_extend_truncate.act
address_of_scalar.act
if_compare.act
while_compare.act
return_byte.act
return_word.act
exit.act
```

Acceptance criteria:

- `BYTE COLOR=$02C8; COLOR=4` preserves `$02C8` in MIR before emission.
- Word operations either remain valid pre-materialization pseudo ops or are
  byte-expanded/helper-selected before pre-emission.
- Tracked emission receives only concrete scalar actions.

Suggested commits:

```text
mir6502: preserve absolute-backed globals
mir6502: complete scalar pre-materialization lowering
mir6502: materialize scalar word operations
mir6502: verify scalar pre-emission invariants
mir6502: emit scalar MIR through tracked emission
```

## Milestone 1: Complete Storage And Layout Facts

Goal: make every target storage family representable and printable before richer
expression lowering.

Scope:

Represent these storage families in MIR tables and `MirFrame`/`MirStorageSlot`:

```text
ordinary global scalar
ordinary local scalar
routine parameter
absolute-backed alias
static data
routine storage cell
spill cell
fixed zero-page cell
virtual zero-page cell
inline array storage
descriptor storage
array backing storage
record storage
machine block/static payload storage
```

Implementation notes:

- Preserve NIR stable IDs as executable identity.
- Keep display names as printer metadata only.
- Model absolute aliases as either absolute-backed globals or direct
  `MirMem::Absolute` uses.
- Keep fixed zero-page locations separate from allocatable virtual zero-page
  slots.
- Do not assign real zero-page addresses in this milestone.

Printer requirement:

```text
global g0 COLOR: byte absolute $02C8
global g1 x: byte storage global+0
local l0 tmp: word frame+0
static s0 str0: bytes [...]
```

Required fixtures:

```text
absolute_alias_byte.act
absolute_alias_card.act
global_scalars_layout.act
local_scalars_layout.act
routine_params_layout.act
static_string_layout.act
```

Acceptance criteria:

- MIR snapshots expose storage backing facts.
- Verifier rejects storage references with missing or inconsistent backing.
- Absolute aliases do not allocate ordinary storage.

Suggested commits:

```text
mir6502: print storage backing facts
mir6502: model absolute-backed aliases
mir6502: model routine parameter homes
mir6502: model local and spill storage homes
mir6502: verify storage backing facts
```

## Milestone 2: MIR Value, Place, And Address Classifier

Goal: centralize MIR target-shape decisions instead of scattering special cases
across assignments, calls, conditions, and returns.

Scope:

Add a classifier layer under `src/mir6502/`, for example:

```text
src/mir6502/classify.rs
```

Suggested shape enums:

```rust
pub enum MirValueShape {
    ConstByte,
    ConstWord,
    StorageByte,
    StorageWord,
    AddressValue,
    CallableAddress,
    DerefValue,
    IndexedElement,
    FieldElement,
    CallResult,
    Computed,
    Unsupported,
}

pub enum MirPlaceShape {
    DirectMemory,
    AbsoluteMemory,
    StaticMemory,
    InlineArrayElement,
    DescriptorArrayElement,
    PointerDeref,
    RecordField,
    Unsupported,
}

pub enum MirAddressShape {
    Direct,
    Absolute,
    Static,
    InlineArrayBase,
    DescriptorBackingBase,
    PointerValue,
    ZeroPageStaged,
    RoutineAddress,
    Unsupported,
}
```

The exact Rust shape may differ. The invariant matters more than the enum names:
assignment lowering, value reads, address-of lowering, call planning, returns,
and conditions should consume shared classification helpers.

Acceptance criteria:

- Assignment lowering consumes `MirPlaceShape` or equivalent.
- Expression/value lowering consumes `MirValueShape` or equivalent.
- Address-of and array decay consume `MirAddressShape` or equivalent.
- Unsupported shapes fail before emission with precise diagnostics.

Required fixtures:

```text
classify_scalar_storage.act
classify_absolute_alias.act
classify_address_value.act
classify_unsupported_deref_before_support.act
```

Suggested commits:

```text
mir6502: add value and place classifier
mir6502: classify address values
mir6502: route assignments through place classification
mir6502: route value reads through value classification
```

## Milestone 3: Static Data, Strings, And Address Values

Goal: make address-like values first-class MIR values.

Scope:

Support:

```text
string literal static bytes
StaticAddr
GlobalAddr
RoutineAddr
array decay where the storage kind is already supported
@symbol
@record.field when record field offsets are available
routine/function address values where legal
```

MIR requirements:

- `LeaAddr` or equivalent address-materialization op;
- low/high address byte materialization;
- address values assignable to word/pointer storage;
- address values passable as call args;
- address values returnable where the language permits.

Required fixtures:

```text
string_literal_address.act
assign_static_address_to_pointer.act
address_of_global.act
address_of_local.act
address_of_array_base.act
routine_address_value.act
```

Acceptance criteria:

- Address values are represented as word values, not source strings.
- String/static bytes are represented exactly once.
- `display` text, if present, is diagnostics only; bytes are authoritative.

Suggested commits:

```text
mir6502: lower static data and string addresses
mir6502: materialize address values
mir6502: pass address values through ABI homes
```

## Milestone 4: Full Direct Call Support

Goal: replace call placeholders with a real ABI-driven call planner for direct
calls.

Scope:

Add or extend:

```text
src/mir6502/abi.rs
src/mir6502/call_plan.rs
```

Planner inputs:

```text
NirSignature
NirCallee
NirEffects
MIR argument value/address shapes
```

Planner outputs:

```text
MirCallAbi
Vec<MirArgHome>
Option<MirResultHome>
MirEffects
```

Support in this milestone:

```text
direct user procedure calls
direct user function calls returning byte/word
runtime calls with known signatures
builtin calls that lower through explicit mappings
OS/opaque calls as conservative barriers
```

Rules:

- Keep argument packing in one place.
- Preserve left-to-right evaluation where argument evaluation can have effects.
- Preserve `clobbers`, `preserves`, memory effects, stack effects, OS flags, and
  opaque flags.
- Reject calls without complete signature/effect facts.

Required fixtures:

```text
call_no_args.act
call_byte_arg.act
call_word_arg.act
call_many_args_sargs.act
func_returns_byte.act
func_returns_word.act
call_with_address_arg.act
builtin_putchar.act
runtime_helper_call.act
os_call_opaque_barrier.act
```

Acceptance criteria:

- Call fixtures show concrete ABI homes.
- Calls remain barriers according to effects.
- Verifier rejects missing ABI/effects.

Suggested commits:

```text
mir6502: add call ABI planner
mir6502: lower direct procedure calls
mir6502: lower direct function returns
mir6502: lower builtin calls through ABI planner
mir6502: model OS call barriers
```

## Milestone 5: Inline Byte Arrays And Strings

Goal: support inline byte-addressable arrays before descriptor-backed or pointer
arrays.

Scope:

Support:

```text
BYTE ARRAY a(n) where n <= inline threshold
CHAR ARRAY a(n)
STRING a(n)
initialized inline byte arrays
constant index read/write
dynamic BYTE index read/write
array decay to base address
passing inline byte array as an array parameter
```

MIR requirements:

- inline array storage/backing facts;
- constant index -> direct memory offset;
- dynamic index -> selected indexed address form where legal;
- base-address materialization;
- element-size facts from NIR, never source syntax.

Required fixtures:

```text
byte_array_const_read.act
byte_array_const_write.act
byte_array_dynamic_read.act
byte_array_dynamic_write.act
char_array_dynamic_read.act
string_index_read.act
array_decay_to_pointer.act
pass_byte_array_param.act
initialized_byte_array.act
initialized_string.act
```

Acceptance criteria:

- Constant indexing lowers to direct offset when safe.
- Dynamic indexing lowers to a MIR address strategy, not source syntax.
- Array decay produces a word address value.

Suggested commits:

```text
mir6502: model inline byte array storage
mir6502: lower inline byte array constant indexes
mir6502: lower inline byte array dynamic indexes
mir6502: materialize inline array base addresses
mir6502: pass inline byte arrays as array arguments
```

## Milestone 6: Unsized And Pointer-Backed Arrays

Goal: support array variables that are pointer cells.

Scope:

Support:

```text
BYTE ARRAY p
CARD ARRAY p
INT ARRAY p
address initializers such as BYTE ARRAY screen=$580
pointer-backed constant index read/write
pointer-backed dynamic index read/write
passing unsized arrays as array parameters
```

MIR requirements:

- unsized array storage as a two-byte pointer cell;
- address initializer materialization into low/high pointer bytes;
- pointer value load from storage;
- pointer staging into zero-page pair;
- `MirAddr::IndirectIndexedY` or equivalent after staging;
- element-size scaling for word arrays.

Required fixtures:

```text
unsized_byte_array_address_initializer.act
unsized_byte_array_const_read.act
unsized_byte_array_dynamic_read.act
unsized_byte_array_dynamic_write.act
unsized_card_array_const_read.act
unsized_card_array_dynamic_read.act
unsized_card_array_dynamic_write.act
pass_unsized_array_param.act
```

Acceptance criteria:

- Unsized array variables occupy pointer-width storage.
- Pointer-backed array access does not pretend the array is inline storage.
- Zero-page staging is explicit in MIR/materialization.

Suggested commits:

```text
mir6502: model unsized array pointer storage
mir6502: lower unsized array address initializers
mir6502: stage pointer-backed byte array addresses
mir6502: lower pointer-backed byte array indexes
mir6502: lower pointer-backed word array indexes
```

## Milestone 7: Descriptor-Backed Arrays

Goal: support sized non-byte arrays and large byte arrays that use descriptors
and backing storage.

Scope:

Support:

```text
CARD ARRAY a(n)
INT ARRAY a(n)
large BYTE ARRAY a(n) when descriptor-backed
local descriptor-backed arrays
initialized descriptor-backed arrays
constant index read/write
dynamic index read/write
array decay to backing data pointer
passing descriptor-backed arrays as array parameters
```

MIR requirements:

- descriptor storage facts;
- backing storage facts;
- descriptor bytes 0..1 as backing pointer;
- descriptor bytes 2..3 as backing byte size when present;
- descriptor pointer load and staging;
- `MirAddr::IndirectIndexedY` for dynamic descriptor-backed indexing;
- element-size scaling.

Required fixtures:

```text
card_array_const_read.act
card_array_const_write.act
card_array_dynamic_read.act
card_array_dynamic_write.act
large_byte_array_dynamic_read.act
local_card_array_dynamic_read.act
pass_card_array_param.act
initialized_card_array_layout.act
descriptor_points_to_backing.act
```

Acceptance criteria:

- Descriptor and backing storage both appear in MIR storage facts.
- Descriptor-backed array decay passes backing pointer, not descriptor address.
- Dynamic indexes stage the element address explicitly.

Suggested commits:

```text
mir6502: model descriptor-backed array storage
mir6502: load array descriptor backing pointers
mir6502: lower descriptor-backed constant indexes
mir6502: lower descriptor-backed dynamic indexes
mir6502: pass descriptor-backed arrays as array arguments
```

## Milestone 8: Pointer Dereference

Goal: support pointer-value loads/stores and pointer dereference places.

Scope:

Support:

```text
p^
p^ = value
@x assigned to pointer
pointer comparison
pointer nonzero conditions
byte and word pointer dereference
```

MIR requirements:

- pointer value materialization as word value;
- zero-page pointer staging;
- indirect-indexed address form for dereference;
- byte/word load/store through staged pointer;
- conservative effects for unknown pointed memory;
- verifier coverage that dereference accesses do not become ordinary globals.

Required fixtures:

```text
byte_pointer_deref_read.act
byte_pointer_deref_write.act
word_pointer_deref_read.act
word_pointer_deref_write.act
address_of_to_pointer.act
pointer_nonzero_condition.act
pointer_compare.act
```

Acceptance criteria:

- Pointer dereference always goes through an explicit address strategy.
- Unknown pointed memory is effect-sensitive.
- Tracked emission is not asked to infer pointer meaning.

Suggested commits:

```text
mir6502: classify pointer dereference places
mir6502: stage pointer dereference addresses
mir6502: lower byte pointer dereference
mir6502: lower word pointer dereference
mir6502: model pointer memory effects conservatively
```

## Milestone 9: Records And Fields

Goal: lower records through byte offsets and target storage facts.

Scope:

Support:

```text
record variable storage
field loads/stores
@record.field
nested record fields
record fields behind pointers when NIR provides exact offsets
record fields inside arrays when NIR provides exact offsets
```

MIR requirements:

- record storage slot with size;
- field byte offset from NIR;
- field type/width facts;
- offset composition for nested fields;
- address materialization for field places.

Rules:

- No field names as executable MIR identity.
- If NIR lacks offset facts, reject before MIR lowering or fix NIR.

Required fixtures:

```text
record_field_byte_store.act
record_field_word_store.act
record_field_read.act
address_of_record_field.act
nested_record_field.act
record_pointer_field_read.act
record_array_field_read.act
```

Acceptance criteria:

- MIR snapshots show numeric offsets, not source field names.
- Verifier rejects unresolved field facts.
- Field accesses compose with address-of, deref, and index paths only when facts
  are exact.

Suggested commits:

```text
mir6502: lower record fields through byte offsets
mir6502: materialize record field addresses
mir6502: lower nested record field places
mir6502: verify record field storage facts
```

## Milestone 10: Control-Flow Completeness

Goal: support every verifier-clean NIR control-flow shape.

Scope:

Support:

```text
IF / ELSE / FI
WHILE / DO / OD
FOR loops after NIR normalization
EXIT inside loops
nested loops
short-circuit AND/OR after NIR CFG lowering
constant branches that survived NIR
function returns from branches
signed and unsigned relational conditions
pointer nonzero conditions
```

MIR requirements:

- bool temp branch support;
- flag-test branch support after materialization;
- byte equality/relational materialization;
- word equality/relational materialization;
- signed `I16` relational lowering;
- multi-block compare sequences where a single flag test is insufficient.

Required fixtures:

```text
nested_if.act
while_loop.act
for_loop_byte.act
for_loop_word.act
exit_loop.act
short_circuit_and.act
short_circuit_or.act
signed_int_compare.act
pointer_condition.act
return_from_if.act
```

Acceptance criteria:

- All branch targets are stable block IDs.
- Compare operands are not duplicated inside `MirCond`.
- Signed word comparisons have focused tests before enabling.

Suggested commits:

```text
mir6502: materialize loop branches
mir6502: materialize signed word comparisons
mir6502: lower short-circuit branch graphs
mir6502: verify structured control-flow targets
```

## Milestone 11: Machine Blocks And Raw Payloads

Goal: preserve or reject inline machine-code blocks at the MIR boundary.

Scope:

Support machine blocks only when NIR carries structured payloads and effects.

Represent:

```text
raw bytes
raw words
label definitions
label references
global/static/routine references
source span / diagnostics metadata
effects and barriers
```

Rules:

- Machine blocks are opaque by default.
- If payload is not structured enough to preserve, reject with a precise
  diagnostic before emission.
- Tracked emission writes raw payload bytes and invalidates state; it must not
  parse machine-block source text.

Required fixtures:

```text
machine_block_bytes.act
machine_block_label_ref.act
machine_block_global_ref.act
machine_block_unsupported_text.act
machine_block_effect_barrier.act
```

Acceptance criteria:

- Structured payloads survive to MIR.
- Unsupported payloads fail before emission.
- Effects/barriers are printed and verified.

Suggested commits:

```text
mir6502: preserve structured machine block payloads
mir6502: verify machine block effects
mir6502: reject unsupported machine block payloads
mir6502: emit machine block payloads as barriers
```

## Milestone 12: Builtins, Runtime, And OS Interactions

Goal: route builtins and runtime/OS calls through explicit MIR call targets and
effects.

Scope:

Support:

```text
known Action! builtins
runtime arithmetic helpers
SArgs / argument frame helpers
OS calls
resident-library calls where modeled
```

MIR requirements:

- `BuiltinId` or equivalent to `MirCallTarget` mapping;
- known runtime helper declarations and targets;
- fixed zero-page ABI homes;
- SArgs frame packing;
- conservative OS-call effects;
- stack effects;
- clobbers/preserves.

Required fixtures:

```text
builtin_putchar_byte.act
builtin_print_string.act
runtime_mul_word.act
runtime_div_word.act
runtime_mod_word.act
runtime_shift_word.act
os_call_opaque_barrier.act
sargs_many_args.act
```

Acceptance criteria:

- Builtins do not lower through source names.
- Runtime helpers carry ABI and effects.
- OS calls are opaque barriers unless precise effects exist.

Suggested commits:

```text
mir6502: map builtins to MIR call targets
mir6502: resolve runtime helper targets
mir6502: pack SArgs frames
mir6502: model OS call barriers
```

## Milestone 13: Indirect Calls And Callable Values

Goal: support typed callable values and indirect calls.

Scope:

Support:

```text
routine address values
callable variables
indirect PROC calls
indirect FUNC calls
routine pointer assignment
callable parameters where NIR supports them
```

MIR requirements:

- `RoutineAddr` and callable address materialization;
- indirect `MirCallTarget`;
- call signature verification;
- ABI planning for indirect calls;
- conservative effects unless known.

Required fixtures:

```text
routine_address_assignment.act
indirect_proc_call.act
indirect_func_call_byte.act
indirect_func_call_word.act
callable_param_forwarding.act
```

Acceptance criteria:

- Indirect callees are typed 16-bit callable values.
- Verifier rejects untyped indirect calls.
- Indirect calls do not consult SemIR for signature recovery.

Suggested commits:

```text
mir6502: materialize callable addresses
mir6502: lower indirect procedure calls
mir6502: lower indirect function calls
mir6502: verify indirect call signatures
```

## Milestone 14: Aggregate And Data Initialization

Goal: make initialized data byte-exact and self-contained in MIR.

Scope:

Support:

```text
initialized scalars
initialized byte arrays
initialized strings
initialized CARD/INT arrays
initialized unsized arrays
descriptor + backing storage
routine-local initialized arrays
static data references
zero-fill
```

MIR requirements:

- authoritative bytes for static/global initialized data;
- descriptor patching facts;
- backing storage references;
- alignment if needed;
- mutable/section facts;
- verifier checks that descriptors point to valid backing storage.

Required fixtures:

```text
initialized_scalar_storage.act
initialized_byte_array_storage.act
initialized_string_storage.act
initialized_card_array_storage.act
initialized_unsized_card_array_storage.act
local_initialized_array_storage.act
zero_filled_storage.act
descriptor_backing_reference.act
```

Acceptance criteria:

- Emitted bytes come from MIR payloads, not reconstructed source text.
- Descriptor/backing references are valid and printable.
- Zero-fill is explicit enough for emission/load-file layout.

Suggested commits:

```text
mir6502: lower initialized scalar storage
mir6502: lower initialized byte array storage
mir6502: lower initialized descriptor-backed arrays
mir6502: verify descriptor backing references
```

## Milestone 15: Zero-Page Allocation

Goal: allocate virtual zero-page slots after correctness is established.

Scope:

Support:

```text
virtual zero-page slots
fixed ABI zero-page slots
temporary pointer pairs
lifetime ranges
non-overlap constraints
call/barrier clobber constraints
fallback to fixed scratch policy where necessary
```

Rules:

- Do not confuse fixed ABI zero-page slots with allocatable virtual zero-page
  temps.
- Zero-page allocation is a MIR pass, not a tracked-emission guess.
- Indexed addressing wraparound rules must be respected.

Required fixtures:

```text
two_pointer_temps_no_overlap.act
array_index_nested_expr.act
call_clobbers_zp_temp.act
runtime_helper_uses_fixed_zp.act
zero_page_indexed_wraparound_guard.act
```

Acceptance criteria:

- Virtual zero-page slots are assigned before pre-emission unless emission
  explicitly owns final assignment.
- Verifier catches overlapping live zero-page slots.
- Calls and barriers invalidate or preserve zero-page facts according to effects.

Suggested commits:

```text
mir6502: compute zero-page temp lifetimes
mir6502: allocate virtual zero-page slots
mir6502: verify zero-page allocation constraints
```

## Milestone 16: MIR Peepholes And Local Target Optimizations

Goal: add target-specific cleanup only after full feature correctness.

Allowed optimizations:

```text
compare/branch fusion
redundant local load removal
local A reuse when verified
zero-page encoding preference when byte-equivalent
constant address low/high folding
helper selection improvements
branch layout hints
```

Rules:

- Peepholes operate only on verified MIR.
- Peepholes must preserve effects and barriers.
- Peepholes must not recover source semantics.
- Hardware registers, unknown absolute memory, pointer writes, calls, OS/runtime
  calls, and machine blocks are conservative unless effects prove otherwise.

Required fixtures:

```text
peephole_redundant_load.act
peephole_compare_branch_fusion.act
peephole_zero_page_direct.act
peephole_no_hardware_store_delete.act
peephole_call_barrier_preserved.act
```

Acceptance criteria:

- Verify before and after every peephole pass in tests/debug paths.
- Barrier-preservation tests exist.
- Peepholes are optional and can be disabled for debugging.

Suggested commits:

```text
mir6502: fuse compare branch patterns
mir6502: remove redundant local loads
mir6502: prefer equivalent zero-page encodings
mir6502: preserve barriers in peepholes
```

## Cross-Cutting Verifier Work

Every milestone should strengthen verification.

The verifier should eventually check:

```text
storage IDs and backing facts are valid
absolute aliases do not allocate ordinary storage
memory destinations are written only through Store
operation definitions target MirDef only
word ops either remain pre-materialization or are byte-expanded/helper-selected
byte Add/Sub carry chains are explicit and unbroken
calls have ABI/effects
machine blocks have payload/effects or are rejected
pointer/index/field places have exact address facts
zero-page virtual slots are allocated before pre-emission
pre-emission MIR has no unsupported pseudo ops
tracked emission receives only concrete-enough actions
```

If a feature is added without a verifier rule that prevents malformed MIR from
reaching emission, the slice is incomplete.

## Suggested Full-Language Commit Series

After the scalar path is green, use this high-level sequence:

```text
mir6502: complete storage backing model
mir6502: add value and place classifier
mir6502: lower static data and address values
mir6502: add call ABI planner
mir6502: lower direct and runtime calls
mir6502: lower inline byte arrays
mir6502: lower unsized array pointers
mir6502: lower descriptor-backed arrays
mir6502: lower pointer dereferences
mir6502: lower record fields
mir6502: complete control-flow materialization
mir6502: preserve structured machine blocks
mir6502: lower builtins and OS calls
mir6502: lower indirect calls
mir6502: lower aggregate initialization
mir6502: allocate zero-page slots
mir6502: add verified MIR peepholes
```

## Suggested First Codex Task For This Plan

Use this only after the scalar backend profile is working end-to-end:

```text
Implement MIR6502 full-language expansion Milestone 1 only.

Goal:
- Extend MIR storage/backing tables so every global/local/static/param storage
  entry prints its backing facts.
- Preserve absolute-backed aliases as either absolute-backed globals or direct
  MirMem::Absolute references.
- Add verifier checks for missing or inconsistent storage backing.
- Add focused fixtures for absolute alias and scalar storage layout.
- Do not add arrays, pointers, records, calls, peepholes, zero-page allocation,
  or emission changes.

Required checks:
- cargo test
- cargo test mir6502_fixtures_match_snapshots
- cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502, if available

Suggested commit:
- mir6502: complete storage backing model
```
