# LONGINT/LONGCARD integration and MIR6502 implementation

Status: complete (2026-09-07). Each major slice is a separately verified commit.

## Contract

- LONGINT is signed two's-complement 32-bit; LONGCARD is unsigned 32-bit on
  every target. Contextual SYS aliases use these names; LONG/ULONG are not
  built-in synonyms. Existing identifiers with any of these names stay legal.
- INT/CARD remain 16-bit on every target. Expression typing follows operands,
  not the assignment destination: widening before an operation differs from
  widening its already-computed narrow result. Wide literals keep their
  explicit inferred wide type; existing literals keep their old rules.
- Preserve modern division/remainder, wrapping and arithmetic-fault semantics,
  enum nominal identity, single-evaluation CASE, embedded arrays, and safe
  call-return facts. No source meaning is reconstructed in machine emission.
- Compiler constant containers may be wider than target values. Narrow CASE
  selectors and calculations do not acquire wide runtime operations.
- Native 68k/65816 validation remains lowering/ABI canaries, not executable
  validation. MIR6502 execution must be tested with cart and standalone.
- This follow-up covers integer conversions and integer code generation;
  direct REAL/32-bit conversions and 32-bit formatted library I/O are separate
  follow-ups. Their existing narrow interfaces are not silently widened.

## Slices

1. Rename the native branch's source types, diagnostics, fixtures and docs;
   retain its width-aware NIR representation. Verify and commit on that branch.
2. Integrate on a branch based on current main, preserving both histories.
   Combine general function/callback result types with enums; wide layout facts
   with embedded-array extents; and wide constant evaluation with modern signed
   arithmetic. Widen CASE label facts through shared typed constants. Verify
   the combined compiler and VM baselines before merging into main.
3. Add MIR6502 wide storage, conversions, direct/indirect calls and returns,
   and basic arithmetic/comparisons using target-owned byte/word legalization.
   Specify argument/result homes; preserve ordinary 8/16-bit paths. Keep any
   not-yet-supported wide operation explicitly diagnosed during rollout.
4. Complete wide shifts, multiplication, division and remainder, including
   signed boundaries, zero faults and mixed-width operands. Reuse general
   lowering/helper contracts; do not specialize for individual samples.
5. Exercise wide CASE, globals/locals, pointers, arrays, record fields,
   prototypes, loops and side effects end-to-end. Document the public surface
   and backend differences and mark complete only after final acceptance.

## Checks

For semantic/IR/codegen slices run the root checks required by AGENTS.md:

```
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Run focused and full VM tests from tools/vm-runtime-tests with --locked.
Use independent host oracles and externally supplied values. Include 16-bit
wrap-before-widen versus explicit wide computation, carry/borrow across all
four bytes, signed/unsigned boundaries, effectful nested calls, pointer/index
aliasing, neighboring-byte guards, and labels differing only above bit 15.
Existing snapshot changes must be explained; no blanket snapshot acceptance.

## Progress

- Slice 1: complete. The 16 focused native type tests, root compiler suite,
  NIR snapshots and all 33 sweep fixtures pass. Existing snapshots are unchanged.
- Slice 2: integrated and verified. Preserve canonical enum signatures and
  record/embedded-array extents; share resolved scalar/enum cast facts, including
  SYS-qualified casts. Constant arithmetic and NIR folding retain signed
  division/remainder, sign extension and target-sized SIZE/ADDRESS masks. CASE
  labels use wide payloads with selector-width comparisons. Dynamic narrow
  multiplication remains 16-bit even when assigned to a wide destination.
  All 2,828 compiler tests, snapshots, 37 NIR sweep fixtures and 118 VM tests
  pass. The only additional test-contract changes are SIZE-role query constants
  and native record extents no longer artificially limited to 16 bits. Enum
  snapshot spelling and callable signature identities remain unchanged.
- Slice 3: wide storage, casts, add/subtract, negation, bitwise operations,
  comparisons, calls and returns are legalized into ordinary word lanes.
  Runtime coverage includes externally supplied signed/unsigned boundaries,
  wrap-before-widen, nested direct/typed-indirect calls, and embedded arrays.
  Callback lowering now distinguishes callable values from array/data
  pointers; the indirect-call trampoline preserves the first argument in A.
- Slice 4: wide shifts, multiplication, signed/unsigned division and remainder
  use compiler-owned kernels through the existing structured helper contracts.
  Runtime tests cover boundary grids, compositions, mixed operands, compound
  assignments, wrap-before-widen multiplication, and Error(100) with a returning
  handler. Baseline and optimized MIR discover mandatory legalization helpers
  independently of optional helper-selection rewrites.
- Slice 5: complete. Wide CASE preserves all
  label bits, signed range order, and one selector evaluation. FOR step facts
  preserve typed constants before induction conversion; wrap thresholds use
  the induction width. Absolute variable addresses are not constant bounds.
  Tests cover numeric limits, large steps, dynamic array/pointer indexing,
  aggregate guards, initializers and pointer-cell overlap. Classic compiler and
  inspection CLI entry points reject wide integer codegen with an explicit
  MIR6502 diagnostic.

Final acceptance: all 2,831 compiler tests and 131 VM tests pass, including the
13 wide-integer/related regression tests. NIR snapshots are unchanged and all
37 sweep fixtures pass. Baseline, optimized and disabled-peephole/helper-selection
configurations verify mandatory wide legalization. Native target type/ABI
canaries remain green. User-owned unrelated artifacts were not included.

## MIR6502 legalization and ABI

A 32-bit NIR temp/block parameter is represented by low and high word temps.
Edges carry both lanes. Loads/stores capture an indirect effective address once
and access offsets 0 and 2; memory remains little-endian. Carry/borrow between
word operations is an explicit value, not an implicit flags dependency.

Source signatures still describe one logical LONGINT/LONGCARD parameter. The
Action ABI concatenates its four little-endian bytes with other arguments:
the first three bytes occupy A/X/Y, and subsequent bytes use the usual $A3-up
argument area (including the existing SArgs path for larger signatures).
Four-byte results occupy $A0..$A3. Caller-side result capture must precede any
overlapping outgoing argument write, particularly at $A3. Private arithmetic
helpers have their own independently described input/result signatures.

The initial wide arithmetic helpers consume four word lanes at $82/$84/$C0/$C2
and return low/high words at $C4/$C6. They declare private scratch $82..$87 and
$C0..$C7, A/X/Y/flags clobbers and balanced returning stack effects. They link
only when used, on either runtime. Division additionally declares the existing
Error handler's conservative memory/OS effects. Signed division truncates
toward zero, remainder follows the dividend sign, and MIN/-1 wraps; a runtime
zero divisor reports Error(100) and cannot resume the failed operation. Shifts
are logical and produce zero for counts at least 32, without truncating a wide
count to its low byte. No cartridge arithmetic implementation is used for wide
operators, and no width-changing optimization is required for correctness.

The existing materialization fixture now checks the argument-preserving
JSR/JMP/indirect-JMP trampoline instead of requiring a PHA-based return-address
sequence. This is an intentional emission bug fix; NIR snapshots are unchanged.

Boolean results composed from word comparisons expose their physical branch
reads before spill coloring and cleanup. The existing retained-comparison
selector remains responsible for direct compare predicates. Post-home
verification permits a virtual Boolean condition only with such a retained
compare producer; other condition reads require explicit homes. The barrier
regression now checks the captured Boolean's load/test after the barrier rather
than an implicit temporary reference. This is a correctness fix, not a new
wide-specific optimization pass.
