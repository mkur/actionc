# MIR68K descriptor alignment and guarded memory accesses

Status: implementation in progress. Slice 1 is complete and validated; slices
2–4 follow below. Commit after each completed slice.

## Objective and baseline

Reduce the cost of reading and writing arrays through mutable descriptors while
preserving Action!'s unaligned accesses, evaluation order and rebinding behavior.
Keep the original MC68000, native ABI, r68k VM and existing Amiga HUNK path.

Baseline compiler: `38471f2`, after the four
[index arithmetic optimization slices](INDEX_ARITHMETIC_OPTIMIZATION.md).
Default machine options and optimized NIR give:

| Workload | Instructions | Executable bytes | Largest frame |
| --- | ---: | ---: | ---: |
| matrix1, flat/pointer | 71806 | 990 | 68 |
| matrix1, shaped | 144051 | 1332 | 88 |
| jfdctint, flat/pointer | 38767 | 5768 | 742 |
| jfdctint, shaped | 43279 | 6658 | 886 |

These are uninstrumented default workloads, not CPU-cycle measurements. Current
MIR for matrix1's `Multiply` proves alignment of the flat variant's element
pointers. Shaped element accesses lack those proofs and use bytewise longword
loads/stores. The descriptor cells themselves can be aligned while their
contents point to odd memory.

Capture a fresh baseline CSV, resolved options, source hashes and the relevant
MIR/machine listings before implementing slice 1. Preserve existing historical
reports; keep per-slice artifacts under `build/mir68k-descriptor-alignment/`.

## Boundaries and invariants

- SemIR retains source meaning, widths, lvalues and array/routine disambiguation.
- NIR owns alignment facts about captured values and mutable storage at a
  particular program point. Reuse storage IDs, regions, effects, CFG and
  dataflow; add no source strings or executable shape metadata.
- MIR68K owns access selection and runtime alignment checks. Emission owns
  instruction encoding, physical branches, relocations and listings.
- Shared analysis remains valid for all target layouts. Only MIR68K emits the
  new guarded path; classic 6502 remains a correctness baseline. Neither a
  65816 VM nor a new public artifact version is part of this milestone.
- A global descriptor initializer is not an entry-time promise. Existing native
  tests overwrite descriptors before calling the program. No whole-program
  absence-of-rebinding shortcut may erase that supported behavior.
- A fact about a pointer slot and a fact about its pointee are distinct. Track
  the pointer-byte region using the actual target pointer width; distinguish
  it from the adjacent descriptor size word, inline elements and backing data.
- Preserve all source arithmetic and destination/base/index/RHS capture order.
  Never reevaluate an index, reload a captured descriptor, or speculate a data
  access just to establish alignment.
- On this target, both words and longwords require even addresses. Test the
  complete effective address, including displacement, field offset and index.
- Volatile accesses retain their existing observable access sequence. Do not
  widen or merge them using the new descriptor facts or runtime guards.

## Slice 1: Track proven descriptor pointer values in NIR

Extend the existing even/unknown analysis in `src/nir/analysis/alignment.rs`.
Its current tracked homes are private scalar cells; actual descriptor pointer
slots need explicit eligibility and mutation handling.

1. Identify descriptor slots from verified storage/backing facts, including
   invocation-local descriptors. Do not recognize them from names, printed
   types, or a guessed object size. Keep slot extent separate from backing
   extent and descriptor count metadata.
2. Start external/global/routine-static descriptor contents as unknown at each
   routine entry. Establish an even fact after a dominating runtime assignment
   from a proven-even captured value. Automatic descriptor initialization may
   establish a fact only where the NIR contract guarantees that initialization
   runs for this invocation and the actual backing layout proves even alignment.
   Unknown incoming parameters and results remain unknown.
3. Transfer facts in operation order. A descriptor load captures the fact valid
   at that load; a subsequent rebind invalidates the cell fact, not the already
   captured SSA value. A later assignment must never justify an earlier load.
4. Replace a slot fact on exact full-pointer stores. Invalidate it on overlapping
   or partial writes, volatile mutation, unknown indirect writes, and calls or
   machine effects that may touch it. Preserve it only when existing structured
   privacy/region/effect proofs establish disjointness. Declared array bounds
   alone do not prove an unchecked dynamic store disjoint from a descriptor.
5. Intersect facts at reachable joins. A loop needs a grounded entry fact and
   preservation on every backedge; an unvisited edge is not proof. Include
   parallel edges with different arguments and recursion/local initialization.
6. Continue issuing opaque alignment receipts for captured values. Recompute
   after NIR mutation; MIR verification must reject missing, mismatched or stale
   derivations. Keep volatile access eligibility explicit when lowering so the
   stronger analysis cannot silently alter its access sequence.

Reuse `analysis/storage.rs` and applicable exact-region/effect queries from
`analysis/aggregate_regions.rs`. Unknown alias cases lose precision; this slice
must not grow into a general interprocedural alias analysis. In particular,
shaped matrix1's global descriptor loads may still be unknown after this slice.
That is an expected input to slice 2, not a reason to assume immutable backing.

Primary consumers: `src/mir68k/{lower,verify}.rs`. Update
[NIR_TARGET_SHAPE.md](NIR_TARGET_SHAPE.md) and
[MIR68K_EXECUTION_CONTRACT.md](MIR68K_EXECUTION_CONTRACT.md) with the resulting
proof contract, rather than implementation history.

Acceptance:

- Explicit even assignments and safe automatic initialization yield usable
  proofs; odd/unknown rebinds and unsafe joins lose them.
- Pre-entry descriptor replacement, a load before a later even store, aliasing
  partial writes and unknown call effects cannot produce false proofs.
- Stronger static facts select native accesses in focused nonvolatile kernels;
  existing volatile traces and mutable-descriptor tests remain unchanged.

## Slice 2: Guard profitable unknown-alignment accesses on MIR68K

Implement a bounded per-access fast path after final address formation. Start
with nonvolatile longword loads/stores through unknown indirect addresses.
Byte operations need no check. Keep unknown word accesses on the existing path
unless measurement demonstrates that guarding them is worthwhile.

The selection order is: existing statically proven access; eligible guarded
access; existing bytewise fallback. Do not guard copies, real operations or
volatile operations in this slice. Preserve conservative handling of explicit
absolute/device accesses; an alignment check is not a memory-validity check.

For one eligible operation:

1. Compute its final address once, using the existing full-width arithmetic.
2. Test bit zero in a scratch data register; do not read guest memory to test
   alignment and do not use a later-CPU instruction form.
3. On the even path, perform one native longword load/store. On the odd path,
   execute the existing bytewise sequence at the same address.
4. Join with identical result, normalization and scratch-register contracts.
   Preserve D0's pending store value while forming/testing the address. Account
   for flags and temporary-forwarding state at every physical branch/join.

Use the existing typed machine-block/label machinery and final branch relaxation.
The local guard is a runtime condition, not a forged static alignment receipt.
Keep it local to that access: no loop cloning, speculative backing accesses,
hoisted descriptor checks or address-register allocation in this milestone.

Add a separate `guarded_memory` materialization option and matching developer
switches `--guarded-memory` / `--no-guarded-memory` in both measurement runners.
Default it off for this slice. `--no-codegen-opt` must disable it. Preserve the
meaning of `--no-pointer-alignment` as the existing static-proof selection
switch; report both settings explicitly and reject conflicting guard switches.
No new public language syntax or compiler CLI flag is needed.

Primary code: `src/mir68k/{materialize,machine,encode,temporary_forwarding}.rs`
and a small memory-selection helper if needed. Extend `verify.rs` only for
structured contract changes; retain the existing ABI and scratch register pool.

Acceptance:

- Literal MC68000 qualification or existing qualified encodings support the
  exact guard sequence. No odd path executes an unaligned word/long instruction.
- Aligned unknown-pointer longword kernels execute fewer instructions than the
  bytewise baseline; statically aligned kernels acquire no redundant guards.
- Odd accesses, store values, calls and completion behavior match with guards
  off/on, including register allocation and forwarding off/on in focused cases.
- Faulting paths do not resume, and guards make no additional data reads/writes.
  Where the VM exposes partial effects at an inaccessible boundary, check those
  too; do not claim exact CPU bus-cycle equivalence from a byte trace.

## Slice 3: Complete regression coverage and choose defaults

Add focused cases alongside `pointer_alignment.rs`, `multidimensional.rs` and
`index_arithmetic.rs` in `tools/vm68k-runtime-tests/tests/`. Share the existing
compiler/VM adapters and symbol metadata; add no fixed-address benchmark data.

Cover the following independently rather than taking their entire Cartesian
product:

- Even/odd base pointers; even/odd field offsets; odd record strides; negative
  coordinates and byte/word/long source indices; offsets beyond 64 KB.
- Descriptor replacement before entry, explicit rebinding within a loop, and
  rebinding during an RHS call after destination capture. Check both the old
  destination and the next iteration's new base.
- Exact, partial, aliased and unknown writes to descriptor slots; disjoint
  writes to backing data and to the adjacent size word where provable.
- Zero-trip and multi-iteration loops; mixed joins and parallel edges; local
  descriptors in recursive invocations; source width wrap boundaries.
- BYTE, CARD/INT and LONGCARD/LONGINT reads/writes; volatile wide accesses,
  overlap-safe copies, mixed aligned/odd neighbors and exact volatile traces.
- Bare images and relocated HUNK execution at multiple bases; ABI canaries,
  stack preservation, fault exits, guard branches and register pressure.

Run the full shaped/flat matrix1 and DCT reference suites on both VM workspaces:
252 matrix cases and 181 DCT cases, with their existing type/mode coverage.
Require unchanged full states, not just the documented checksums. Also measure
all existing native benchmark families to catch code-size and stack regressions.

Compare baseline, static proofs only, guards only, both enabled and conservative
materialization. Raw/optimized NIR and target option settings must remain
separate in records. On deliberately odd pointers, the extra guard is expected
overhead; report it explicitly. Favor the longword-only guard unless wider
eligibility earns a measured benefit without material corpus regressions.

Enable the profitable policy by default only after these checks. Require a
clear improvement over the shaped matrix1 baseline and no unexplained DCT
regression. Report instructions, executable bytes, maximum frames and stack
traffic; do not promise GCC parity or prescribe an unmeasured percentage gain.

## Slice 4: Refresh and extend the paired MC68000 GCC comparison

Keep the existing insertion-sort and flat matrix1 C comparisons as controls.
Add explicit variant mapping so the runner can pair both flat and shaped
Action! implementations with corresponding C sources without assuming every
benchmark has a `kernel.inc` file.

- Add a shaped matrix1 C adaptation retaining row/column order, 16-bit counters,
  32-bit elements, initialization, volatile input and checksum behavior. Model
  mutable descriptor-backed arrays with pointer-to-row values and separate
  backing objects. Adapt the C symbol view accordingly; do not mistake a pointer
  symbol's storage address for its element backing.
- Add flat and shaped DCT C adaptations preserving operation order, constants,
  initialization, both passes and Descale semantics. Use the pinned oracle's
  defined 32-bit wrapping/sign-preserving shift approach. `-fwrapv` alone does
  not define signed left shifts; avoid relying on those or overflowing signed
  shifts in the C mirror. Retain the upstream attribution and permission text.
- Keep original reference C and vector files unchanged. Validate every selected
  matrix1 LONGINT/10x10x10 case (22) and all 181 DCT cases for Action! and both
  GCC modes. Keep insertion sort's 209 cases. Compare complete input/output
  states, row-pass DCT snapshots, rounding cases, checksums and statuses.
- Use instrumented builds for intermediate-state correctness, and separately
  use uninstrumented default builds for headline code-quality measurements.
  Label reference-corpus instruction totals when capture instrumentation is
  included. Do not let snapshot calls distort the primary performance claim.
- Keep current original-MC68000 GCC flags, `-O2`/`-Os`, ABI choices, helper
  accounting, no LTO and r68k execution conventions fixed. Record actual
  toolchain versions, selected libgcc, source hashes and commands. C correctness
  comparisons use valid aligned C objects; Action!'s odd-address semantics stay
  covered by the Action! differential tests, not undefined C dereferences.

Primary files: `tools/compare_mir68k_c.py`, `tools/mir68k-c-reference/`,
`tools/vm68k-runtime-tests/examples/{c_reference.rs,c_reference/,measurement/}`.
The GCC toolchain remains an explicit developer dependency, not a prerequisite
for ordinary `cargo test`.

Acceptance: every comparison row is emitted only after its reference checks
pass. Publish a new CSV and report linked from
[MIR68K_C_COMPARISON.md](MIR68K_C_COMPARISON.md); keep prior baselines intact.
Report remaining costs and whether guard overhead or missing static proofs is
still significant. Address-register allocation/postincrement stays later work.

## Validation and completion

For shared NIR/proof changes, run the repository-required checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Use focused native tests for slice 2; run both full benchmark targets and the
broader native suite when selecting defaults in slice 3. Slice 4 needs the paired
GCC command, its actual LF/CRLF manifest/instrumentation tests, and affected
reference targets. Do not rerun the whole compiler suite for reporting-only
changes. Normalize host text before newline-sensitive instrumentation; preserve
binary and guest bytes. Reuse isolated VMs and compile once per configuration.

If unrelated untracked samples interfere with a broad scan, run the exact sample
check against tracked sources with the current compiler library and disclose the
scope. Do not alter that work or share Cargo output directories across checkouts.

For each slice, review the diff, record relevant validation and measurements,
update the contract/report as needed, and commit the completed slice. Keep work
on the current branch; no extra branches or CI waits are required. The milestone
is complete when all four slices pass their gates, the chosen defaults and
fallback policy are documented, and final measurements are reproducible.
