# MIR6502 Address-Consumer Materialization Plan

Snapshot date: 2026-06-02.

This note is a focused Codex-ready plan for improving MIR6502 materialization
when a word value is consumed as an address. It is downstream of the MIR6502
machine contract, object-emission plan, and tracked-emission boundary.

Related documents:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/MIR6502_IMPLEMENTATION_PLAN.md`
- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_FULL_LANGUAGE_EXPANSION_PLAN.md`

## Problem

Pointer dereference and later pointer-backed indexing need an address in a
zero-page pointer pair. The current path can materialize an address-valued word
temp as an ordinary spill, then reload that spill into the zero-page pair.

That creates this inefficient shape:

```text
address storage -> ordinary word temp or spill -> zero-page pointer pair -> indirect access
```

The desired shape is:

```text
address storage -> zero-page pointer pair -> indirect access
```

This is not a peephole. It is target materialization infrastructure. Address
consumers need an address home, not an ordinary word-value home.

## North Star

When a MIR value is consumed as an address, materialization should route it
directly to the required address home.

The first address home is a fixed zero-page pointer pair used for indirect
indexed access. General zero-page allocation can come later.

## Red Lines

Do not include broad optimization in this slice.

Out of scope:

- memory constant propagation;
- rewriting a known pointer value into direct absolute memory access;
- general register allocation;
- general zero-page allocation;
- broad copy propagation;
- target peepholes;
- branch layout optimization.

Tracked emission must not recover pointer meaning, source meaning, or storage
meaning. MIR materialization must decide the address staging path before
emission.

## Milestone 1: Define Address Consumers

Goal: represent why an address is needed.

Add a small abstraction equivalent to:

```text
IndirectY address consumer
Absolute address value consumer
ABI word consumer
Store-word consumer
```

For the first implementation, support only the indirect-indexed address consumer.

Acceptance criteria:

- Pointer dereference materialization can request an address in a zero-page
  pointer pair.
- Existing scalar materialization remains unchanged.
- Unsupported consumers are rejected or deferred with clear diagnostics.

Suggested commit:

```text
mir6502: define address consumer materialization
```

## Milestone 2: Add Zero-Page Pointer Pair Homes

Goal: name the pointer pair used for indirect access without scattering concrete
addresses through materialization.

Add a pair abstraction with two classes:

```text
fixed zero-page pair
virtual zero-page pair
```

For the first implementation, use a fixed pair matching the current backend
scratch convention.

Rules:

- Do not implement general zero-page allocation here.
- Keep fixed pairs separate from virtual pairs.
- Print the selected pair clearly in MIR.

Acceptance criteria:

- Address staging can target a named pair.
- The pre-emission verifier treats fixed pairs as concrete enough.
- Virtual pairs remain deferred to the zero-page allocation milestone.

Suggested commit:

```text
mir6502: add zero-page pointer pair homes
```

## Milestone 3: Add Address Staging And Indirect Access Forms

Goal: make the address staging dependency explicit in MIR.

Add MIR forms equivalent to:

```text
stage address to zero-page pair
load indirect through zero-page pair
store indirect through zero-page pair
```

These may be dedicated MIR ops or structured forms using existing load/store
address variants. The staging operation should still be visible and verifiable.

Acceptance criteria:

- MIR can represent staging an address into a pair.
- MIR can represent byte and word indirect loads/stores through the staged pair.
- The printer shows staging and indirect access clearly.
- The verifier rejects obvious uses of unstaged or unknown pairs where it can
  prove them.

Suggested commit:

```text
mir6502: add address staging and indirect access ops
```

## Milestone 4: Implement Address-To-Pair Materialization

Goal: add the helper that routes address values directly into a pair.

Add a materialization helper equivalent to:

```text
materialize_address_to_zp(value, pair)
```

Initial supported address sources:

```text
constant word address
word temp address
explicit low/high word value
static address
global address
routine address
```

For the first useful version, support the common pattern where a word load is
used only as the address for a following indirect access. Materialize the loaded
address directly into the fixed pair instead of first assigning it an ordinary
spill home.

Acceptance criteria:

- Address-valued temps can be staged to the fixed pair without ordinary memory
  spills.
- Pointer dereference materialization uses this helper.
- Unsupported address sources produce targeted diagnostics or remain in a form
  rejected before emission.

Suggested commit:

```text
mir6502: materialize address values to zero page
```

## Milestone 5: Fuse Producer Loads Into Address Staging

Goal: remove the unnecessary spill in the common local pattern:

```text
word load produces temp
following indirect access consumes that temp as address
```

If the temp is used only by the address consumer, materialize the producer load
directly into the fixed pair.

Rules:

- This is not general copy propagation.
- This is not memory constant propagation.
- This is a narrow materialization rule.
- Be conservative around calls, barriers, machine blocks, and unknown memory
  effects.

Acceptance criteria:

- Post-materialization MIR for pointer dereference does not contain an ordinary
  word-temp spill between loading the pointer value and staging the indirect
  address.
- Existing scalar fixtures remain green.

Suggested commit:

```text
mir6502: stage loaded pointer addresses without spills
```

## Milestone 6: Emit Staged Indirect Loads And Stores

Goal: emit the staged-indirect MIR forms through tracked emission.

Support:

```text
stage address to fixed pair
byte indirect load
word indirect load
byte indirect store
word indirect store
```

Rules:

- Emission receives the already-selected pair and offset facts.
- Emission must not infer pointer meaning.
- Ordinary instructions still route through tracked emission helpers.
- Staging into the fixed pair must update or invalidate tracked state
  conservatively.

Acceptance criteria:

- Byte pointer dereference stores emit through staged indirect access.
- Word pointer dereference stores emit two byte stores through staged indirect
  access.
- No source-name or SemIR recovery occurs in emission.

Suggested commit:

```text
mir6502: emit staged indirect loads and stores
```

## Milestone 7: Pointer Dereference Fixtures

Goal: lock in object-code behavior for address-consumer materialization.

Add focused fixtures for:

```text
byte pointer dereference store
word pointer dereference store
address-of scalar assigned to pointer followed by word dereference store
address-of scalar assigned to pointer followed by word dereference load
```

Acceptance criteria:

- Object code is generated for all fixtures.
- No unnecessary ordinary word-temp spill appears between pointer load and
  zero-page staging.
- Output is correct even without constant propagation to direct absolute stores.
- Existing scalar fixtures remain green.

Suggested commit:

```text
mir6502: add pointer dereference materialization fixtures
```

## Suggested First Codex Task

```text
Implement address-consumer materialization for pointer dereference.

Goal:
- Add an explicit address-staging path for values consumed as indirect addresses.
- Add a fixed zero-page pointer-pair home for the first implementation.
- Materialize a loaded pointer value used by a following indirect access by
  staging the source directly into the pointer pair, not by spilling the word temp
  to ordinary memory first.
- Add MIR/object fixtures for byte-pointer and word-pointer dereference stores.
- Do not implement memory constant propagation.
- Do not add general zero-page allocation.
- Do not add peepholes.

Acceptance:
- Pointer dereference emits object code without a temporary spill between loading
  the pointer value and staging the indirect address.
- Existing scalar fixtures remain green.

Required checks:
- cargo test
- cargo test mir6502_fixtures_match_snapshots
- cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502, if available

Suggested commit message:
- mir6502: materialize deref addresses directly to zero page
```
