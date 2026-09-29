# Resident pointers in bounded 65816 leaf routines

Status: slice 1 implemented; slices 2 and 3 pending.

## Goal and baseline

Extend the existing pointer-leaf allocator so private pointer values remain in
the three existing DP slots throughout a small leaf routine. Reuse NIR storage
promotion and MIR allocation rather than adding a separate local allocator.

The motivating example is Exec816 `AddHead`: `chain`, `item`, and the captured
`first = chain.lh_Head` fit in `$80..$82`, `$83..$85`, and `$86..$88`. The
zero-offset address `@chain.lh_Head`, including its pointer cast, carries the
same 24-bit value as `chain` and needs no additional slot or copy.

| Measurement | Current compiler (`50c89af5`) | Target |
| --- | ---: | ---: |
| Complete AddHead code | 146 bytes | At most 96 bytes |
| Fixed frame / local stack peak | 4 / 4 bytes | 0 / 0 bytes |
| Temporary spill storage | 0 bytes | 0 bytes |
| Incoming pointer captures | Repeated staging | One capture per parameter |

The [current listing](benchmarks/65816-local-loads/addhead.asm) is measured
compiler output. The proposed assembly has been assembled to confirm 96 bytes;
it still needs execution qualification. Measure cycles under the same native
harness; do not infer a cycle count from code size. The size and zero-frame
target apply to optimized NIR. Raw NIR must remain correct and retain its
existing conservative fallback where private locals are not promoted.

## Design

There are two independent blockers today:

- `src/nir/promotion.rs` admits bounded pointer-home promotion only when every
  operation is a load or store. Address formation and casts prevent removal of
  `first` and repeated parameter loads.
- `src/mir65816/emit/allocation.rs::leaf_intervals` has a matching load/store
  whitelist and requires pointer-typed temporaries. Address/cast identities
  therefore prevent DP allocation even after local promotion.

Extend those existing paths in that order of responsibility: NIR proves that
storage can become values; MIR proves which values can share physical DP homes.
Do not infer identities from source names or reclassify ordinary pointee memory
as private storage.

Keep the current bounded scope: one block, no block parameters, at most 64
operations, no calls, and a procedure return. Add only identities whose complete
24-bit representation is unchanged:

- A verified representation-preserving cast between data pointers, or between a
  data pointer and ADDRESS.
- Taking an address through an existing pointer with zero displacement and no
  index. Taking the address of a local or parameter is a different operation
  and must retain the existing escape/addressability rules.

Equal width alone is insufficient proof. Constants, nonzero address arithmetic,
other integer calculations, volatile operations, dynamic indexes, calls,
machine code and control flow retain existing selection. Ordinary pointer-field
loads/stores still support their existing bounded nonzero displacements; this
restriction concerns computing a new address value.

Use typed value identities in the allocation plan. Several SSA temporaries may
name one immutable pointer value and share its slot even when their uses
overlap. Compute the lifetime of the complete identity group, including uses as
both address and stored data. Distinct values retain closed operation lifetimes:
the source base stays intact until the bank byte has been read. The existing
dying-base reload exception must consider every alias's last use before reusing
its slot.

Continue to allocate from the three ABI pointer slots. When the complete plan
does not fit, use the whole-routine stack strategy; add no partial spilling,
extra DP reservations or values surviving ordinary calls in this work.

## Slice 1: admit and coalesce pointer identities in DP allocation

Primary files: `src/mir65816/emit/allocation.rs`,
`src/mir65816/emit/pointer_values.rs`, and the affected selection/liveness tests.

- Classify eligible address/cast operations using verified MIR types, widths,
  cast semantics and address geometry. Record their source identities without
  changing logical temporary IDs or types.
- Build deterministic identity groups and allocate group lifetimes to the
  existing slots. Preserve the current result for already-admitted routines.
- Make selection consume coalesced identities without copying or materializing
  them. In particular, a zero-offset address using the same DP home must finish
  successfully rather than falling into generic address construction and
  clobbering resident scratch.
- Recompute the identity and lifetime proof in the allocation verifier. Reject
  unrelated overlapping values, forged alias bindings, stale reload exceptions,
  wrong widths and scratch outside the three slots. Keep all materialized
  temporary identities truthfully represented in existing physical maps.
- Preserve the closed DP plan's ownership over home-demand decisions. Borrowed
  stack bindings, direct local loads and expression consumers must not also
  claim an operation allocated by this plan.

Completion: leaf routines with casts and zero-offset address identities use DP
without extra slots or identity copies. Execute a focused emitted-code case;
cover cast chains, later uses of the original value, alias-aware dying-base
reloads, three-slot capacity and four-distinct-value fallback. Commit the slice.

## Slice 2: promote private pointer locals and incoming values

Primary files: `src/nir/promotion.rs`, existing home-elision/cleanup code,
`tests/nir_native_promotion.rs`, and the native pointer/address tests.

- Broaden only the `Native65816` bounded pointer profitability policy to permit
  the proved identity operations from slice 1. Keep storage legality in
  `is_promotable()` and `is_proven_private_to_invocation()`; preserve definite
  initialization, escape, addressability, volatility and initializer checks.
- Use existing promotion to capture eligible immutable pointer parameters once
  and replace private local stores/reloads with typed values. Remove obsolete
  local homes through existing cleanup/home elision.
- Preserve each indirect read, its complete snapshot, and public store order.
  `first` must be loaded in full before any list mutation. Retaining a pointer
  value never permits forwarding a pointee load across a write.
- Let final allocation recalculate frames, incoming argument displacements and
  stack peaks. With no frame and no local stack use, the existing empty-frame
  entry/return policy removes the guard and cleanup automatically.

Completion: optimized AddHead holds its three distinct values in DP, loads
`first` directly into its assigned slot, and meets the zero-frame/96-byte target.
Add renamed/retyped record examples and different pointer-field displacements
to prove general applicability. Check escaping locals, partial accesses,
parameter mutation and unsupported operations still receive safe handling.
Raw NIR and other targets retain their existing promotion policies. Commit the
slice with focused NIR and emitted-code regressions.

## Slice 3: qualify and measure the combined change

- Reuse `pointer_allocation`, `pointer_reload`, `address_consumers`,
  `pointer_preemption` and `stack_checks` coverage. Compare generated AddHead
  with an independent assembly/reference oracle in raw and optimized modes.
- Check exact three-byte external read/write extents and order, distinct banks,
  `$xxFFFF` crossings, aliased/overlapping nodes and the empty-list sentinel.
  Keep private overlapping-word argument captures separate from external
  accesses. Exercise fixed and relocated code placements.
- Check bounded completion, stack balance, M/X return state, D/DBR and I
  preservation, DP bounds, truthful frame maps and interrupt/task suspension
  while all three slots are live. Extend the existing context harness rather
  than inventing a new interrupt protocol.
- Preserve existing pointer-leaf and stack-fallback code-quality coverage.
  Measure routine-entry-through-RTL bytes and cycles against the reference;
  investigate a missed size target instead of silently relaxing it.
- Compile the same frozen Exec input set used by the
  [local-load measurement](benchmarks/65816-local-loads/README.md) once after
  qualification. Record compiler/input hashes, per-routine code/frame changes,
  full routine-code totals and map validation. Explain any growth separately.
- Update the [emission contract](MIR65816_EMISSION_CONTRACT.md) and retain the
  existing ABI/image format. Record results alongside the reproducible listing
  and measurements, then commit the slice.

## Validation budget and integration

During each slice run focused tests for its new behavior and its fallbacks.
At integration, run the affected MIR65816 backend suite and native qualification
through `tools/native65816-runtime-tests/qualify.py`, including the required
modes and context checks. Slice 2 changes shared NIR code, so the repository's
shared-contract checks also apply: `cargo test nir_fixtures_match_snapshots`,
`cargo run --bin actionc-nir-sweep -- fixtures/nir`, and `cargo test`. Batch these
after the implementation is stable; reuse passing results unless later code
changes or failures justify another run. Verify LF/CRLF handling if adding
newline-sensitive source or assembly fixtures.

This is compiler work in actionc. Exec source changes, compiler pin movement
and play-image refresh are separate integration actions. Record any local
compiler override used for measurements; compiler qualification alone does
not qualify the hosted Exec system.

Reserved bank-zero delta for every slice: **0 fixed bytes and 0 bytes per task**,
including guards, alignment and unused reserved capacity. The nine DP bytes
already belong to the compiler's 64-byte scratch region. Task-stack reservations
and interrupt headroom remain unchanged even when individual calls use less
stack.

## Slice 1 result

Typed pointer/ADDRESS identities now share DP homes and group lifetimes. The
verifier independently rebuilds those identities, including the last use of
all aliases before dying-base reuse. Identity casts and zero-offset address
formation emit no copy; closed pointer allocation owns its complete demand
plan. Existing stack fallback and incoming geometry checks remain in force.

Validation: 15 pointer-value unit tests, 27 emission integration tests and six
native cases (`pointer_allocation` and `pointer_reload`) pass. The native cases
cover raw/optimized execution, all address bits, exact access traces, relocated
code and IRQ/NMI/task reentry. Two older unit expectations were updated because
same-home identities now emit no instructions. Reserved bank-zero delta is
0 fixed bytes / 0 bytes per task.
