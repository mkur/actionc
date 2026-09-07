# LONGINT/LONGCARD integration and MIR6502 implementation

Status: in progress. Each major slice is a separately verified commit.

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
- Slices 3–5: pending. MIR6502 still deliberately rejects runtime wide values
  at this integration boundary; native backends retain their existing lowering.
