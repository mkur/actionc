# Native 65816 pointer promotion and direct-page allocation

Status: all five slices implemented and qualified in the emulator.

| Slice | Status | Validation |
| --- | --- | --- |
| 1: reproducible reference | Complete | Native `pointer_allocation` target: generated/reference execution, aliasing, exact traces, LF/CRLF |
| 2: NIR promotion | Complete | Five-op unlink, home removal, legality/barrier tests, target isolation, native execution |
| 3: allocation contracts | Complete | Typed homes, closed intervals, corrupt plans, reuse and whole-routine fallback; selection activation in slice 4 |
| 4: selection and maps | Complete | 129 bytes / 189 VM cycles / zero frame; v3 tagged maps, ABI v1 unchanged |
| 5: execution qualification | Complete | 33 native tests in debug/release, differential traces, IRQ/NMI, NIR snapshots/sweep; two pre-existing TN failures retained |

## Objective and measured baseline

Remove avoidable pointer-local traffic and assign short-lived pointer values to
the existing per-domain direct-page scratch region. Implement general behavior
for eligible routines; never recognize a routine name, record name or sample.

The motivating source is an ordinary procedure with no result:

```action
TYPE Node=[Node POINTER ln_Succ Node POINTER ln_Pred]
PROC Remove(Node POINTER item)
  Node POINTER previous,following
  previous=item.ln_Pred
  following=item.ln_Succ
  previous.ln_Succ=following
  following.ln_Pred=previous
RETURN
```

| Implementation | Code bytes | VM cycles | Fixed frame bytes |
| --- | ---: | ---: | ---: |
| Original bytewise selector | 369 | 609 | 38 |
| Previous scalar selector | 223 | 420 | 38 |
| Pointer allocation, optimized | 129 | 189 | 0 |
| Pointer allocation, raw NIR | 191 | 324 | 6 |
| Handwritten direct-page reference | 127 | 200 | 0 |

Measurements include the complete checked entry and RTL, with identical code
placement and three rotations of nodes at `$21FFFF`, `$32FFFC`, and `$43FFFE`.
The reference uses nine bytes of existing scratch at D-relative offsets 0, 3,
and 6; it assumes neither a common data bank nor a changed ABI. Its current
local artifacts are under `target/remove-65816-review/`; these ignored files
are evidence to reproduce, not dependencies for permanent tests.

First acceptance target: optimized `Remove` has no frame objects or stack
temporaries, at most three simultaneously live pointer slots, at most 144 code
bytes and 220 VM cycles on this comparison. Aim to match or beat 127 bytes and
200 cycles. Account for any remaining gap in the generated instructions before
closing the work; do not silently relax the regression budgets.

## Ownership and scope

SemIR continues to own source semantics. NIR owns promotion of proven-private
storage into typed values. MIR65816 owns liveness, direct-page placement,
instruction scratch requirements, and addressing selection. Emission owns
encoding, relocation and accurate physical-location maps. No stage recovers
facts from source strings or display names.

The first allocator applies only to `wdc-65816-native` routines with one block,
no block parameters, no calls, and a procedure return without a value. Admit an
explicit whitelist of ordinary three-byte pointer loads/stores, with direct
storage or indirect addresses and constant displacements supported by Y.
Initially exclude volatile operations, dynamic indexes, pointer arithmetic,
casts, aggregate copies, machine operations and function results. Determine
eligibility from verified typed MIR, not syntactic resemblance to `Remove`.

Other routines use the existing stack allocation and instruction selection.
Scratch pressure also falls back to that complete path in the first release.
This deliberately avoids partial spill/reload machinery, call-crossing live
ranges, CFG allocation, and register allocation in A/X/Y. Those are follow-up
work after the bounded path is qualified. No ABI, pointer representation, DBR
policy or application allocation policy changes are needed.

`--no-opt` continues to disable shared NIR optimization. It does not disable
legal target instruction selection/allocation; test both modes, and apply the
zero-frame performance goal to optimized output.

## Slice 1: reproducible reference and regression harness

- Add the unlink source and corrected assembly reference to the native runtime
  test fixtures. Use the generated ABI constants and preserve the existing
  stack checks. At zero frame size, incoming bytes are at `4,S` through `6,S`.
- Replace dependencies on saved local images with compilation through the real
  native driver. Keep the measured historical baseline documented, without
  retaining a second obsolete compiler or obsolete image format.
- Measure routine-entry-through-RTL cycles with an independent assembly caller.
  Check final bytes, exact neighbor writes, the unchanged removed node, stack
  balance, preserved I/D/DBR, and sixteen-bit A/X/Y at return.
- Cover distinct banks, field/word crossings, a circular singleton and aliased
  neighbors. Preserve original read-before-write and store ordering.
- Normalize checked-out assembly/source text when performing newline-sensitive
  extraction; verify both LF and CRLF through any new such path.

Completion: the current compiler and assembly reference are executable under
one durable harness, with explicit budgets and no source-name special cases.

## Slice 2: native pointer promotion and home removal

Files: `src/nir/promotion.rs`, existing storage analysis/optimizer/home-elision
modules, `src/compiler/native65816.rs`, and native promotion tests.

- Introduce an explicit 65816 profitability policy that retains current native
  loop promotion and additionally admits bounded straight-line pointer homes.
  Select it in the native driver; preserve other targets' existing policies.
- Reuse `is_promotable()` and `is_proven_private_to_invocation()`. Require exact
  pointer types, definite initialization, ordinary automatic storage, and no
  address, alias, volatile, initializer or machine-visibility blockers. Do not
  weaken these proofs merely to obtain the benchmark result.
- Promote eligible locals with the existing SSA rewriting machinery. Capture
  immutable, non-address-required pointer parameters once, using the existing
  parameter-capable promotion machinery where its entry-value proofs apply.
  Do not generalize this into forwarding loads from arbitrary pointed-to memory.
- Preserve indirect loads/stores and their order/effects. A promoted private
  pointer value surviving a pointer write does not license reusing a pointee
  load or changing alias assumptions.
- Run existing cleanup/home elision after promotion. Verify that `previous`
  and `following` disappear from executable storage and physical frame plans;
  removing just their stores would leave the six-byte local frame allocated by
  `mir65816/lower.rs`. Prefer existing `home_elision` over new pruning rules.
- Verify before and after transformation. Preserve readable names as metadata.

Completion: optimized unlink NIR contains one captured input pointer, two
pointer-field loads, two pointer-field stores and a procedure return, with no
local pointer reloads or homes. Tests prove rejected promotion cases and
unchanged classic/small-model/68k policy behavior.

## Slice 3: explicit allocation locations and scratch ownership

Files: `src/mir65816/emit/allocation.rs`, `emit/mod.rs`, `emit/select.rs`, and
allocation contract tests.

- Replace the assumption that every `TempId` maps to a stack `Slot` with a typed
  location such as `Stack { offset, width }` or
  `DirectPage { offset, width }`. Preserve stable temporary IDs and deterministic
  allocation. Keep frame objects, stack spill bytes, and DP usage distinct.
- Describe the selected leaf operations' scratch reads/writes explicitly. The
  current selector is not safe to reuse unchanged: `PTR` occupies 0..2,
  `RESULT` occupies 8..11 and overlaps the third pointer slot, and copies,
  arithmetic, indexing and call/return marshalling use other scratch ranges.
- Allocate the bounded path from the ABI's pointer slots at 0, 3 and 6. Make
  indirect memory operands carry their actual pointer-slot identity; do not
  reconstruct every pointer in the hard-coded `PTR` slot. Any operation needing
  unmodelled scratch makes the routine ineligible.
- Compute typed MIR def/use intervals, including address operands, stored
  values and terminator uses. Never parse printed IR. Treat inputs and outputs
  of a multi-instruction operation as simultaneously live until the complete
  operation finishes: overwriting an address slot after loading only its low
  word can corrupt the remaining bank-byte access.
- Use deterministic linear allocation for this single block. Reuse a slot only
  after the prior value's full last use. With more than three required slots,
  choose the existing stack path for the whole routine before emitting bytes.
- Validate the completed plan before emission: all definitions/uses have valid
  typed locations; live ranges do not overlap in a slot; scratch clobbers miss
  resident values; DP accesses stay within owned scratch; and no DP value crosses
  an unsupported operation or boundary. Test intentionally corrupted plans.
- Allocate stack space only for remaining physical stack homes. Recompute final
  frame extent, incoming offsets, spill bytes and local stack peak from the
  completed plan, retaining all existing last-byte/range checks.

Completion: allocation is explicit and verifiable, stack fallback is complete,
and no DP address is misrepresented as a stack displacement.

## Slice 4: consume allocated pointers directly and publish accurate maps

Files: `src/mir65816/emit/select.rs`, `emit/code.rs`, `src/mir65816/image.rs`,
`tools/disassemble65816.py`, and affected CLI/image consumers.

- Teach value loads/stores and pointer-field selection to consume both location
  kinds. Load field results directly into their allocated homes; store resident
  pointer values through the base slot without intermediate frame copies.
- Keep constant field offsets in Y and all 24 address bits intact. Do not
  mutate a resident pointer to form an address. Ineligible offsets/indexes use
  the ordinary routine path until their scratch behavior is separately covered.
- Preserve exact source memory extents: no fourth-byte pointer access and no
  overlapping external/indirect accesses. Existing overlapping-word transfers
  are usable only within owned internal storage under their documented rules.
- Maintain known M/X state, including labels and return. Use the existing
  sixteen-bit/byte transfer selection first; add INY-based sequences only if
  measurement justifies them, with encoder/disassembler and execution coverage.
- Emit the checked zero-frame entry and RTL when no stack storage remains. Do
  not remove stack checks or fabricate a return value to meet a size target.
- Change image temporary metadata to an explicit tagged physical location.
  Bump the image transport version to 3; retain physical ABI v1. Emit/read v3
  and reject older images with a recompile diagnostic, following the existing
  transport-version policy. Update the disassembler and every version-sensitive
  CLI, image, runtime-fixture and documentation consumer together.
- Image validation checks stack homes against frame bounds and DP homes against
  scratch bounds. Lifetime/scratch overlap proofs belong to the allocation
  validator; a final map alone cannot prove them. Image round trips must retain
  all locations and must not report DP residents as spills or omit their IDs.

Completion: optimized unlink uses three DP pointer values, no local frame, the
unchanged public ABI, and truthful exported allocation metadata.

## Slice 5: qualify behavior and code quality

Add focused cases covering:

| Area | Required evidence |
| --- | --- |
| Generality | Unlink plus pointer copy/swap, chain traversal within one block, field permutation and different record offsets/names |
| Lifetimes | Values used both as addresses and data; dying address versus newly loaded pointer; three live pointers and pressure beyond capacity |
| Conservative fallback | Direct/indirect/recursive calls, branches and block parameters, escaping/addressed locals, volatile memory, indexed addresses, large offsets, aggregates and pointer results |
| Memory semantics | Separate banks, crossings at `$xxFFFF`, exact three-byte extents, aliasing neighbors, retained load/store order and guards |
| ABI/maps | No frame for unlink; correct incoming offsets; mixed parameter layouts; last stack byte at 255/256; DP bounds, transport round trips and corrupt allocation rejection |
| Asynchronous execution | Inject IRQ at every reachable enabled instruction of a fixture that holds all three DP pointers live; two tasks executing the routine; seeded IRQ/NMI schedules under the existing platform policy |

Compare against the existing stack strategy and the independent reference in
the VM. Test both NIR optimization modes. Run the complete native qualification
in debug/release because DP ownership, hidden accumulator state and suspension
are correctness boundaries. Do not infer interrupt safety merely from a leaf
routine having no ordinary calls.

Required compiler checks after promotion/lowering/contract changes:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
python3 tools/generate_abi65816.py --check
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

During development run affected promotion, storage, ABI, allocation, emission,
image and native runtime targets. Run the required broad checks at integration,
without repeatedly rerunning already passing matrices absent new changes.
Explain NIR fixture changes as intentional optimized storage-to-value changes;
raw NIR meaning and memory ordering remain unchanged.

Track independently observed TN sample failures separately: sample-catalog
coverage and standalone analysis of `LOCATION.ACT` with missing `DIR_*` facts.
Recheck their current status; do not suppress failures or alter unrelated sample
work to present this optimization as a fully green repository run.

Update the lowering/emission contracts, image-format documentation, native
runtime coverage table and qualification evidence with the final implementation.
No implementation slice is complete with weaker verification, incorrect frame
maps, a special case for `Remove`, or unexplained memory-order changes.

## Final qualification

The [qualification record](abi/action65816-pointer-allocation-qualification.json)
binds compiler/fixture hashes and saved context artifacts to these results:

- 33 native execution tests pass in debug and release, with identical inputs.
- NIR snapshots and all 51 sweep fixtures pass without fixture changes.
- 13 emission/allocation/image tests and five promotion tests pass, including
  classic, small-model and 68k policy isolation.
- Pointer IRQ injection covers 164 raw and 100 optimized `(task domain, PC)`
  sites. Both tasks and IRQ dispatch execute the same leaf with separate scratch.
  Two seeded IRQ/NMI schedules pass in both optimization modes.
- ABI generated files are current; all eight saved v3 context images disassemble.
- The final full compiler run reports 3,256 passed, 24 ignored and two
  pre-existing TN failures: missing catalog roles and standalone `LOCATION.ACT`
  analysis without imported `DIR_*` constants. These are retained unchanged.

## Measured selection tradeoff

The selected word-plus-bank-byte field transfers use 129 bytes and 189 cycles,
versus the reference's byte transfers with INY at 127 bytes and 200 cycles.
Both include the same checked entry. The two-byte size difference buys eleven
fewer cycles, without adding index-state tracking or a new encoding. Retain this
tradeoff within the explicit 144-byte / 220-cycle acceptance limits.

## Follow-up scope

After the bounded path is qualified, consider allocation across supported
operation regions, structured clobber-aware spills around calls/helpers, CFG
liveness and stack-slot reuse. Each extension needs its own verifier guarantees
and asynchronous execution evidence. A shared-bank pointer model or a new ABI
is not part of this work.
