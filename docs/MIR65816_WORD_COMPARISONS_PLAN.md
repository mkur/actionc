# Native 16-bit comparison implementation plan

Status: implemented on 2026-09-21. Baseline/coverage is committed in `9dff815`,
selection in `51e5bc7`, and qualification in the commit containing the
[results report](MIR65816_WORD_COMPARISONS.md). Maximum measures 146 bytes /
149 VM cycles; optimized sum loop measures 213 / 2,465, matching the estimates.

The approved plan below is retained as the acceptance boundary. It was based on
main `943ed21`, after the completed [native word returns](MIR65816_WORD_RETURNS.md).

## Objective and scope

Use one A16 CMP for eligible two-byte equality/inequality and unsigned ordering,
then materialize exactly one Boolean byte, 0 or 1, in the existing result home.
Apply the same selection to raw and optimized compilation. Signed equality and
inequality are eligible because they compare the same normalized word bits;
signed ordering retains the current implementation.

Preserve physical ABI v1, image v3, the experimental o65 profile, allocation,
stack guards, call barriers, argument/result layouts and public entry contracts.
This slice reduces code, cycles and DP scratch traffic; it does not reduce frame
size. No SemIR/NIR change, compare-to-branch fusion, persistent register/flag
allocation, DP allocator expansion, load folding, branch relaxation, general
mode cleanup, or Exec816 compiler-pin update belongs here. In particular, the
MIR branch still reloads its Boolean instead of consuming flags from CMP.

## Current code and measured baseline

[`Builder::compare`](../src/mir65816/emit/select.rs) compares bytes from high to
low. Each byte stages the right operand through DP `$10`, performs an A8 CMP,
and dispatches to less/greater/equal paths through inverse branches plus JML.
Signed comparisons XOR the most significant bytes with `$80`. Every operation,
including equality, uses the three-way dispatch and stores its Boolean byte.

[`Mir65816Op::Compare`](../src/mir65816/mod.rs) carries the **operand width**;
its destination is always `ByteSize::ONE`. Native ADD/SUB and returns already
provide `WordOperand`, `word_operand` and full-word stack displacement checks.
Reuse these instead of introducing another interpretation of operand homes.

The immutable [post-return snapshot](benchmarks/65816-word-returns/after/tables.md)
is the baseline. Its emitting revision is `cac8aeb`; `943ed21` adds tests and
qualification without changing emission. During planning, all 224 saved file
hashes across 56 build artifacts were verified, together with 264 identical
debug/release measurement records: 14 paired kernels and 66 input vectors.
Representative optimized Action results include guards and RTL:

| Kernel / input | Code bytes | VM cycles | Stack bytes below entry S | DP byte reads + writes |
| --- | ---: | ---: | ---: | ---: |
| identity(13) | 63 | 71 | 4 | 0 |
| add(13,41) | 74 | 98 | 8 | 0 |
| maximum(13,41) | 186 | 173 | 6 | 4 |
| sum loop(13) | 252 | 2,829 | 16 | 56 |
| recursive sum(13) | 286 | 4,171 | 190 | 56 |

In maximum, the current comparison occupies 66 bytes including its initial SEP
and final Boolean store. The sequence below takes 26 bytes when A is already
16-bit and both sources are stack words. For the loop's stack/immediate pair,
it takes 27 bytes. Listing-derived estimates are **146 bytes / 149 VM cycles**
for maximum(13,41), and **213 bytes / 2,465 cycles** for optimized sum loop(13).
These are estimates, not measurements of an implemented change. Establish
regression budgets of at most 155 bytes / 160 cycles and 220 bytes / 2,500 cycles
respectively; require unchanged stack depth and guard costs. Raw output must
also improve, but has its own baseline: sum loop is 254 bytes / 2,904 cycles.

## Selection and instruction sequence

Attempt a private `word_compare` before `operation`'s generic A8 transition.
Select only operand width two, the operators below, an exact one-byte stack
destination, and two eligible `WordOperand`s:

| Operand | Selected representation |
| --- | --- |
| U16 | Immediate preserving all 16 bits. |
| U8 | Zero-extended immediate, consistent with existing bytewise semantics. |
| Temp with matching two-byte stack home | Checked complete stack word. |
| Param with physical width two | Checked word from its authoritative incoming or mutable frame-object home. |

Byte/wide storage, DP homes, null/address/symbolic forms and other legal
unsupported shapes retain the existing path. Do not read a neighbor to widen a
byte home, truncate wider storage, infer signed widening, or recover the original
memory source of a temp. Consume the explicit casts already present in MIR.

Preflight the destination and both inputs before emitting anything, allocating
labels or changing mode knowledge. Check both operands even if an earlier legal
operand is unsupported, so malformed later inputs remain errors. Missing IDs,
width/home mismatches, invalid extents and arithmetic overflow must not become
fallback. A destination must have width one, not width two. Its last valid
displacement is 255; a word source must start at most at 254. Both checks include
the actual `Builder::delta`, reject zero/out-of-range effective displacements,
and use checked arithmetic.

Normalize predicates after classification:

| MIR predicate | Signed flag | Load A / CMP operand | Branch to true |
| --- | --- | --- | --- |
| Eq | Either | left / right | BEQ |
| Ne | Either | left / right | BNE |
| Lt | False | left / right | BCC |
| Ge | False | left / right | BCS |
| Gt | False | right / left | BCC |
| Le | False | right / left | BCS |
| Lt, Le, Gt, Ge | True | Existing bytewise path | Existing dispatch |

Swapping already captured private operands for Gt/Le avoids a second flag test.
It does not reorder source loads, calls or volatile accesses. CMP sets the
unsigned carry/equality conditions independently of incoming carry. It does not
produce the overflow flag needed for a signed subtraction test; do not implement
signed ordering as N XOR an old V flag.

Use `LDA d,S`/`LDA #word`, followed by `CMP d,S` (`$C3`)/`CMP #word` (`$C9`).
Keep existing typed label fixups and `Code::branch` (inverse branch over JML).
The control-flow shape is:

```text
a16(); LDA normalized_left; CMP normalized_right
branch(predicate, true_label)
a8(); LDA #0
jump(done)
mark(true_label); a8(); LDA #1
mark(done); a8(); STA destination,S
```

Consume CMP flags before either Boolean LDA overwrites them. Every arm and the
join explicitly establish A8: [`Code::mark`](../src/mir65816/emit/code.rs) resets
local mode knowledge, so the true path must not inherit fallthrough knowledge.
Retain the final join SEP even if both paths already execute in A8; general mode
tracking is outside this slice. Both control-flow outcomes store exactly one
byte and finish with correct local mode knowledge. Following operations and
terminators retain their existing mode transitions.

The selected sequence uses no DP scratch, helper, push, X or Y, and keeps no
register/flag value alive beyond the operation. Read both source words before
writing the Boolean, including when stack-slot reuse aliases a dead input with
the destination. Calls/helpers may still clobber all 64 DP scratch bytes and
A/X/Y/flags. A value captured before a mutating call stays in its allocated home;
a required reload after that call remains a distinct operation. Original
volatile, pointer and absolute-memory access order/width must remain unchanged.

## Commit 1: semantic coverage and verified baseline

Record baseline revision, compiler/tool hashes, saved artifact verification and
pre-change test results under `docs/benchmarks/65816-word-comparisons/`.
Reuse intact `target/word-returns-after` artifacts. If reconstruction is needed,
build in an isolated checkout of `943ed21`, using that checkout's compiler and
build runner; `build.py --actionc` alone does not change recorded provenance.
Never regenerate the historical snapshot using the new emitter.

Add `tools/native65816-runtime-tests/tests/word_comparisons.rs`, using the existing
serialized-image harness, independent assembly callers and host expectations.
Semantic probes must pass before changing selection. Batch operations and supply
runtime inputs after compilation to avoid recompiling every input pair:

| Coverage | Required cases |
| --- | --- |
| Relations and boundaries | All six CARD relations and all six INT relations over pairs from 0, 1, `$00FF`, `$0100`, `$7FFF`, `$8000`, `$FFFF`; interpret signed expectations independently. Exercise equal operands and high-byte versus low-byte decisions. |
| Boolean consumers | Stored and returned Boolean values must be exactly 0/1; also cover branches, loops, multiple uses and a Boolean surviving a call. Neighboring byte canaries catch an accidental A16 store. |
| Sources and modes | Stack/stack, stack/immediate, immediate/stack, mutable parameters, explicit narrow/signed casts, and a preceding byte operation. Use selector-level probes for shapes source lowering does not naturally retain. |
| Calls and memory | Captured aliased/volatile words, bank-crossing pointer reads and reloads after mutation; retain exact original byte traces. Direct and typed indirect assembly callees clobber all scratch and A/X/Y/flags. |
| Fallback and ABI | Signed ordered relations, byte/three-/four-byte comparisons and DP/narrow operand fallback. Preserve S, D, DBR, widths, binary mode and incoming I; test both I states. |

Use boundary pairs that distinguish unsigned ordering from signed ordering and
from testing only the subtraction sign. Vary incoming C/V in isolated instruction
probes. Test explicit signed widening rather than inventing new mixed-width
language rules. Normalize newline-insensitive fixture text and verify LF/CRLF
through the actual compilation path when adding fixture instrumentation.

## Commit 2: checked native comparison emission

Implement the selector and dispatch gate in `select.rs`, leaving the generic
comparison and branch terminator unchanged. Add a focused private
`compare_tests.rs` beside [`word_tests.rs`](../src/mir65816/emit/word_tests.rs).
Check the predicate mapping, signed Eq/Ne selection, all four signed-order
fallbacks, both input orders, authoritative parameter homes and aliased result
storage. Preflight tests cover missing IDs, bad widths/homes, unsupported then
malformed inputs, last-byte boundaries, nonzero delta and overflow. Confirm
fallback/error leaves code, labels, fixups and mode knowledge untouched.

Add `$C3` to [`disassemble65816.py`](../tools/disassemble65816.py)'s stack operand
table. Extend [decoder tests](../tools/test_disassemble65816.py) for CMP stack
encoding in both modes, CMP immediate widths across REP/SEP, and truncation.
Compare isolated encodings with ca65 output and execute final linked bytes;
never identify instructions by scanning arbitrary operand/data bytes as opcodes.

Add focused whole-routine budgets to
[`mir65816_emission.rs`](../tests/mir65816_emission.rs) and exact dynamic checks
over the selected comparison interval: two byte reads per stack source, one
Boolean byte write, no DP traffic or pushes, and both truth paths. Preserve
frame/storage/call metadata and guards. Update the scalar-selection section of
the [emission contract](MIR65816_EMISSION_CONTRACT.md) with eligibility, signed
fallback, Boolean width and flag lifetime. Run focused checks before committing.

## Commit 3: preemption and measured qualification

Extend the existing exhaustive context test's coverage accounting to identify
reached selected comparison sequences from instruction boundaries, with M=0 at
CMP. Distinguish these from guard comparisons. Require IRQ injection at operand
load/CMP, after CMP while C/Z are live, at both Boolean arms and before/after the
byte store. Cover equality and carry predicates, both outcomes and both task
domains; add a small targeted case where the current corpus lacks coverage.
Verify A/P restoration when the interrupt handler clobbers them. Retain seeded
IRQ/NMI schedules, domain ownership, masking/NMI policy, and existing arithmetic,
return and pointer interruption coverage. Merely varying I is insufficient.

Run full native qualification in both host builds, including relocated o65 at
both placements, stack faults, assembly interop and preemption. Add a targeted
relocated comparison probe if existing o65 cases do not exercise the new path.
Bind actual pass counts and saved artifacts to source, tool, VM and patch hashes.

Rebuild the same 14-pair corpus with LF/CRLF equivalence, and execute raw and
optimized final machine code in both debug/release host harnesses. Save the new
snapshot under `docs/benchmarks/65816-word-comparisons/after/`, with a delta against
the unchanged post-return baseline. Extend `report.py` to retain maximum
listings alongside the existing selections. Report all vectors, DP and stack
traffic, guard costs and unchanged frame/stack peaks; verify unaffected kernels
and require no Action code-size or cycle regression in this corpus.

### Account for complete-word reads explicitly

The old comparison can stop after unequal high bytes. A native CMP always
reads both bytes of each stack source. This is safe for invocation-owned homes,
but **stack-read traffic is not an invariant** even though stack depth and writes
are unchanged. For example, maximum(`$8000`,`$7FFF`) currently reads 14 stack
bytes overall; the selected version should read 16. The original volatile or
shared memory was already captured separately and must retain its exact trace.

Keep [`delta.py`](../tools/compare65816/delta.py)'s strict default. Add an optional
explicit expected-stack-read-deltas input, keyed by case/mode/compiler/vector;
unlisted records still require equality. For this unchanged corpus, predeclare
only +2 for Action maximum vectors 3 and 4 in each target mode: four records,
where the high bytes differ. Verify that prediction against decoded execution
intervals, reject missing/unused keys and other differences, and report the
accepted deltas. Do not derive an unrestricted allowance from observed failures.
All remaining preserved fields and exact vbcc-record equality stay mandatory.
Test both strict rejection and the exact permitted deltas without weakening
arithmetic/return checks. Update report wording so it cannot claim unchanged
stack-read traffic when this option is used.

Retain the known optimized vbcc unlink corruption as a reported failure. Run both
external comparison commands even if the first fails; require zero Action
failures, no new C failures and identical vbcc measurements. Do not call the
external comparison green or include invalid C results in performance claims.

Commit a results document and machine-readable qualification record, update the
native-runner/comparison READMEs and this plan's status, and record measured
results against the estimates. Signed ordering and compare-to-branch fusion
remain separate follow-up slices, chosen from the resulting listings.

## Validation commands for implementation

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission \
  --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -m unittest discover -s tools -p 'test_disassemble65816.py'

python3 tools/native65816-runtime-tests/qualify.py \
  --test word_comparisons --test arithmetic --test word_arithmetic \
  --test word_returns --test execution --test interop --test indirect \
  --test memory --test stack_allocation --test stack_faults

# Final qualification after focused checks:
python3 tools/native65816-runtime-tests/qualify.py -- --nocapture
python3 tools/native65816-runtime-tests/qualify.py --release -- --nocapture
```

Use the [comparison runner](../tools/compare65816/README.md) for build, both
execution commands, reporting and delta generation with distinct artifact
directories. Add/run focused tests for the delta option. Native executions must
use `qualify.py` and its pinned CPU timing correction, not bare cargo in that
workspace. Run the corpus generator's `--check` if fixtures change. Check edited
links, changed-file formatting and `git diff --check`. No full root suite or NIR
sweep is required for this emitter-only boundary; broaden if implementation
crosses that boundary. Report VM timings as such, without claiming board timing.
