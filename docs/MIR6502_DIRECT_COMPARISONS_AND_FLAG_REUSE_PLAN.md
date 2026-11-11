# MIR6502 direct comparisons and flag reuse

Prepared 2026-11-11. Implementation is pending. The starting compiler is
`1eadf8f14cae7177f0b5e8c483502769415783fb`, whose compiler sources match the
frozen TN comparison revision `d11fe9ca`.

Extend the existing MIR6502 comparison selectors so an unsigned BYTE loaded
through a pointer can remain in A and feed an immediate carry branch. Start
with a previously captured left operand and an indirect right operand in
`left <= right` or `left > right`. Keep the original capture and memory-read
sequence. This is a backend slice using the current typed MIR forms.

The [TN/Oscar64 callable scorecard](../surveys/tn/c-oscar64/kernels.json)
provides a concrete workload. Its callable `Instr` bodies measure 66 bytes for
Actionc and 34 for Oscar64, with 15,832 versus 8,683 cycles in one long search.
Those figures include ABI, pointer placement and loop differences. Measure the
contribution of this selector independently before attributing a saving to it.

## Existing machinery and confirmed gap

The current backend already provides the following pieces.

| Home | Existing responsibility |
| --- | --- |
| [compare_branch.rs](../src/mir6502/materialize/compare_branch.rs) | Comparison operand consumers, addressed BYTE forwarding, unsigned operand reversal and branch selection |
| [pilots.rs](../src/mir6502/rewrite/pilots.rs) | Candidate discovery and routine-wide definition/use proofs |
| [context.rs](../src/mir6502/rewrite/context.rs) | Reaching definitions, liveness and generation-qualified proof queries |
| [machine_values.rs](../src/mir6502/analysis/machine_values.rs) | Available register values and machine-state facts |
| [driver.rs](../src/mir6502/rewrite/driver.rs) | Transaction validation, effect checks and analysis invalidation |
| [materialize.rs](../src/mir6502/materialize.rs) | Ordering of producer selection, operand reversal, branch expansion and final placement |

`addressed_byte_compare_candidate` currently recognizes three adjacent
operations comprising an addressed BYTE load, a direct BYTE load, and their
comparison. It forwards the addressed value into A and can reverse the
predicate. Its pilot proves that the removed definitions have no remaining
uses. `inclusive_compare_reversal_candidate` handles two directly addressable
BYTE operands whose reads may be reordered.

A fresh raw-MIR dump of TN's `Instr` has a different header sequence.

```text
v0 =.b load global g0+0
v1 =.w load param p0+0
v2 =.b load computed v1[#$00;1]+0
v3 = cmp.b v0 le v2
branch bool v3 ? b2 : b6
```

The index capture precedes the pointer and length loads. The addressed-load
selector's adjacent direct-load pattern therefore does not match. The current
materialized header stores the loaded length in a private byte, reloads the
index into A, and branches on a combination of carry and zero.

```text
a =.b load_indirect (zp$AC),y+0
store.b zp0, a
a =.b load global g0+0
flags = cmp.b a le *zp0
branch flag c_clear|z_set ? b2 : b6
```

Use raw typed MIR as the source of capture and evaluation-order obligations.
The existing materialized sequence describes emitted work and supplies the
measurement baseline.

## First implementation shape

Match the final addressed BYTE load and comparison without requiring the
other operand's defining load to be adjacent. Resolve that operand through the
shared reaching-definition analysis and retain its original capture.

```text
v0 =.b load index_home
v1 =.w load pointer_home
v2 =.b load *v1+0
v3 = cmp.b v0 le v2
branch bool v3 ? body : exit
```

Select the following equivalent value flow.

```text
v0 =.b load index_home
v1 =.w load pointer_home
a =.b load *v1+0
v3 = cmp.b a ge v0
branch bool v3 ? body : exit
```

The existing branch materializer then consumes carry directly. For the initial
predicates the truth mappings are as follows.

| Original predicate | Selected comparison | True condition |
| --- | --- | --- |
| captured index <= loaded byte | loaded byte >= captured index | C set |
| captured index > loaded byte | loaded byte < captured index | C clear |

Keep the captured index in its ordinary selected home when needed. An
additional proof may allow a direct memory operand if its value remains equal
to the original capture at the comparison. The original capture order alone
provides no permission to reread that cell after an arbitrary indirect access.

When the direct memory operand is proved available, the target length check
can have this form.

```asm
LDY #0
LDA (pointer),Y
CMP index_home
BCC exit
```

When the snapshot must remain, `CMP captured_index` supplies the equivalent
comparison. Final allocation decides whether removing the length home produces
a net saving after retaining the index snapshot and preparing the pointer.

## Legality and architecture

SemIR continues to establish Action! typing and evaluation order. NIR supplies
verified typed loads, captured values, comparisons and CFG edges. MIR6502 owns
A placement, predicate orientation, pointer-address preparation and C/Z branch
selection. Emission writes the selected operations. The existing
[pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md) and
[rewrite workflow](MIR6502_REWRITE_WORKFLOW_PLAN.md) govern the implementation.

Each accepted candidate must establish all of these properties.

- Both compared values are unsigned BYTE values. The first scope covers `<=`
  and `>` with the addressed load on the original right side.
- The addressed load immediately precedes its comparison in the same block.
  Its address, width, execution count and source-read order are preserved.
- The comparison is the block's final operation and its Boolean result feeds
  the terminator exclusively. Every removed loaded-value use resolves to the
  same reaching definition. Check successors, backedges and edge arguments.
- The other operand is a captured BYTE with a proven reaching definition.
  Retain its definition and any other uses. Preserve pointer producers and
  source-visible global, local, parameter and fixed-cell writes.
- Pointer materialization preserves the captured operand. Retain a physical
  capture home when the pointer setup needs A or another occupied register.
  Validate complete byte lanes and actual clobbers through existing analyses.
- Predicate reversal preserves the branch truth and edge identities. Any
  changed A/X/Y or residual C/N/Z/V state is dead or explicitly preserved on
  every outgoing path, apart from the selected branch's intended condition.
- Calls, runtime helpers, machine blocks, writes and opaque barriers break the
  candidate window. Existing volatile-access barriers stay in their original
  positions. A hardware or unknown indirect access requires an exact retained
  read/effect representation for forwarding to qualify.
- The ordered source-read sequence is checked explicitly. The rewrite driver's
  aggregate effect equality alone does not prove access order. Forwarding a
  value keeps the originating read at the same point.
- Removed temporary stores, address preparation and allocation changes satisfy
  existing home-observation, physical-alias and placement checks. Include
  aliases to parameter homes, fixed zero page and any affected scratch slots.
- Discovery uses typed routine, block, temp and storage identities. The same
  shape must qualify with different names, addresses and surrounding programs.

The first slice uses `Load`, `Compare`, `MirCondDest::Flags` and
`MirCond::FlagTest` through existing materialization. General flag retention
across joins, call boundaries and independent comparisons belongs to a later
extension with its own evidence. Signed and word comparisons, pointer-loop
hoisting, argument staging and source-loop changes have separate scopes.

## Implementation slices

### 1. Freeze observations and reproduce the missing form

Save the compiler revision, configuration, raw MIR, materialized MIR, assembly,
XEX hashes and routine sizes for the neutral example below and TN/TNDBG.
Record both cartridge and standalone runtime builds. Preserve the current
raw/optimized NIR modes used by the affected VM harnesses.

The following commands reproduce the TN observation with the current compiler.

```sh
cargo build --locked --bin actionc --bin actionc-emit
mkdir -p build/direct-comparisons
target/debug/actionc-emit --backend mir6502 --profile modern --runtime standalone --emit-mir6502 samples/tn/modern/TN.ACT > build/direct-comparisons/TN.raw.mir
target/debug/actionc-emit --backend mir6502 --profile modern --runtime standalone --emit-materialized-mir6502 samples/tn/modern/TN.ACT > build/direct-comparisons/TN.materialized.mir
```

```action
MODULE
BYTE index=$E0
BYTE result=$0600

PROC Probe(BYTE POINTER base)
  IF index <= base^ THEN
    result=17
  ELSE
    result=29
  FI
RETURN

PROC Main()
  Probe($6000)
RETURN
```

Use runtime-initialized bytes at `index` and `$6000` when executing the example.
Also add a same-shape case with ordinary named storage and a source-independent
routine name. Use the smaller examples to distinguish shape mismatch from
liveness, effect, alias and profitability rejection. Reuse the current
optimization reporting facilities to record the actual blocker.

Acceptance requires a reproducible missed case, its actual pipeline location,
and a neutral example with an independent unsigned-comparison oracle.

### 2. Extend the analyzed producer selector

Extend the addressed BYTE comparison family to accept a previously captured
operand and the adjacent addressed-load/comparison suffix. Retain the earlier
capture and pointer producers. Reverse `<=` to `>=` or `>` to `<` so the loaded
BYTE is the comparison's left operand in A.

Reuse the existing discovery pilot, reaching-definition queries and removed-use
checks. Carry the predicate-equivalence and source-order justification with the
candidate. Rebuild invalidated analyses after an accepted transaction. Keep the
current adjacent-load cases working through their normal entry points.

Run the existing branch materializer after the producer rewrite. Its flag
comparison and terminator must stay connected through the final materialized
operations, including any setup that writes A, Y or status flags. Use the
existing verifier and machine-state checks to validate that connection.

Gate the extension with the existing optimized peephole configuration. Keep
feature-off output available for differential measurements. Add an applied
counter and a useful blocker reason using the current reporting model.

Acceptance requires the neutral case to reach a single carry condition, preserve
its captures and ordered reads, and improve emitted bytes or executed cycles
with no image growth. Changes must also pass the existing adjacent-load cases.

### 3. Qualify captures, effects and flags

Batch related checks around distinct boundaries.

| Test layer | Required coverage |
| --- | --- |
| Selector and proof tests | Both predicates, branch polarities, pointer/index operand forms, dominating capture, a capture with extra uses, and a loaded value used in a successor |
| Def/use and CFG negatives | Ambiguous reaching definitions, redefinition, backedge/edge-argument uses, a Boolean with another consumer, and stale rewrite generations |
| Machine-state tests | Pointer setup clobbering A/X/Y, independently live flags on successors, and intervening flag writers. Execute baseline/selected comparison fragments over all 65,536 unsigned input pairs, checking the host oracle, branch destination and required exit state. |
| Effect tests | Exact pointer/operand read order and count, retained volatile barriers, hardware-read fallback, calls, machine blocks and intervening stores |
| Physical alias tests | The pointer addressing the index cell, either byte of its parameter home, affected zero-page/scratch homes, and page-crossing addresses |
| VM execution | Values 0, 1, 127, 128, 254 and 255 spanning less/equal/greater outcomes, repeated calls, observable final index/capture values and both runtimes |
| Existing behavior | Stored/returned Boolean values, signed/word comparisons, short-circuit consumers, comparison loops and previously supported direct-operand reversal |

Run exhaustive truth checks at the cheapest meaningful instruction/proof layer.
Use a focused boundary matrix for full compiler/VM executions. Extend existing
CPU and read-trace helpers where they express the required observation.

Normalize host text before newline-sensitive source instrumentation. Compile
and execute both actual LF and CRLF forms through the affected fixture path.
Preserve binary and ATASCII bytes exactly.

Acceptance requires the positive examples to execute correctly and every
unsupported or observably different shape to keep its valid fallback.

### 4. Measure applicability and qualify the backend

Count selected and blocked sites in TN, TNDBG and representative existing
library, graphics and runtime fixtures. Record the reason for remaining
rejections. Compare optimized feature-off/on builds with identical source,
origin, runtime and NIR mode.

Report the following measurements together.

- Neutral-example and affected-routine emitted bytes and cycles for distinct
  comparison outcomes, including page-boundary placements.
- TN/TNDBG complete XEX sizes, code and storage extents, and occupied zero page.
- Candidate, applied and blocked counts across the inspected corpus.
- Ordered-read traces and any differences in retained capture homes or pointer
  setup. Account for branch relaxation and final layout changes in whole-image
  results.

Use the current comparison-family ranking and final costing support where
applicable. Check the resulting emitted bytes and required capture homes through
the ordinary pipeline. A measured regression should narrow the candidate or
add a focused profitability check using existing cost machinery. Preserve the
previous valid materialization on a proof, verification, layout or cost failure.
Report estimated local savings separately from measured bytes and target cycles.

Finish with a measured improvement in the neutral example and a truthful TN
result. An unchanged TN result requires the exact remaining blocker and the
measured scope to be recorded. The 32-byte callable gap measures all differences
between the two bodies and serves as an investigation reference.

## Validation commands for implementation

During development, run the affected selector/rewrite tests by filter. The
final MIR6502 checks should include the affected unit tests and explicit
integration targets, followed by the emission/load fixture sweep.

```sh
cargo test --locked --lib mir6502::
cargo test --locked --test mir6502_materialization_gap --test mir6502_rewrite_inventory --test mir6502_fixtures
cargo run --locked --bin actionc-mir6502-sweep -- --emit-load fixtures/mir6502
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test comparison_values --test comparison_loops --test tn_codegen_reproducers --test tn_dispatch
```

Run the new regression target as well if it has a separate integration or VM
home. The opt-in Oscar64 comparison target can confirm that its frozen Action!
and C controls stay reproducible. Measure the changed compiler's TN images
separately so the frozen controls keep their original identities.

These are MIR6502 backend checks under [AGENTS.md](../AGENTS.md). If the
implementation changes shared NIR, semantic lowering, verification, printing or
their contracts, also run the mandated shared checks.

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Keep existing snapshots unless the selected materialized output intentionally
changes. Explain each changed fixture as a target-code selection improvement
or a bug fix. Update the existing MIR6502 contract if implementation introduces
a new invariant that consumers need to rely on.

## Completion evidence

Record the implementation revision, completed slices, proof boundaries,
validation scope, and before/after measurements here. Keep large dumps, images
and logs under ignored `build/direct-comparisons/`. Add the focused tests and
compact measured results with the implementation slice.

### Slice 1 observations

The [baseline manifest](benchmarks/6502-direct-comparisons/baseline.json)
records the starting compiler and source hashes, image hashes and routine
sizes. The generic fixture is
[direct_comparisons.act](../fixtures/runtime/direct_comparisons.act).
`AtMost` and `Greater` use an absolute-backed zero-page index. `NamedAtMost`
uses an ordinary named index through a local BYTE capture. Each pointer input
is populated at runtime by the VM test.

All three reproduce the missing capture-first shape. The original adjacent
addressed/direct-load selector requires the direct load after the addressed
load, while these routines define the capture earlier. The final length
comparison therefore spills the indirect value and uses two status tests.
With optimized NIR, the callable routine sizes are 30, 30 and 31 bytes. Their
selected sample calls take 54, 54 and 55 VM cycles, including the common call
wrapper, for operands 128 and 254. Raw NIR retains the local capture's extra
work and takes 75 cycles in `NamedAtMost` for that case.

The baseline VM target passes unsigned host-oracle checks for all three
routines at two pointer placements, including a page boundary. It executes
both actual LF and CRLF sources under raw/optimized NIR and both runtimes.
The resulting LF/CRLF images match byte for byte.

```sh
ACTIONC_COMPARE_METRICS=1 cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test direct_comparisons -- --nocapture
```
