# Native signed 16-bit comparisons and branch fusion

Status: completed on 2026-09-23. The baseline, typed forms and selection slices
are committed as `bfa740b0`, `18fd7bd4` and `7afa719f`; final qualification and
measurements are recorded in the [results](benchmarks/65816-signed-word-comparisons/README.md)
and [qualification record](abi/action65816-signed-word-comparisons-qualification.json).
The plan was proposed against main `f27218bc`, following the completed
[BYTE/pointer comparisons](MIR65816_BYTE_POINTER_COMPARISONS_PLAN.md). Its original
scope and forecasts below remain for comparison with the measured outcome.
Other [size backlog](BACKLOG.md#native-65816-code-size-reduction) items remain deferred.

## Objective and scope

Select native A16 signed `<`, `<=`, `>`, `>=` over verified two-byte values.
Materialize one canonical BYTE result for value consumers, or consume the
comparison directly in an adjacent sole-use Branch. Apply the same selection
in raw and optimized emission. Prioritize code size, then measure cycles and
stack/DP traffic.

Preserve physical ABI v1, image v3, o65, frame/home allocation, all stack guards,
call barriers, observable memory accesses and task/IRQ/NMI contracts. Reserve
no new DP or bank-zero bytes. Existing unsigned word and signed Eq/Ne selection,
BYTE/pointer comparisons and the unsigned X-loop rule retain their behavior.

Exclude signed BYTE/four-byte ordering, pointer ordering, zero/constant-specific
shortcuts, source-load folding, nonadjacent fusion, Boolean-expression chasing,
new forwarding/residency rules, scalar DP admission changes, broader branch
relaxation, guard compaction and Exec compiler-pin changes. No SemIR/NIR or
public MIR shape changes and no new optimizer pass belong here.

## Frozen planning baseline

[`word_condition`](../src/mir65816/emit/select.rs) at the planning revision excluded signed
ordering. That fallback compares high bytes after XOR `$80`, then low bytes,
stages through DP `RIGHT`, emits three result arms and stores a Boolean. A
following Branch reloads that Boolean. The existing `Condition`, `native_compare`,
`compare_branch` and `sole_branch_conditions` already provide the integration
points; use them rather than creating another comparison/use-analysis framework.

Planning measurements and source are frozen in
[planning.json](benchmarks/65816-signed-word-comparisons/planning.json) and
[planning-probes.act](benchmarks/65816-signed-word-comparisons/planning-probes.act).
The planning compiler emitted both modes, with identical LF/CRLF images. Routine
sizes include entry guard, argument captures and return code:

| Probe | Raw bytes / frame | Optimized bytes / frame |
| --- | ---: | ---: |
| `RetLt`, `RetLe`, `RetGt`, `RetGe`, each | 157 / 6 | 157 / 6 |
| `BranchLt`, `BranchLe`, `BranchGt`, `BranchGe`, each | 202 / 6 | 202 / 6 |
| `Imm` (`a < 0`) | 161 / 6 | 153 / 4 |

An existing ignored library inventory test was run against current main for
both Dijkstra and the frozen Exec source. It proves width, signedness, exact
Compare/Branch adjacency and sole use; final operand/home eligibility still
requires selector preflight.

| Dijkstra routine | Signed ordered sites, raw and optimized | Predicates |
| --- | ---: | --- |
| `Init` | 2 | Le |
| `Enqueue` | 1 | Ge |
| `Find` | 4 | two Gt, two Le |
| `Benchmark` | 1 | Le |
| Total | 8 | all adjacent sole-branch consumers |

Dijkstra's current baseline is **6,365 bytes raw / 5,779 optimized**, including
22 guards. Optimized `Find` remains 2,132 bytes. Its prior qualified original
benchmark takes 1,971,316,896 / 1,841,244,249 cycles, with stack peaks 96 / 86.
See the [latest results](benchmarks/65816-byte-pointer-comparisons/README.md).
Keep the original Dijkstra comparison and its matched DIV/MOD driver adaptation
immutable; this plan does not add DIV/MOD support.

**Frozen Exec `8e1ff57` has zero signed word ordering sites** in either mode.
Its nine signed word Eq/Ne sites are already native. The optimized shell's
502,845 executable / 519,393 XEX bytes are therefore an equality control, not a
promised size win. Its remaining generic comparisons are other widths. Use the
same frozen source/version strings/configuration when comparing; do not compare
against a moving live Exec checkout or change its compiler pin.

## Reuse from MIR6502

[`materialize/compare_branch.rs`](../src/mir6502/materialize/compare_branch.rs)
already implements signed subtraction and `N xor V`, with overflow correction
in `apply_posthome_signed_relation`. Its checks for stable sources and dead
machine state explain why captured operands and explicit flag lifetimes matter.
Adapt the arithmetic identity and boundary tests to one A16 subtraction. Do not
copy its label-pattern discovery or post-home CFG rewrite: MIR65816 can select
the sequence directly from a verified Compare and its existing branch-use proof.

## Checked selection contract

Extend the existing private word-condition description with a comparison kind
(ordinary CMP versus signed subtraction). Keep common preflight shared by
materialized and fused emission. The ordinary CMP branch must retain its exact
selected instructions; Eq/Ne remain bit equality, irrespective of signedness.

Admit only operand width two and the four signed ordering predicates. Reuse
`word_operand`, `word_home` and parameter-home resolution: captured stack words,
U8/U16 immediates with existing widening rules, and already valid scalar DP word
homes. Do not relax the scalar allocator's whitelist or change placement. An
explicit cast remains responsible for signed widening; U8 immediates are not
implicitly sign-extended. Symbolic addresses, wider/mixed unsupported values
and non-word homes retain fallback.

Validate both inputs and the exact one-byte stack result home, including the
last accessed byte with the current S delta, before labels, instructions or
state changes. Do this even for fusion. Check malformed IDs/widths/bounds as
errors; legal unsupported forms must fall back atomically. Retain the result
home and its metadata when its runtime write is omitted.

The signed path retains the fallback's operation barrier. Do not introduce a
new accumulator-forwarding opportunity as part of this slice. Subtraction and
correction destroy A's source identity; the join must not claim either input
or the arithmetic result as a resident source word. Calls and helpers may still
clobber all scratch, A/X/Y and arithmetic flags. Source volatile/aliased reads
occur separately before Compare and stay ordered.

## Instruction choice and signed correctness

Use binary A16 subtraction and correct the result's sign on overflow. Native
entry/call contracts already require decimal clear. Do not add CLD, change I,
or widen that contract. `SEC` establishes the subtraction's carry input on
every path; incoming C/V are arbitrary.

Conceptually, for `left < right`:

```asm
; A16, binary arithmetic; operands are captured words or immediates.
LDA left
SEC
SBC right
BVC corrected
EOR #$8000
corrected:
BMI true_outcome
```

After subtraction, signed less-than is `N xor V`. Conditional XOR of bit 15
makes N alone encode that truth. EOR can also change Z: **do not use corrected Z
for equality or inclusive ordering**. Normalize predicates instead:

| Source relation | Subtraction order | True branch after correction |
| --- | --- | --- |
| `left < right` | left minus right | BMI |
| `left >= right` | left minus right | BPL |
| `left > right` | right minus left | BMI |
| `left <= right` | right minus left | BPL |

Operand swaps affect captured values only. Equality produces N=0 and therefore
the correct inclusive result. Test overflowing differences explicitly, including
`32767 - (-1)`, whose corrected accumulator can be zero despite unequal inputs.
The hardware flag effects are specified in WDC's
[W65C816S datasheet, tables 5-1 and 5-5](https://www.wdc65xx.com/wdc/documentation/w65c816s.pdf):
CMP does not define V; SBC does; EOR updates N/Z without overwriting C/V. The
sequence above is the compiler design derived from those effects and MIR6502's
existing correction, not a new hardware instruction or persistent signed flag.

The assembly shows logical branches. Emit the correction through typed
`branch(OverflowClear, corrected)`, keeping the existing six-byte inverse
branch/JML form. Do not insert a raw short BVC or register it as a MIR dispatch.
Only the final BMI/BPL uses `dispatch` for a fused Branch, allowing the existing
layout finalizer to choose its normal short/long form. Materialized predicates
use the existing two 0/1 arms and result store. Restore/retain A16 before edges,
and consume the sign decision before any parallel edge copy.

Reuse `sole_branch_conditions` unchanged. Another condition use, either edge's
argument, a returned/passed/stored Boolean, or any intervening operation prevents
fusion. Same-target edges with different arguments and backedges still execute
their original scheduled copies. Keep fused MIR span attribution. A signed
fused pair has one final recorded dispatch; its internal correction branch is
part of the selected CFG, not another source Branch.

## Typed effects, tracking and tooling

Add only the missing forms: `WordOp::EorImm`, `Branch::Minus` and
`Branch::OverflowClear`; `Plus`, SEC and all required SBC operand forms exist.

- [`selected.rs`](../src/mir65816/emit/selected.rs): typed forms/encodings.
- [`effects.rs`](../src/mir65816/emit/effects.rs): A16 EOR reads/writes all 16 A
  bits and writes N/Z, preserving C/V; BMI reads N; BVC reads V. Existing SBC
  effects read C and decimal mode and write A/N/Z/C/V. Keep those distinct.
- [`tracked.rs`](../src/mir65816/emit/tracked.rs): word EOR updates A/N/Z at
  Word width, preserving C/V, with conservative values and joins. Reuse current
  arithmetic and branch transitions; never treat the subtraction as CMP.
- Existing selected-CFG, machine-liveness and replay code must validate the new
  forms without a parallel model. Add focused tests: C is live from SEC to SBC,
  V from SBC to BVC, and N through the correction/skip join to BMI/BPL. Replay
  must reproduce bytes, effects, labels and boundaries, including conservative
  unknown value facts at the join.
- [`layout.rs`](../src/mir65816/emit/layout.rs) already accepts BMI/BPL dispatch
  opcodes. Leave its predicate policy unchanged: overflow branches are internal
  long branches, not new relaxable sites. Retain all relocation/PER remapping.
- [`disassemble65816.py`](../tools/disassemble65816.py) and the native
  [`forwarding` instruction-boundary scanner](../tools/native65816-runtime-tests/tests/support/forwarding.rs)
  need BVC **and BVS** decoding because long BVC is encoded with inverse BVS.
  Their width-sensitive immediate decoders already recognize word EOR. Keep
  unknown/truncated encodings rejected. Add Python decoder tests.
- Keep the existing CMP window decoder word-CMP-specific. Add a separate exact
  signed-window checker in native test support, grounded in MIR spans and final
  bytes, including correction target and final edge transfers. Do not accept
  an arbitrary SBC or nearby branch as evidence of successful selection.

## Size targets

The returned stack/stack probe's current comparison window is 70 bytes from
SEP through the Boolean store. The proposed window is approximately 36 bytes
with A already word-wide, including the long correction branch, long outcome
branch and existing Boolean arms. The branch-only probe currently spends 84
bytes through its Boolean reload and dispatch; the proposed window is 16–20
bytes depending on the existing final dispatcher. These are encoding forecasts,
not results of an implemented selector; surrounding mode requests/layout may
affect whole-routine savings.

Set acceptance ceilings of **130 bytes** for each 157-byte returned probe and
**150 bytes** for each 202-byte branch probe, in both modes, with six-byte frames
and guards intact. Require the immediate probe to shrink in both modes. Freeze
baseline VM counters in slice 0 before introducing selection. Compare operands
must need no new DP scratch writes or pushes; fused predicates remove the
Boolean-home write/reload. Complete word reads can increase private input-read
traffic compared with a high-byte early exit; report this instead of requiring
an invalid universal read-count reduction.

Require net raw and optimized Dijkstra code reduction, including a reduction
in `Find`, and report actual per-routine attribution. Do not promise removal of
its address-generation overhead or extrapolate a whole-program total from eight
sites. Keep all 28 current small-corpus Action images and frozen Exec shell
images/XEX byte-identical; explain any unexpected change before accepting it.

## Commit-sized implementation slices

All four slices are complete. The returned probes measure 123 bytes; branch
probes measure 140, both below their ceilings with six-byte frames retained.
Dijkstra shrinks 487 bytes raw and 489 optimized; all 22 guards and frame/home
contracts are unchanged. The 28 small-corpus images and both frozen Exec shell
images/XEX files are byte-identical. Full native debug/release qualification
passes; the unchanged external vbcc `unlink` failure remains visible in corpus
reports. See the results linked above for counters, provenance and limits.

### 0. Freeze semantic and machine-code baselines

- Add `signed_word_comparisons` native tests and selector eligibility/use-shape
  probes. Include materialized/returned/stored/passed/reused results as well as
  branch-only cases; current fallback must pass them before emission changes.
- Verify the eight Dijkstra sites' operand forms and physical homes. Record
  source/compiler/VM hashes, final listings, frames, guards, cycles and traffic
  for raw/optimized runtime-input probes. Keep planning and earlier snapshots
  immutable; save large artifacts under ignored output directories.
- Freeze the existing corpus, Dijkstra and fixed Exec equality controls. Commit
  tests and baseline evidence with production output unchanged.

### 1. Add and qualify the missing typed instruction forms

- Implement the three forms, state/effect handling and the two decoder additions
  above. Add independent ca65 byte checks and VM execution of the complete
  correction sequence; cross-check the new typed forms through replay.
- Test unknown versus constant tracker values, joins, A16 enforcement and
  independently live C/V/N flags. Do not weaken checked-rewrite proofs.
- Production selection remains unchanged. Require baseline image equality and
  passing focused effects/state/selected/replay/decoder checks, then commit.

### 2. Select native signed predicates and fuse adjacent branches

- Extend common word preflight/kind and implement signed emission. Connect both
  value and branch consumers together through the existing machinery; keep
  native equality/unsigned paths unchanged and existing unsupported fallbacks.
- Add atomic preflight, exact-window, mutable-parameter, overlap and branch-use
  negative tests. Execute both nonempty edges, same-target alternatives and
  backedges with preserved parallel-copy behavior.
- Meet probe ceilings. Validate overflow/no-overflow paths under IRQ/NMI,
  direct/indirect call clobbers and o65 relocation before committing this slice.

### 3. Final qualification and measurement

- Run the complete native suite in debug/release plus the root MIR65816/CLI/o65
  checks. Exercise actual LF/CRLF source compilation and any changed
  newline-sensitive fixture path. Do not edit qualification inputs during a run.
- Rebuild/execute the small corpus and full 33-case Dijkstra comparison, both
  compiler modes and both incoming I states. Preserve the known vbcc optimized
  `unlink` failure in corpus reports. Freeze the CLI binary used by authenticated
  measurements so another Cargo build cannot replace it during execution.
- Rebuild the same frozen Exec shell in raw/optimized modes and require image
  and XEX equality. Any live Exec update is a separate workload. Hosted Exec
  acceptance remains necessary before claiming hosted qualification or changing
  its pin; neither action belongs to this compiler slice.
- Record measured savings, unchanged contracts and limits. Update the emission
  contract, this plan, quality plan and backlog; commit qualification separately.

## Required correctness cases

| Area | Gate |
| --- | --- |
| Signed truth | Cross-product of `-32768,-32767,-257,-256,-1,0,1,255,256,32766,32767`; all four predicates, both orders, runtime operands and seeded broader pairs, host i16 oracle |
| Overflow | `32767,-1`; `-32768,1`; `32767,-32768`; `-32768,32767`; equality at both extremes; arbitrary incoming C/V; both correction paths and corrected-zero/non-equal case |
| Consumers | Canonical 0/1, BYTE return zero extension, stores/call arguments/reuse, two Branch uses, condition in either edge argument, intervening operation/call; exact fusion disqualifiers |
| Physical homes | Captured temp/parameter/immediate forms, mutable parameter, same input, dead-input/result overlap, live-source non-overlap, last valid word/result displacement at nonzero stack delta, invalid widths/IDs/extents and immutable fallback |
| DP scope | Already valid scalar-word homes in private selector tests; unchanged allocator admission and DP reservation, no scratch/result traffic introduced by comparison |
| Side effects | Volatile and aliased captures including bank-crossing words, stores between captures, direct/indirect assembly callees clobbering A/X/Y/P and all 64 scratch bytes; observable traces/canaries unchanged |
| Flags and state | Exact ca65 SBC/EOR/branch bytes and VM truth; independent N/V/C effects/liveness; A16 correction path, conservative join, X16/stack/domain/I invariants and replay equivalence |
| Edges/layout | True/false nonempty copies, same-target different args, backedges, short and long final dispatch, source spans and long correction fixups; no changes to CMP window expectations |
| Preemption | Both incoming I states; representative task instruction sweeps through SEC/SBC/BVC/EOR/join/decision/materialization with full state restoration; seeded IRQ/NMI; retain guard/headroom tests |
| Serialization | Execute JSON and serialized o65 at two code/data placements in different banks; correction and final dispatch targets both remap correctly |

Batch boundary values through a few compiled runtime-input images. Use exhaustive
instruction interruption only for representative overflow/no-overflow and
materialized/fused probes, not for the full input cross-product.

Local validation follows [backend test scope](../AGENTS.md#backend-test-scope).
During development, select the affected native targets (new signed tests,
word/byte/pointer comparisons, compare-branch, effects/selected actions, machine
liveness, state tracking, preemption, stack faults and o65). Final commands:

```sh
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
cargo test --lib --features native65816-state-proof mir65816::
cargo test --test mir65816_emission --test mir65816_contract --test mir65816_abi \
  --test mir65816_state_boundary --test mir65816_o65 \
  --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -B -m unittest discover -s tools -p 'test_disassemble65816.py'
python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
```

The manual corpus/Dijkstra tests additionally require their authenticated
builder outputs, as documented in [compare65816](../tools/compare65816/README.md).
Do not run unrelated 6502/68k suites or a repository-wide NIR sweep for this
backend-only scope. A shared frontend/IR change would require a separate scope
and the broader contributor checks.
