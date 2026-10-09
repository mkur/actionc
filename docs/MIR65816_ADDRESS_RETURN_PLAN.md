# MIR65816 returned record address implementation plan

Status: **slices 0–3 implemented and qualified**.

[Final evidence](benchmarks/65816-address-returns/README.md) records the checked
21-byte, zero-frame `Chain` sequence in every profile, complete application and
resource deltas, native/hosted qualification and serial compiler-cost acceptance.
The original application size, representative-size, private-traffic and
unchecked-provider gates remain open.

Select a pure computed address together with its native Return consumer, before
allocating intermediate homes. The motivating expression is
`RETURN(MYDOSTYPES.ChainCursor POINTER(@file.storage(0)))` in
`lib/mydos/mydosfile.act`, routine `MYDOSFILE.Chain`. Implement general typed
MIR65816 behavior for returned field/array-element addresses; no Exec-specific
selector or source change is needed.

The objective is one checked 24-bit address calculation into A/X, with no
intermediate DP or stack writes. A wrapper containing only this calculation
should have a zero frame. Preserve the public ABI, source evaluation order,
exact source reads, native state and interruption behavior.

## Current lowering and baseline

Compiler `9624da9a` produces this MIR shape in the frozen Exec816 workload:

```text
t0 = Load3 immutable incoming file parameter
t1 = AddressOf3 indirect(t0), displacement 82
t2 = AddressOf3 indirect(t1), displacement 0, index 0, stride 1
t3 = Cast Pointer 3 -> 3, t2
Return t3
```

The incoming pointer Load already borrows its parameter home. The constant-index
selector already folds zero, and the pointer cast already shares the result
home without emitting a separate copy. However, that folding happens after
storage decisions: the first address is materialized in DP, the second in a
stack home, and Return reloads the pointer into A/X. Eliminating an identity
cast instruction alone cannot remove this round trip.

| Profile | Routine bytes | Instructions | Fixed frame | Spill extent | Local peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 50 | 29 | 8 | 8 | 8 |
| Optimized guarded | 72 | 39 | 8 | 8 | 8 |
| Raw guarded | 72 | 39 | 8 | 8 | 8 |

The [completed call-flow scorecard](benchmarks/65816-call-flow-stage6/README.md)
provides the authenticated application baseline: Exec816 `57df0d7`, 256 files,
1,156 routines and all three profiles. Return the computed pointer value;
never read the contents of `file.storage` while forming its address.

## Scope and ownership

SemIR/NIR retain record layout, lvalue legality, pointer typing and evaluation
meaning. This work consumes resolved MIR byte displacements, widths, address
spaces and stable value/storage identities. No shared NIR form, frontend rule
or record-layout change is planned.

MIR65816 common placement owns the complete address expression and its sole
Return use. Prefer extending the existing deferred address/component owner:
omitted address and alias operations need not promise a live A/X result.
Their pure computation is evaluated at the checked Return consumer, which
establishes the ordinary native result before existing teardown and RTL.
Do not label an address result as a callee-origin `NativeOutput`.

Admission initially requires a same-block chain of at most sixteen pure
address/identity operations ending at Return. Every intermediate has one
definition and one logical operand occurrence across the complete routine.
Include unreachable, address/index, call-target, terminator and edge uses in
that census. No unrelated operation may intervene.

Use a complete captured private pointer home or an already-qualified borrowed
incoming/private source. An observable pointer Load retains its original site
and real capture; this plan does not defer external, indirect or volatile
reads. Borrowed reads must retain their original definition/use identity while
authorizing their actual physical consumption at Return. Revalidate the source
against the final frame; a producer-site binding cannot be looked up at a
different site without that explicit contract.

Returning a pointer into the pointee is ordinary program behavior. Escape checks
protect the authoritative storage containing the source pointer; they do not
forbid the returned pointee address itself.

Representation identities require checked three-byte data-pointer types,
address spaces and allowed cast kinds. Width equality alone is insufficient.
Reuse existing pointer identity and constant-index rules where their complete
predicates apply. Do not broaden arbitrary integer/function-pointer casts or
reuse the looser scalar-width classifier as a representation proof.

Constant index multiplication and displacement composition use checked host
arithmetic. Start with unsigned numeric indices and a complete combined
displacement within `0..65535`; dynamic/signed indices and larger offsets retain
their existing paths. Composition must preserve each link's 24-bit modular
address meaning. Forming a pointee address establishes no alias, object-bound
or memory-reuse fact.

## Target sequence

For a zero-frame `Chain`, the existing native ABI places the incoming pointer
at `$04,S` through `$06,S`. Reuse the exact low-word/high-byte arithmetic
schedule in [long_arithmetic.rs](../src/mir65816/emit/long_arithmetic.rs):

```asm
LDA $04,S
CLC
ADC #$0052
TAY
SEP #$20
LDA $06,S
ADC #$00
REP #$20
AND #$00FF
TAX
TYA
RTL
```

This proposed schedule is **21 bytes and 12 instructions**, reads exactly the
three parameter bytes and writes no DP or stack payload. It preserves carry
into the bank byte, wraps at 24 bits and clears X.high without changing index
width or losing Y.high. Return finishes with A16/X16, the unchanged current-domain
D, DBR=0 and unchanged I.

Final qualification verifies this byte target in all three frozen profiles.
Removing the frame should also remove its guard through the existing
`enter_frame` policy, which already emits no reservation guard for an actual
zero frame. Verify complete frame/staging obligations; do not suppress a guard
merely because some temp homes disappeared. Routines with real locals retain
their frame and result-preserving teardown.

## Slice 0 Baseline and independent fixtures

Freeze the current images, compiler/runtime/fixture inputs and `Chain` bytes,
homes, source spans and resources. Add independent native fixtures for a single
returned field address and the complete field/index/cast pattern; first require
them to pass with the conservative compiler. Use ordinary record definitions
with several offsets, array strides and unrelated names.

An independently assembled caller supplies pointer bytes and checks full A/X
results, X.high, native modes, I/D/DBR, S, source bytes and neighboring canaries.
Give the pointed-to range access observers so any accidental payload read is
detected. Record raw/optimized and guarded/release baselines and unsupported
shapes. Add the tests to the qualified runner without introducing a new harness.

Commit this slice after its focused fixture checks pass.

## Slice 1 Direct address results into Return

Admit a sole constant-offset `AddressOf` with no index, optionally followed by
checked representation-preserving pointer casts, ending in a native three-byte
Return. Integrate ownership into
[home demand](../src/mir65816/emit/home_demand.rs),
[placement](../src/mir65816/emit/placement.rs) and the existing
[address consumers](../src/mir65816/emit/address_consumers.rs).

Preflight the entire source and result schedule before atomically omitting the
producer/alias homes. Recompute mixed placement and complete allocation, then
require frame, spill extent and local peak not to exceed the preceding qualified
plan. A rejected trial restores the whole candidate chain and existing choices.
Final missing sources, broken ownership or stale generations are errors.

Resolve the source's actual final location, evaluate the pure address at Return
with the existing exact A/X arithmetic, and use the existing Return tail.
Selection must not call `self.temp` for an admitted omitted result. Preserve
ordinary memory-backed fallback and closed scalar/DP/pointer allocation profiles
unless their interaction is explicitly verified.

Bind deferred expression, source read, consumer and result lanes into common
selected verification/replay. Recompute the proof from immutable MIR and final
allocation; recorded admission answers and missing map entries are not proofs.
Retain each original operation's identity and attributable empty span; emitted
computation belongs to the consuming Return. Check exact coverage after selected
rewrites and layout. Update placement/emission contracts with this boundary.

Acceptance: independent direct-field wrappers lose their intermediate homes
and frame, while wrappers with real locals retain correct teardown. Native,
allocation, placement and replay checks pass. Commit this slice separately.

## Slice 2 Compose field and constant element address chains

Extend that same plan through consecutive constant `AddressOf` nodes and
qualified pointer identities. Move shared constant-index interpretation into
a typed helper used by planning and selection, rather than folding only after
a home has already been demanded. The zero-index node is a checked identity;
nonzero constant elements contribute their stride-scaled byte displacement.

Retain every logical definition/use while giving the complete expression one
consumer and one source-read contract. Admit or reject the whole chain before
allocation. Do not leave the first field address materialized in DP when its
only purpose is supplying the final returned element address.

Acceptance: the ordinary `Chain` source reaches the proposed 21-byte zero-frame
sequence in raw/optimized and guarded/release builds, with no intermediate
memory homes or DP writes. Nested fields, zero/nonzero literal indices and
different record layouts pass. Dynamic indices, additional uses, incompatible
casts, barriers, escaped pointer-source storage and out-of-range composition
retain complete fallback. Commit this slice after focused qualification.

## Slice 3 Integrated qualification and application measurements

Run the affected backend suites and independent native oracles. Cover low-word
carry, bank `$FF` wrap, null/high-bit pointers, dirty hidden B/X.high, exact
three-byte reads, source-range boundaries, real frames, recursion, fixed and
supported o65 placements. Suspend each reached arithmetic/mode/teardown boundary
with task/IRQ/NMI contexts. Exercise LF/CRLF through any new newline-sensitive
fixture parsing or instrumentation.

Rebuild all three frozen Exec816 profiles against the stage-6 baseline. Publish
the accepted/refused address-return census, actual `Chain` disassembly, complete
application bytes, data/reservations and every per-routine code/resource delta.
Do not predict whole-application savings from this one wrapper or count the
already empty pointer-cast span as a new saving. Preserve all prior call-flow
and pointer/store/argument admissions.

Retain the 441 frozen native vectors and their exact value/access oracles and
existing cycle/stack limits. Run the full affected 65816 suite for final emission
qualification. Measure serial compiler time/RSS against the consistent stage-6
Rust-1.99 baseline, retaining the 5%/10% median limits and all samples.
If layout or artifact-consumer behavior changes, include the affected hosted
fixtures and keep prior failure dispositions explicit.

Reserved bank-zero change must be **0 fixed bytes and 0 bytes per task**.
Require no routine frame/spill/local-peak growth and explain any code growth
before accepting the series. Carry forward the existing application size,
private-traffic and unchecked-provider gates without resetting their targets.
Publish compact evidence under `docs/benchmarks/65816-address-returns/`, update
the boundary documents and commit this final slice separately.

## Validation entry points

During development, select focused units from demand/allocation, placement,
address selection, pointer identities, wide returns and replay. Batch relevant
root integrations and native targets, for example:

```sh
cargo test --locked --features native65816-state-proof --lib mir65816::emit::
cargo test --locked --features native65816-state-proof \
  --test mir65816_emission --test mir65816_state_boundary \
  --test mir65816_address_selection --test mir65816_contract --test mir65816_o65
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test address_consumers --test indexed_addresses --test pointer_values \
  --test pointer_forwarding --test pointer_coalescing --test pointer_preemption \
  --test wide_returns --test home_demand --test stack_allocation --test replay -j2
```

Add the new independent address-return fixture to this batch when it exists.
For final backend qualification, use the full 65816 library/integration/native
commands in the [call-flow plan](MIR65816_CALL_FLOW_IMPLEMENTATION_PLAN.md#validation-entry-points)
and its pinned-vector/serial-measurement procedures. Shared NIR/semantic/verifier
contracts remain outside scope; changing them would require an explicit boundary
update and all shared-contract checks in `AGENTS.md`.
