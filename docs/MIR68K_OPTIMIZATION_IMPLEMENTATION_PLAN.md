# MIR68K alignment, control flow and register retention

Status: proposed. Implement in the slices below, committing each major slice
after its validation gate passes.

The objective is to remove the largest avoidable costs exposed by the
[MC68000 GCC comparison](MIR68K_C_COMPARISON.md), then return to language and
platform coverage. This follows the completed
[first code-quality milestone](MIR68K_CODE_QUALITY_PLAN.md).

## Scope and baseline

Keep the original MC68000, existing native ABI and r68k execution path.
Improve pointer alignment, comparison branches and register retention across
blocks. Reuse existing NIR promotion and storage analysis. Physical registers,
condition flags, instruction selection and spill placement belong to MIR68K.

The current optimized Action! baseline is:

| Benchmark | Executable bytes | Instructions | Stack bytes read / written |
| --- | ---: | ---: | ---: |
| Insertion sort | 2,242 | 10,617 | 3,506 / 2,586 |
| Matrix1 | 1,790 | 162,876 | 72,256 / 42,304 |

These are default-entry measurements, including required helpers. The existing
comparison also checks 209 insertion-sort and 22 matrix1 reference vectors per
compiler configuration. Instruction counts are not cycle or wall-clock timings.

GCC parity is not an acceptance target. Multidimensional arrays, new language
constructs, platform startup, interprocedural specialization, a general-purpose
register allocator and additional C benchmark ports remain later work.

## Slice 1: Preserve the comparison and isolate each change

Extend the native measurement tools with explicit switches for the new
optimizations as they are introduced. Record the switches with the compiler
revision, GCC version, flags and inputs. Keep raw/optimized NIR separate from
target optimization settings; promotion is a NIR transformation.

Keep the existing conservative materialization path. Extend `--no-codegen-opt`
to disable the new target transformations as well. Provide a separate way to
select the existing promotion policy for comparisons. Do not silently change
what an old measurement configuration means.

Use the existing seven-benchmark Action! runner and paired C runner. Add stack
traffic to the former using the VM's existing counters, so improvements can be
checked beyond matrix1 and insertion sort. Retain per-slice CSVs and hot-loop
disassemblies under `build/`; publish the final measurements in `docs/`.

Primary files: `tools/vm68k-runtime-tests/examples/code_quality.rs`,
`tools/vm68k-runtime-tests/examples/c_reference.rs`,
`tools/compare_mir68k_c.py`, and the native compilation options.

Acceptance: existing settings reproduce the recorded measurements, and both
runners still validate their results. Keep the C sources and GCC flags fixed
throughout this milestone.

## Slice 2: Carry proven pointer alignment into memory operations

Add a reusable analysis over verified NIR, keyed by routine, block, temp and
storage IDs. Its result describes guaranteed address alignment; it does not
select MC68000 instructions or introduce executable metadata operations.
Recompute facts after NIR transformations rather than caching stale proofs.

Start with addresses of objects with verified layouts, numeric addresses and
static-data relocations. Propagate through pointer copies, compatible casts,
constant offsets, indexed addresses and block parameters. Use the existing CFG,
dataflow and storage analyses to follow stores/loads of private pointer locals,
including loop-carried updates, before broader scalar promotion is available.

The proof must cover these boundaries:

- Alignment of a pointer's storage cell does not prove alignment of its value.
  An array descriptor load needs a proof about the stored pointer. Reuse
  structured backing, region and effect facts to prove a descriptor unchanged;
  an initializer alone is insufficient for a mutable descriptor. Check writes,
  aliases, escaping addresses and reachable call/machine effects. Unknown
  mutation invalidates the claim.
- Joins retain only guarantees shared by every reachable predecessor. Distinguish
  an unvisited path from a reachable unknown value. Loop proofs require a
  grounded entry value and invariant-preserving updates.
- Unknown parameters and call results remain unknown. Truncating casts, odd
  offsets and unproved dynamic strides lose stronger alignment. Calls and
  indirect writes invalidate tracked cells unless existing effects prove that
  those cells cannot be affected.
- The first version need only prove even alignment. Both word and longword
  accesses on the MC68000 require even addresses; longwords do not require
  four-byte alignment. Pointer arithmetic must retain the language's wrapping
  and byte-offset semantics.

Have `src/mir68k/lower.rs` consume the analysis into the existing
`Mir68kAddress` facts. Centralize the effective-address check, including base,
displacement and index stride, so lowering, verification and materialization
agree. Alignment claims must be validated against their derivation, rather
than accepted as arbitrary annotations. MIR68K then selects native word/long
accesses where proven. Preserve the existing access sequence for volatile
operations and the bytewise fallback for unknown or odd addresses.

Primary code: a new analysis under `src/nir/analysis/`, the NIR analysis API,
and `src/mir68k/{lower,verify,materialize}.rs`. Document the fact ownership in
`NIR_TARGET_SHAPE.md` and its use in `MIR68K_EXECUTION_CONTRACT.md`.

Acceptance: matrix1's provably aligned pointer loop uses native longword
accesses and executes fewer instructions. Test aligned/odd absolute addresses,
fields, strides, loop joins, mutable descriptors, escaping pointer cells and
unknown calls. Differential VM cases must preserve values, faults and volatile
access traces. Do not modify the benchmark or assume C pointer alignment to
make this gate pass.

## Slice 3: Branch directly from comparisons and use fallthrough

At MIR68K materialization, recognize a comparison whose result has exactly one
use: the same block's branch terminator. Initially require the comparison to be
the last executable operation. Emit `CMP` followed by the appropriate `Bcc`,
without `Scc`, boolean normalization or a temporary store/reload.

Use typed MIR operands and use counts. Preserve signed/unsigned conditions,
operand widths and evaluation order. Comparisons used as numeric values retain
their existing 0/1 result; ordinary branches still accept any nonzero condition.
No NIR comparison or source-language semantics need to change.

Simplify physical fallthrough after edge transfers have been placed. Omit an
unconditional jump only when its destination is the immediately following
block. Invert a condition when that exposes a valid fallthrough. Preserve
selected-edge argument transfers and their parallel-assignment semantics.
Run existing branch relaxation after the final layout changes.

Primary code: `src/mir68k/materialize.rs`, a small MIR use analysis/selection
module, and physical layout support. Keep the typed MIR comparison and branch
forms unless a demonstrated implementation need requires a contract extension.

Acceptance: focused loops contain direct conditional branches and fewer
executed instructions. Test all comparison relations at BYTE, CARD, INT,
LONGCARD and LONGINT boundaries, both branch arms, backedges, boolean reuse,
and edges with arguments. Existing distant-branch and relocation tests remain
green; literal MC68000 encoding tests qualify any newly emitted instructions.

## Slice 4: Extend existing NIR promotion for native loops

`src/nir/promotion.rs` already implements storage-to-value promotion, including
dominance, block parameters and edge arguments. Its current profitability gates
favor hot bytes, selected word induction variables and short relays. Extend
this machinery with an explicit native promotion policy; do not build a second
SSA promotion engine or globally relax the existing thresholds.

Separate legality from profitability. Continue to require the shared storage
analysis to prove promotion safe. The native policy admits eligible automatic
8/16/32-bit integer and pointer locals used repeatedly in loops. Addressable,
volatile, aliased, persistent or insufficiently initialized homes retain their
current behavior. Passing a pointer value does not by itself expose the local
cell holding that pointer; taking the cell's address can do so.

Thread the policy through `src/nir.rs` and native compilation options. Keep the
current policy as the default for existing 6502 consumers. Run promotion once
in the existing schedule, followed by home elision and normal NIR cleanup.
Initially make the broader native policy opt-in until slice 5 demonstrates its
profitability with register allocation.

Primary code: `src/nir/{promotion,home_elision}.rs`, `src/nir.rs`, native options
and NIR promotion/storage regressions.

Acceptance: native pointer/counter loops expose verifier-clean loop-carried
values instead of repeated source-local loads/stores. Cover conditional
definitions, nested loops, calls, escapes, initializers and read-before-write.
Record intentional native snapshot changes. Existing Atari policy and fixtures
remain unchanged. Promotion alone is not claimed as a performance win if the
values merely spill into new stack slots.

## Slice 5: Retain selected values across blocks

Add a bounded allocator over typed MIR68K temps and block parameters, before
materialization. Compute liveness including edge arguments and use a
deterministic interference-based assignment. Prioritize values used repeatedly
in loops; leave values that do not fit in their existing stack homes.

Start with D4-D7 for scalar integers and pointer values. Materialization can
move a retained pointer into A0/A1 when addressing memory. Keep D0/D1 and A0/A1
as existing scratch/return registers, reserve A6/A7 for frame/stack use, and
leave D2/D3 outside the pool because division borrows them. Additional register
classes and live-range splitting can follow a later measured need.

Implement these invariants together:

- Save and restore exactly the callee-preserved registers used by each routine,
  across every normal return. Validate actual helper clobbers as well as ABI
  declarations. Nested and recursive calls must preserve live caller values.
- Resolve edge arguments as parallel copies, using scratch or a spill slot for
  cycles. Only the selected edge executes its copies. Avoid staging every edge
  argument through memory once its final location is known.
- Define width handling explicitly. Big-endian byte/word stack homes and the
  low bits of a data register are different representations. Preserve required
  extension/truncation behavior and never reuse a cached value at an unrelated
  width.
- Check the allocation against liveness, register reservations, clobbers and
  edge transfers. Retain deterministic spilling under pressure and the
  conservative path for differential execution.
- Assign final frame slots after locations are known. Preserve addressable
  objects and ABI offsets; remove unused private temp homes. Existing NIR home
  elision can omit promoted locals from memory symbols. Never publish a stale
  frame location for a value now held only in a register. A new debugger
  register-location format is outside this slice.

Primary code: new liveness/allocation modules under `src/mir68k/`,
`materialize.rs`, frame handling and allocation verification. Keep the existing
block-local forwarding pass compatible with the new homes and clobbers.

Acceptance: matrix1 and insertion-sort hot loops retain selected counters or
pointers across backedges, with reduced measured stack traffic. Test register
pressure, mixed widths, cyclic edge copies, calls, division, early returns,
recursion and stack/ABI preservation. Enable the broader native promotion and
allocation together only after their combined measurements justify doing so.

## Slice 6: Measure, document and close the milestone

Run the unchanged paired C corpus and all seven native benchmark measurements.
Compare executable bytes, instructions, stack traffic and frames. Inspect hot
loops to tie improvements to the intended mechanisms. Record both cumulative
results and targeted feature-on/off comparisons; do not multiply every existing
test by every possible option combination.

Completion requires all reference states to match, the specific code-generation
gates above to pass, and lower instruction counts and stack traffic for the two
paired benchmarks. Review any regression in the wider corpus and resolve or
explicitly justify it before enabling a new default. Do not set an arbitrary
percentage reduction before measuring the implementation.

Update `MIR68K_C_COMPARISON.md`, the CSVs, execution contract and runner
documentation. Run full cross-platform CI once the milestone is integrated.
Commit the final measurements, then return to construct/platform coverage.
Smaller instruction-selection ideas such as SWAP, postincrement and byte
branches remain follow-ups unless a correctness requirement needs them here.

## Validation budget

For tooling-only slice 1, run affected runner/example tests and baseline
commands. For MIR68K changes, run focused backend checks and the native suite:

```sh
cargo test --lib mir68k
cargo test --test mir68k_contract --test native_routine_abi --test native_type_surface
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml
```

Shared NIR analysis, promotion or contract changes in slices 2 and 4 also need
the repository's required checks before committing:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Measure each completed optimization with the relevant benchmark selection and
paired comparison. Run the wider measurements at allocation/default changes
and final integration. Do not repeat passing suites without further changes or
an unresolved concern. New host-text instrumentation must exercise both LF and
CRLF through its actual path; preserve binary and ATASCII bytes exactly.
