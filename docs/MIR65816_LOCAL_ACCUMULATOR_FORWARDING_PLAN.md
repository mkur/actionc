# Native 65816 local accumulator forwarding implementation plan

Status: implementation in progress on 2026-09-21 against main `0e8248c`, after direct single-word
edge copies. This is step 2 of the [code-quality roadmap](MIR65816_CODE_QUALITY_PLAN.md).
The measurements below are the existing baseline; the reductions are forecasts.

## Objective and first-slice boundary

Remove a private word reload when the immediately preceding eligible MIR
operation has already left that exact temporary in A16, with matching N/Z.
Keep the temporary's store and allocated home. Apply the same selection to raw
and optimized MIR, after existing verification and allocation.

For example, retain the first store but omit the second instruction here:

```asm
STA result,S       ; native ADD/SUB has left this word and its N/Z in A
LDA result,S       ; redundant private reload
TAY                ; existing word-return teardown begins
```

This first slice is deliberately adjacent-operation forwarding. It does not
track values through arbitrary intervening instructions, across block entries,
or through edge assignments. It establishes the value/width/flags proof needed
before broadening register retention.

Preserve [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md), image v3, the
[o65 profile](MIR65816_O65_PROFILE.md), public argument/result placement, all
stores, complete frame maps, staging reservations, stack checks, interrupt
headroom and Exec816's compiler pin. No NIR change, home coalescing, frame
shrinking, DP allocation, X/Y retention, operand reordering, mode propagation,
branch relaxation or store elimination belongs in this slice.

## Rechecked baseline and representative forecasts

Current main has CFG-aware stack-slot reuse, bounded pointer-leaf DP allocation,
native word arithmetic/returns/comparisons, comparison fusion, word edges,
empty-edge cleanup and direct single-word edges. It is newer than Exec816's
`c2268b7` pin. The current [encoder](../src/mir65816/emit/code.rs) tracks only local
accumulator width; `mark` invalidates that knowledge at every label. The
[selector](../src/mir65816/emit/select.rs) still reloads operands unconditionally.

Planning rechecked all 224 saved artifact hashes across 56 builds in
`target/single-word-edges-after`, the matching 264 debug/release measurement
records, the 40 benchmark files and the 15 changed code/test hashes in the
[previous qualification](abi/action65816-single-word-edges-qualification.json).
The corpus contains 14 pairs and 66 vectors, each executed with both incoming
I states. See the immutable [snapshot](benchmarks/65816-single-word-edges/after/tables.md)
and this plan's [baseline record](benchmarks/65816-local-accumulator-forwarding/baseline.json).
No compiler or VM tests were rerun merely to prepare this document.

Each removed A16 `LDA d,S` saves two static bytes, one executed instruction,
five qualified VM cycles and two private stack-byte reads. Writes, DP traffic,
guard costs and stack peaks remain unchanged. Entry-through-RTL forecasts:

| Kernel / input | Mode | Static reloads removed | Executed reloads removed | Bytes before / after | Cycles before / after | Stack reads before / after | Unchanged stack peak |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| identity(13) | raw / optimized | 1 | 1 | 63 / 61 | 71 / 66 | 7 / 5 | 4 |
| add(13,41) | raw / optimized | 1 | 1 | 74 / 72 | 98 / 93 | 13 / 11 | 8 |
| subtract(13,41) | raw / optimized | 1 | 1 | 74 / 72 | 98 / 93 | 13 / 11 | 8 |
| maximum(13,41) | raw / optimized | 3 | 2 | 110 / 104 | 111 / 101 | 15 / 11 | 6 |
| sum_loop(13) | raw | 5 | 53 | 174 / 164 | 2,032 / 1,767 | 303 / 197 | 14 |
| sum_loop(13) | optimized | 3 | 40 | 146 / 140 | 1,595 / 1,395 | 245 / 165 | 16 |
| byte_sum($12FFFC,16) | raw | 5 | 65 | 272 / 262 | 5,003 / 4,678 | 660 / 530 | 16 |
| byte_sum($12FFFC,16) | optimized | 3 | 49 | 244 / 238 | 4,476 / 4,231 | 590 / 492 | 22 |

These forecasts come from the saved listings, storage maps and source control
flow, not a proposed emitter's statistics. In each optimized loop, the condition
reload executes `n+1` times, the decrement input reload `n` times, and the reload
before writing the mutable counter `n` times: `3n+1`. The raw loops instead have
four eligible reloads per body and one at return: `4n+1`; the bytewise constant
between the counter load and comparison prevents condition forwarding.
Maximum removes one comparison reload and the return reload on the selected arm;
both return arms contribute static sites. Add/subtract retain the left-operand
load: the preceding operation has put the right operand in A.

The baseline record lists the exact old reload addresses and formulas. These are
representative forecasts, not the complete corpus acceptance list. Before
changing emission, inventory all 28 Action builds and freeze every selected
site/count, including call-containing kernels and rotation. Do not treat cases
omitted from this table as predicted zero. Reconstruct missing baseline artifacts
from an isolated `0e8248c` checkout and its runner; never overwrite prior snapshots.

The baseline slice freezes [all sites](benchmarks/65816-local-accumulator-forwarding/expected-sites.json)
and [positive execution counts](benchmarks/65816-local-accumulator-forwarding/expected-reloads.json):
76 sites and 1,422 executions per incoming I state. Its typed index verifies all
28 Action images remain byte-identical. Four new native probes and the five
existing word-edge probes pass in the debug host (`run-uiudrtc_`).

To avoid reproducing the selector in the test index, `Code.mir_spans` records
nonserialized operation/terminator ranges keyed by block and operation index.
These are emission proof metadata, not ABI or optimizer inputs. The independent
index checks typed adjacency, homes, actual instruction boundaries, producer
stores and consumer encodings against these ranges. No emitted bytes change
in this baseline slice.

## Checked selection

Keep target strategy in MIR65816 selection, consuming verified typed identities
and the allocated frame. Do not perform a raw byte peephole or teach generic
`load_memory` to suppress arbitrary loads.

### Producers

Only these completed sequences may publish a resident-word fact:

1. Existing native two-byte ADD/SUB ending with `STA private_temp,S`.
2. An ordinary, nonvolatile, exactly two-byte MIR `Load` into a two-byte stack
   temporary, using a direct, nonindexed address: automatic frame object,
   parameter, absolute address, or resolved symbol. Its existing A16 source
   read and destination store both remain. Reuse current address/extent checks.

The source of a `Load` is not cached. Only its private captured result is known.
An incoming parameter or addressable local never itself becomes a resident home.
Indirect/indexed loads, DP homes, byte/three-byte/four-byte operations, casts,
constants emitted bytewise, call-result marshalling and edge copies publish no
fact in this slice. In particular, the last overlapping word of a pointer copy
is not an eligible two-byte temporary.

### Consumers

After their existing complete preflight, these sites may omit their initial
stack LDA when its typed source matches the fact:

| Consumer | Required restriction |
| --- | --- |
| `word_binary` | Actual left operand is the same two-byte temporary. Check before CLC/SEC; retain right operand access and arithmetic order. |
| `branch_on_word` | Actual selected left operand matches, including the existing Gt/Le swap. Carry identity together with the checked operand through that swap. Both materialized and fused comparisons qualify. |
| `word_return` | A16 ABI result and matching temporary. Keep the full existing teardown and RTL. |
| Ordinary two-byte MIR `Store` | Source is the matching temporary; destination uses a direct, nonindexed address with no emitted address setup. Keep the original store exactly, then clear the fact. |

Direct Store destinations may be frame objects, parameters, absolute addresses
or resolved symbols, with the existing nonvolatile two-byte transfer semantics.
Pointer/index preparation and volatile transfers retain their full original
sequences. Immediate operands and direct parameter operands always retain their
loads, even if their bits happen to equal A. All edge paths, including the new
single-word path, clear facts and retain their current complete instruction
sequences. An arithmetic producer may immediately become another arithmetic
consumer and publish its new result, allowing straight-line chains.

### Resident-word proof and invalidation

Introduce one optional private Builder fact containing:

- `TempId` and its exact two-byte stack slot; never just a displacement;
- the current S-relative frame displacement state (`delta`, initially require
  zero for eligibility);
- known A16 and the positive proof that N/Z describe that complete word;
- an emission cursor identifying the byte end and label generation at which
  the producer finished.

Provide a narrow encoder query for width/cursor, retaining its existing width
semantics. Every emitted instruction advances the byte cursor, and every
`mark`, including a label at the same byte offset, changes the label generation.
A fact is usable only at exactly its saved cursor, in the same straight-line
region, with the same frame state and known A16. A no-op `a16()` may preserve it;
an actual REP/SEP may not. Checking the cursor makes an unmodelled instruction
invalidate forwarding even if a new selector path forgets a specific clobber.

At MIR dispatch, pass the incoming fact only to the whitelisted consumer paths;
clear it by default for everything else, including zero-byte unsupported paths.
Publish a new fact only after a whitelisted producer has successfully emitted
its complete sequence. Labels, conditional/unconditional transfers, all edges,
call setup, direct/indirect/runtime calls, helpers, stack guards, pushes/pulls,
reserve/release and any S movement discard it. Calls may clobber A/X/Y, flags
and all 64 scratch bytes; no pre-call fact or call result survives as knowledge.

All intervening stores invalidate the old fact, even disjoint ones; an eligible
producer's final private store establishes a new fact. This conservative rule
covers partial writes and reused stack slots without an alias analysis. A new
temporary occupying the same physical bytes does not match an old `TempId`.
Require the current checked allocation to agree with both identities/ranges.

Keep the existing distinction between unsupported fallback and malformed MIR.
Resolve and validate operands, result homes, widths, and both accessed bytes
before changing code or facts. Failed/fallback selection must not leave a
partially updated fact. Existing stack displacement 254/255 and overflow
diagnostics remain in force, even when the load would otherwise be omitted.

### Flags and memory argument

LDA16 replaces N/Z, but preserves C/V. A16 LDA and binary ADC/SBC produce N/Z
for the complete result, and STA preserves them. With the saved-cursor check,
the removed reload therefore has no CPU-state effect. The ABI's decimal-clear
contract remains a prerequisite for native arithmetic.

Unchanged A alone is insufficient: CMP, LDY, transfers and many other
instructions can change flags. Do not add a flags-dead exception for CMP/ADC
consumers in this slice; a missing N/Z proof retains LDA even if a later
instruction would overwrite flags. Likewise, do not preserve a word fact
through A8 or attempt to reconstruct the hidden accumulator byte.

Only non-addressable compiler temporary reads disappear. Source loads, source
stores, volatile byte accesses and their order/width stay identical. No value
loses its stack home or becomes dependent on surviving a call. Asynchronous
preemption can interrupt between the producer store and the consumer: the
existing bridge must restore full A/P and all other CPU state before resuming.
Keep task/IRQ DP isolation and interrupt reserves unchanged.

## Test and measurement infrastructure

Several current tests identify a word operation by an initial LDA. Removing it
must not silently drop the operation from coverage:

- [`support/comparison.rs`](../tools/native65816-runtime-tests/tests/support/comparison.rs)
  and `compare_branch.rs` recognize LDA/CMP windows and count fused branches.
- [`word_returns.rs`](../tools/native65816-runtime-tests/tests/word_returns.rs)
  and `preemption.rs` anchor return tails on LDA; forwarded tails begin at the
  first teardown instruction. They retain a nonzero frame because the private
  temporary keeps its home. Existing zero-frame return loads remain unchanged.
- `word_arithmetic.rs`, `word_comparisons.rs`, `o65.rs`, `support/context.rs`
  and `code_quality.rs` contain exact bytes, traffic, budgets or window consumers.

Extend the test model to distinguish a loaded operand from a resident operand,
with an optional load PC and an explicit first reached consumer PC. A bare CMP,
CLC/SEC or TAY is not evidence of forwarding. Ground resident windows in a
test-only index of verified MIR producer/consumer identities, checked homes,
labels/fixups, and validated final instruction boundaries. Prove the producer
ends in the retained private store and is adjacent to the consumer in its
straight-line region. Reject missing evidence, wrong width/home/operand,
intervening labels/instructions, malformed tails and inconsistent targets.

Use the existing typed edge-index approach as a model, without changing edge
decoding or weakening its checks. Keep this index outside the serialized image
and o65 formats and outside CPU execution. For saved corpus images, independently
prepare their recorded source/mode/layout, require identical image bytes, and
then index the artifact. For o65, rebase routine-relative evidence after
serialization/relocation and validate the actual bytes and targets; execute
the loaded bytes after discarding compilation objects.

Count a forwarding once at its first executed consumer instruction. Add
Action-only `forwarded_word_loads` and per-site metrics; do not count a removed
instruction or infer success from an absent opcode. Preserve semantic fusion,
word-edge and direct-edge counts, mapping moved sites rather than dropping them.
Expected results and execution counts must come from independent source/vector
semantics or hand-constructed fixtures, not that index or the new emitter.

Add a dedicated mutually exclusive `--forwarded-word-load-counts` mode to
[`delta.py`](../tools/compare65816/delta.py); do not loosen its strict default or
reuse an earlier stack-write exception. For independently predicted executed
count C, require exactly `-C` instructions, `-5C` cycles and `-2C` stack reads;
all stack writes and DP accesses stay equal. Static size falls by exactly two
bytes per declared site. Missing, duplicate, extra and external-compiler count
entries fail. Unselected builds/records remain unchanged except zero-valued new
Action metrics and necessary address remapping within changed builds.

Add an instruction-stream checker proving only declared `LDA d,S` instructions
and required address fixups differ. Check labels, relocation targets, code
ranges and routine identities when an entry points at an omitted load: it must
now resolve to the intended surviving instruction. The first slice must not
remove a load reached independently through a label. Preserve all unaffected
artifacts and vbcc records; normalize only the existing exact vasm source-path
header difference. Keep the known optimized vbcc unlink failure visible in both
host runs. Do not classify it as a passing compiler output.

## Regression and qualification matrix

| Area | Required evidence |
| --- | --- |
| Positive selection | Direct word Load and ADD/SUB producers into each allowed consumer; arithmetic chains; signed/unsigned word bit patterns; selected compare operand after Gt/Le swapping; both materialized and fused comparisons. |
| Negative selection | Wrong TempId sharing a slot, wrong range/width, partial overwrite, unrelated A value, intervening store, CMP/LDY changing only flags, labels/joins/backedges, A8/unknown mode, A8-to-A16 transitions, constants, DP and wide/overlapping pointer transfers. |
| Validation | Last valid word displacement 254, rejected 255, transient S and arithmetic overflow, missing/malformed homes; fallback/errors preserve preflight behavior and cannot leak a fact. |
| Machine state | Independent ca65 original/forwarded snippets; zero, sign-bit, carry/borrow and overflow boundaries; compare A/N/Z/C/V/X/Y/S/D/DBR/I at the consumer boundary and after execution; five-cycle and two-read saving per removed load. |
| Storage/aliasing | Exact retained stores, frame/staging canaries, reused homes and mixed widths; an addressable local changed through an alias must be reloaded; source/volatile traces unchanged, including bank crossings. |
| Calls/helpers | Live words across direct, indirect, recursive and runtime calls, with assembly clobbering A/X/Y and all scratch; outgoing reservations and result marshalling remain barriers; helper/aggregate-copy paths cannot retain stale facts. |
| Control flow | Both branch arms and truth outcomes, repeated conditions, block parameters and same-target edges; direct and cyclic edge sequences remain intact; no fact flows from emitted fallthrough into a join. |
| Preemption | IRQ in both task domains at producer store, every surviving consumer boundary, live carry/compare flags and return teardown; full restored CPU/frame checks against an uninterrupted reference; both existing seeded IRQ/NMI schedules. |
| Relocation/guards | Serialized raw/optimized o65 at $100000 and $600000 with moved data/imports; resident comparisons/returns and calls; existing stack-floor/ceiling/underflow failures and raw fault state. |

Execute serialized raw and optimized output with both incoming I states. Keep
the general preemption, cyclic-copy, direct-copy, fallback comparison and
zero-frame return probes, even when their PCs or window shapes move. Coverage
counts may change with emitted instructions; retain semantic coverage and
explicit resident-window counts instead of lowering assertions to make tests pass.

## Commit-sized implementation sequence

1. **Baseline and independent probes.** Freeze full-corpus sites/count formulas
   and baseline hashes before changing emission. Add `accumulator_forwarding`
   runtime fixtures, ca65 state/traffic oracles and typed resident-window support.
   Keep current-emitter execution expectations until the next commit, and verify
   unchanged artifacts. Include alias/call/flags/width/slot-reuse negatives.
2. **Checked adjacent-word forwarding.** Add the private fact/cursor checks and
   selected producer/consumer hooks, with focused compiler tests. Enable the
   new expectations and update affected exact-byte/traffic tests. Update the
   [emission contract](MIR65816_EMISSION_CONTRACT.md). Keep allocation, stores,
   edge selection and public formats unchanged. Commit after focused raw/optimized
   machine-code execution passes.
3. **Qualification and measurements.** Complete IRQ/NMI and relocated o65 probes,
   strict delta/listing tooling and full corpus counts. Run final native suites
   in both host builds, compare saved artifact hashes, save before/after results
   and a qualification JSON, and update the roadmap/results links. Commit the
   completed evidence without including unrelated local changes.

Suggested compiler checks once implementation exists:

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract \
  --test mir65816_emission --test mir65816_o65 \
  --test actionc_65816_cli --test actionc_65816_o65_cli
```

Use the [qualification runner](../tools/native65816-runtime-tests/README.md),
never bare cargo against the native runtime workspace. The first target below
is to be added by this plan:

```sh
python3 tools/native65816-runtime-tests/qualify.py \
  --test accumulator_forwarding --test word_arithmetic --test word_returns \
  --test word_comparisons --test compare_branch --test word_edges
python3 tools/native65816-runtime-tests/qualify.py --test preemption --test o65
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

Finish with the [comparison build/run/report workflow](../tools/compare65816/README.md)
in a new `target/local-accumulator-forwarding-after` directory, including
`build.py --verify-crlf`, both separately invoked external host runs, the new
delta/checker and their negative tests. Normalize only newline-insensitive host
fixture text; verify LF/CRLF through any changed instrumentation path. Format
only changed Rust files with `rustfmt --edition 2024 --config skip_children=true`.

No semantic/NIR boundary change is planned, so the contributor-mandated NIR
snapshot/sweep/full-root suite is required only if implementation crosses that
boundary. Avoid rerunning passing suites unless later changes warrant it.

Completion requires every predicted reload removal and exact traffic/cycle
delta to be accounted for, all source behavior and ABI/guard/storage contracts
preserved, full native debug/release qualification, matching artifacts and the
known external failure reported accurately. These are VM results; board and
Exec816 loader qualification remain separate. Broader forwarding across safe
instructions or edges can be proposed from the resulting measurements afterward.
