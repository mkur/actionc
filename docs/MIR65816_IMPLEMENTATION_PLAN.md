# MIR65816 implementation slices

The [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md) and its
[layout manifest](abi/action65816-native-v1.json) define the machine contract.
The [Exec readiness requirements](MIR65816_EXEC_READINESS_REQUIREMENTS.md)
define acceptance. Completing a planning slice does not qualify emitted code.

Implement and commit each slice after its relevant checks. Keep the classic,
small-model and 68k behavior covered. Preserve unrelated working-tree changes.
Update this status table in the same commit as each completed slice.

| Slice | Deliverable | Status |
| --- | --- | --- |
| 0 | Physical ABI specification and this plan | Complete |
| 1 | Generated Rust/assembly constants and typed ABI layout calculations | Complete |
| 2 | Native call plans, result lanes and boundary/state contracts | Complete |
| 3 | Native frame placement, incoming offsets and stack verification | Complete |
| 4 | Minimal native emission and assembly interoperability | Complete |
| 5 | Indirect calls across banks | Complete |
| 6 | First-task entry, return, IRQ/COP save/restore | Complete |
| 7 | Two-context reentrancy and asynchronous qualification | Pending |

## Slice 1: one source for ABI constants

- Generate checked-in Rust constants and assembler equates from the JSON
  manifest, with a deterministic `--check` command.
- Add a target-owned ABI module that classifies verified NIR scalar types,
  computes aligned argument offsets and odd outgoing extents, and specifies
  A/X result lanes and unused-bit requirements.
- Distinguish ADDRESS/SIZE alignment from pointer alignment despite equal
  widths. Reject unsupported physical signatures explicitly.
- Test the published mixed-width example, empty calls, all supported scalar
  classes, width/extent errors, and generated-file freshness.

Completion: compiler and assembly consume matching versioned constants; pure
layout calculations match the ABI examples. No calling sequence is emitted.

Implemented in [the ABI module](../src/mir65816/abi/mod.rs). Regenerate with
`python3 tools/generate_abi65816.py`; check with the same command plus `--check`.
`cargo test --locked --lib mir65816` covers the typed layouts, generated
Rust/assembly agreement (including CRLF input), and existing lowering canaries.

## Slice 2: physical call plans

- Use the ABI calculations for native incoming and outgoing homes, preserving
  the separate small-model policy.
- Record exact result placement, padding/cleanup, direct and indirect transfer
  peaks, and native CPU boundary requirements.
- Record PC, stack memory and domain-memory ownership in the switch contract.
- Keep existing abstract aggregate lowering distinct from v1 qualification;
  do not mislabel transformed aggregate interfaces as qualified v1 exports.
- Exercise real source -> verified raw/optimized NIR -> MIR calls, including
  direct, indirect, recursive, zero-argument and mixed-width signatures.

Completion: both ends of a native call agree on the public layout and its
state obligations. No plan claims final allocated spill or instruction costs.

[ABI pipeline regressions](../tests/mir65816_abi.rs) cover raw/optimized source
lowering, both memory models, exact result homes, transfer peaks and original
aggregate-interface exclusions. The ABI, type-surface, lowering-contract and
aggregate-indirect integration targets passed (61 tests) for this slice.

## Slice 3: frames and stack verification

- Reserve an even fixed native frame; outgoing arguments are per-call areas
  below it. Retain distinct private homes for mutable/addressable parameters.
- Publish incoming displacements and frame addresses relative to body S.
- Verify object alignment/extents, return/argument displacements, and known
  call transients. Supply reusable checked displacement/accounting operations
  for the final allocator/emitter; final spills require another check.
- Reject unsupported native frames, including failures caused by incoming
  arguments or temporary S movement rather than just local-object size.
- Cover the 254-byte limit, exact access boundaries, padding, nested calls and
  unchanged small-model behavior. Complete required NIR/compiler checks.

Completion: the immediate ABI-planning milestone is ready for instruction
selection, with generated constants, concrete plans and regression coverage.

Native frames now contain an even fixed area with per-call outgoing space
below it. Parameter plans expose checked body-relative incoming offsets;
mutable/addressable parameters retain private invocation storage. The
[MIR verifier](../src/mir65816/mod.rs) checks frame/argument alignment, object
extents, caller/callee agreement, return cleanup and required saved state.
[Checked stack operations](../src/mir65816/abi/stack.rs) validate the last byte
of each access, temporary S movement and unsigned reservation bounds.

`minimum_stack_peak` includes the fixed frame and known call costs.
`allocation_complete` remains false: instruction selection must account for
live temporaries, spills, actual accesses and interrupt headroom before
reporting a final bound. The ABI, routine, type-surface and lowering-contract
integration targets passed (60 tests), including rejected oversized frames
and deliberately corrupted plans. Broad validation is recorded below.

## Slice 4: minimal emitted execution

- Add target instruction selection, storage for live temps, encoding,
  relocation/linking and a freestanding image path with explicit diagnostics.
- Start with integer loads/stores, casts, simple arithmetic, branches, direct
  calls and returns; expand the supported scalar widths through real execution.
- Preserve A/X during frame release and caller cleanup. Emit stack checks and
  account for all spills, pushes and IRQ headroom before claiming final bounds.
- Use independent assembly callers/callees and the VM's native 24-bit bus.
  Check exact results, full pointer values, stack balance and guard bytes.

Completion: an Action! binary calls assembly and is called by assembly with
the advertised scalar subset, including no arguments and the ABI example.
Unimplemented source operations fail before a successful image is reported.

The [native emission contract](MIR65816_EMISSION_CONTRACT.md) documents the
implemented scalar subset, the `actionc-65816` driver and image/import format.
The emitter owns invocation slots for temporaries/edge copies, checks final
displacements and emits checked frame/call reservations. Lowering also retains
cast signedness, typed temporary/entry facts and complete control-flow edges.

The [independent execution workspace](../tools/native65816-runtime-tests/README.md)
loads serialized images into the pinned native VM and assembles handwritten
callers/callees with ca65/ld65. It covers scalar boundaries, the ABI example,
recursion, bank crossings, volatile access and pre-write stack faults. The
legacy declaration-address resolver still needs widening; the initial driver
rejects wide numeric bare declaration initializers and supports banked memory
through explicit pointers. Validation is recorded below.

## Slice 5: indirect transfer

- Emit the stack-synthesized far transfer with six-byte transient accounting.
- Exercise target offsets $0000 and $FFFF, multiple code banks, callable
  storage/relocations, and unchanged result/argument conventions.
- Reject illegal branch/PER placement and ABI/signature mismatches.

Completion: direct and indirect calls have identical visible entry/exit state
and independent binary execution evidence.

The emitter now captures full-width callable values and synthesizes the far
transfer on the current stack. Allocated peaks and pre-write checks include
all six transient bytes. PER continuations are checked after placement. The
independent VM corpus covers targets at $050000/$06FFFF, all scalar results,
mixed assembly arguments, scratch clobbers and overflow before transfer writes.

Validation (2026-09-16): all 12 native execution tests passed in debug/release;
the emission, ABI, lowering-contract and CLI targets passed (25 tests). ABI
generation, scoped formatting and whitespace checks passed. Shared NIR and
semantic contracts did not change in this slice.

## Slice 6: assembly context interface

- Generate/import the shared saved-frame and domain offsets.
- Implement entry/restore stubs, a fabricated task image, task return/exit,
  non-switching IRQ dispatch and COP yield against the fixed ABI.
- Verify full A/X/Y/S/D/DBR/P/PC/PBR and domain memory, including noncanonical
  interrupted widths, stack-switch windows and exact return-address bytes.
- Keep NMI bounded and assembly-only; publish platform memory reservations.

Completion: the assembly harness starts and suspends a native context from
the emitted artifact without using the compiler IR as its execution oracle.

The [context interface](MIR65816_CONTEXT_INTERFACE.md) provides checked domain
and first-task construction plus a configurable ca65 bridge. Six executable
context tests cover full register restoration, task/yield/exit paths, invalid
COP/domain use and NMI throughout IRQ-stack/domain transition windows.

Validation (2026-09-16): all 18 native execution tests passed; the six new
context tests also passed in release. Generated constants, scoped formatting,
whitespace and documentation links were checked. No shared NIR/semantic
contract changed. The status-timing corrections remain slice 7 work.

## Slice 7: reentrancy and preemption

- Run two contexts through the same recursive routines and helpers, retaining
  live scratch, automatic arrays, mutable parameters and escaped live locals.
- Resolve the VM's documented REP/SEP/RTI timing limitations before relying on
  exact instruction-boundary IRQ injection. Qualify the declared NMI policy.
- Implement/verify IRQ save/restore barriers, nested tokens, volatile traces,
  bank crossings, checked stack overflow and terminal faults.
- Complete G1–G6 for the advertised kernel subset and record reproducible
  compiler/assembler/VM versions, images, memory maps and interrupt schedules.

Completion: executable readiness evidence permits ordinary Exec implementation.
Scheduling policy, allocation, IPC, drivers and GEM integration remain Exec
work on top of that compiler boundary.

## Validation rules

Run focused tests after each change and the generator's freshness check when
its input/output changes. For changes affecting compiler/NIR contracts, follow
[AGENTS.md](../AGENTS.md): NIR snapshots, the fixture sweep and the full compiler
suite. Do not rerun passing suites without a new change or unresolved concern.

The pre-existing TN sample catalog/standalone-source failures recorded during
the lowering work must be reported separately from regressions. Use an
isolated checkout when unrelated sample edits or untracked programs would
contaminate broad validation; preserve those user changes.

### ABI-planning milestone validation (2026-09-16)

An isolated checkout of the committed slices plus the slice 3 patch ran:

- `cargo test --locked nir_fixtures_match_snapshots`: passed.
- `cargo run --locked --bin actionc-nir-sweep -- fixtures/nir`: all 51 fixtures
  passed loading, semantic analysis, lowering, verification and optimization.
- `cargo test --locked --no-fail-fast`: 3,231 passed, 24 ignored, two existing
  TN sample failures. All targets ran despite those failures.
- `python3 tools/generate_abi65816.py --check`, scoped formatting, whitespace
  checks and relative documentation links: passed.

The existing failures are `sample_catalog_classifies_every_action_source`
(DIR.ACT, LOCATION.ACT, MYDOS.ACT and PANELDIR.ACT lack catalog entries) and
`parses_all_sample_programs` (LOCATION.ACT is analyzed without the DIR_RANGE,
DIR_INVALID, DIR_OK and DIR_END definitions). These occur outside MIR65816;
unrelated sample edits were excluded from this validation.

This milestone completed slices 1–3. These planning tests did not claim emitted
native execution or an Exec acceptance gate.

### Scalar emission milestone validation (2026-09-16)

An isolated checkout of slice 3 plus the slice 4 changes excluded unrelated
sample edits and untracked programs. It ran:

- `cargo test --locked nir_fixtures_match_snapshots`: passed.
- `cargo run --locked --bin actionc-nir-sweep -- fixtures/nir`: all 51 fixtures
  passed loading, semantic analysis, lowering, verification and optimization.
- `cargo test --locked --no-fail-fast`: all targets ran. The new scalar example
  required updating the broad corpus's expected successful fixture count from
  361 to 362; `cargo test --locked --test nir_corpus` then passed. Across the
  full suite and this affected-target rerun, 3,239 tests passed, 24 were ignored
  and only the two existing TN failures described above remain.
- The separate `tools/native65816-runtime-tests` workspace: all nine tests
  passed in debug and release, including 72 arithmetic executions and
  handwritten assembly interoperability in both optimization modes/I states.
- The documented public CLI example produced a valid image with three routines.
  ABI generation, scoped formatting, whitespace and relative documentation-link
  checks passed.

The VM is pinned to `56ddc5c5de41f0e7294e87c440869550eaf53292`; the local toolchain
was Rust 1.95.0 and ca65/ld65 2.18 on macOS ARM64. The emission contract and
execution workspace document the supported subset and platform obligations.

Slice 4 is complete. Slice 5 adds indirect calls across banks. Context switching,
asynchronous execution and complete G1–G6 acceptance remain pending.
