# Emitted native 65816 execution

This isolated workspace tests the
[native emitter](../../docs/MIR65816_EMISSION_CONTRACT.md) and
[context bridge](../../docs/MIR65816_CONTEXT_INTERFACE.md). It loads serialized
JSON images and experimental o65 applications and executes their bytes on the
VM's independent native 24-bit bus.
The compiler does not depend on the VM.

An opt-in [actionc/vbcc code-quality comparison](../compare65816/README.md)
executes paired C and Action! kernels from external build artifacts. Its
`code_quality` test is ignored by default; it reports incorrect external compiler
outputs as failures and saves the complete measurements for analysis.

## Reproduce qualification

Install Rust, Python 3.12+, git, ca65 and ld65. From the actionc repository root:

```sh
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
python3 tools/native65816-runtime-tests/qualify.py --cpu
python3 tools/native65816-runtime-tests/qualify.py --test comma_groups --test memory --test interop
python3 tools/native65816-runtime-tests/qualify.py --test o65
```

The runner uses VM base `56ddc5c5de41f0e7294e87c440869550eaf53292` plus
[`vm-status-timing.patch`](vm-status-timing.patch), exported from actionc-vm
commit `da81c1e`. REP/SEP/RTI timing corrections are necessary for asynchronous
qualification. It archives the pinned commit from a sibling VM repo when
available, otherwise fetches that exact base into a private temporary checkout.
It checks/applies the patch and verifies cached CPU file hashes on reuse.
It never edits the sibling VM's working tree.

For native qualification, the runner hashes compiler/fixture/tool inputs before
starting Cargo and rejects changes, additions or deletions before publishing the
manifest. Keep those inputs stable for the full run. The provenance guard's five
mutation controls run with `python3 -B tools/native65816-runtime-tests/test_qualify.py`.

Use this runner instead of bare `cargo test`: the lockfile expects the corrected
CPU path override supplied by the runner. The published VM dependency remains
pinned; the independent VM commit need not already be published. `--prepare-only`
prints the qualified checkout, `--cpu --release` checks its CPU release build,
and other arguments are forwarded to `cargo test`, for example
`--test preemption -- --nocapture` to print instruction-address counts.

The assembler is invoked directly; missing tools fail the tests. No Atari ROM,
OS, device intercept or host scheduler participates. The earlier
`vm65816-runtime-tests` workspace remains the jgenesis comparison experiment.

## Arithmetic helpers

`arithmetic_helpers.rs` checks all 256² unsigned byte pairs, boundary cross
products and deterministic random wider inputs against independent host
integer arithmetic. Inputs are written after compilation. It also exercises
signed MIN/-1, zero faults, all unsigned power-of-two reductions, side effects,
compound and bank-crossing stores, exact ca65 argument/result lanes, helper
entry stack bounds and rebased serialized o65 files. `preemption.rs` reenters
the same helper bodies from two tasks and the IRQ dispatcher, checking every
reached helper instruction and seeded IRQ/NMI schedules. NMI calls no helper.

`arithmetic-metrics.json` records emitted body bytes, zero-frame stack costs,
DP usage, body cycles and cycles including the independent assembly caller.
`arithmetic-range-*.json` records observed kernel timing ranges and maximizing
inputs over the numerical corpus. Profile v2 and
image v4 are used only when the raw arithmetic-fault dependency survives.

## Corpus

| Target | Tests | Coverage |
| --- | ---: | --- |
| `acyclic_edges` | 2 | Independent direct/reordered word-copy sequences, self/repeated sources, full A/status preservation, staging canaries and unsafe schedule rejection. |
| `state_tracking` | 4 | Independent ca65 bytes and VM facts, both widths, hidden B, full status masks, homes/NZ, loop joins, direct/indirect stack phases, PER continuations after branch shortening and o65 rebasing; trace-on/off output equality. |
| `control_flow` | 5 | Independent REP/JML omission, all short predicates, emitted BRA/BRL/JML reach limits against ca65 with every status byte, page/bank boundaries, edge copies and two o65 placements. |
| `guard_branches` | 3 | Exact compact guards against ca65, floor/ceiling/underflow paths, moved fault exits, and IRQ/NMI restoration at every reached guard instruction in both task domains, including the local BRA fault arm. |
| `accumulator_forwarding` | 4 | Frozen full-corpus sites, typed resident-word evidence, independent ca65 register/flag/traffic equivalence, all four consumers, volatile traces, LF/CRLF and evidence rejection. |
| `arithmetic` | 1 | 72 boundary executions across BYTE/CARD/INT/SIZE/LONGCARD/LONGINT, checked against host arithmetic. |
| `word_arithmetic` | 4 | Independent ca65 encodings, CARD/INT boundary cross-products, operand order and carry chains, volatile/aliased bank-crossing memory, live words across calls that clobber A/X/Y and all DP scratch. |
| `word_returns` | 3 | Independent callers, signed/unsigned bits, mixed result lanes, zero/nonzero frames, clobbering calls, volatile/aliased bank-crossing loads, exact return-tail reads and no DP traffic. |
| `byte_returns` | 2 | All 256 constants with full A16 zero extension, both I states and compiler modes, fixed images and two o65 placements, framed/branch returns, captured/direct/indirect fallbacks, independent ca65 tails, exact RTL reads and no tail writes or DP traffic. |
| `word_comparisons` | 5 | All signed/unsigned word relations, stored/returned Boolean bytes, canaries, casts and operand orders, clobbering calls, exact volatile/stack traces, independent CMP encodings, and code/cycle/stack budgets. |
| `long_equality` | 5 | LONGCARD/LONGINT Eq/Ne and zero tests, all individual bits and boundary/random pairs, every Boolean consumer, mutable parameters, unchanged volatile/bank-crossing loads, full call clobbers, exact ca65 encodings and private word traffic, same-target edges/backedges, and two o65 placements. |
| `call_copies` | 2 | Native argument/result widths, independent assembly callees clobbering DP, exact ca65 capture suffixes and owned-byte writes, discarded results, direct/indirect calls, mutable parameters, LF/CRLF parsing, and two o65 placements for all scalar types. |
| `empty_edges` | 3 | Compact Goto/Fallthrough/ordinary/fused transfers, both arms and widths, exact cycles, ca65 encodings, full register/flag preservation, no data traffic and decoder rejection cases. |
| `word_edges` | 5 | Direct single-word and staged cyclic copies, typed edge-site evidence, repeated/unused arguments, live-ins, fallback, mutable parameters, ca65 encodings, overlapping homes, exact cycles/flags/traces and staging canaries. |
| `compare_branch` | 6 | Boundary relations and fallbacks, exact fused source traffic, reused conditions, nonempty same-target edges/backedges, volatile/alias/call barriers and decoder rejection cases. |
| `execution` | 5 | Recursion, mutable parameters, loop edges, local addresses/descriptors, record strides, banked code/data and exact volatile byte access. |
| `interop` | 1 | Hand-packed mixed ABI arguments and zero-argument padding, calls both ways, A/X results, unused bits and all 64 scratch bytes clobbered; both I states. |
| `comma_groups` | 1 | Scalar comma groups before contextual types in parameters and fields; mixed-width values, a bank-crossing record, LF/CRLF and both I states. |
| `indirect` | 3 | Targets `$050000`/`$06FFFF`, all scalar results, assembly arguments and six-byte transfer overflow checks. |
| `contexts` | 6 | First-task bytes, yield/exit, full register/flag restoration in every M/X mode, invalid COP/domain paths and NMI through IRQ transition windows. |
| `pointer_allocation` | 2 | Generated/reference unlink; differential stack/DP swaps, chains, field layouts and pressure; mixed arguments, bank crossings, aliasing, exact traces and LF/CRLF. |
| `pointer_preemption` | 2 | Both tasks and IRQ dispatch use the same three-slot leaf; IRQ at 164 raw / 100 optimized task/instruction sites, plus seeded IRQ/NMI. |
| `memory` | 8 | Pointer results and bank-crossing unlink, field offsets around the Y limit, exact volatile three-byte traces, absolute array indices, logical shifts, record/overlap copies and signed/wide pointer offsets. |
| `effects` | 1 | Nested IRQ tokens, pending IRQ, protected multiword writes, polling/reloads and exact volatile traces under optimization. |
| `preemption` | 19 | Two live recursive contexts and shared memory helpers; every reached enabled instruction address, arithmetic/return/comparison windows, zero-frame returns, BYTE constants, long Eq/Ne and zero tests, native call arguments and A/X results, comparison flag outcomes, direct-copy and forwarded live A in both tasks, and seeded IRQ/NMI schedules. |
| `stack_allocation` | 3 | Measured scalar/loop/recursive/indirect call chains with stack ceilings; a long sequence beyond the old allocation limit; live wide values across direct/indirect assembly calls clobbering all DP scratch and A/X/Y. |
| `stack_faults` | 2 | Floor/ceiling/underflow and call transients, with raw fault A/X/S state verified before prohibited writes. |
| `o65` | 12 | Serialized files loaded at two placements; bank carries, BSS, aliases, initialized split/full addresses, moved imports/faults, mixed ABI, resident comparisons/returns, direct/staged edges, multi-bank code and preempted tasks. |

Both raw and optimized NIR are covered. The original **33 tests passed in debug
and release** on 2026-09-17. The later `comma_groups` regression, plus the eight
`memory` tests and `interop`, pass in debug on 2026-09-20; this targeted run does
not claim a new full-suite or release qualification. Local tools: Rust 1.95.0,
ca65/ld65 2.18, macOS ARM64.
Optimized unlink uses 129 bytes, 189 VM cycles and no frame, including checked
entry and RTL; the independent reference uses 127 bytes and 200 cycles. The
[qualification record](../../docs/abi/action65816-pointer-allocation-qualification.json)
binds the 2026-09-17 results to compiler/fixture hashes and context artifacts. Images
use transport v3 and retain physical ABI v1.

The [stack allocation investigation](../../docs/MIR65816_TEMPORARY_ALLOCATION.md)
records the `90bd73e` baseline and reductions from CFG-aware temporary reuse.
Its [2026-09-21 qualification](../../docs/abi/action65816-stack-allocation-qualification.json)
passes all 37 native tests in debug and release, with unchanged ABI and guards.
Run `--test stack_allocation -- --nocapture` to print emitted code sizes, VM
cycles, frames and the observed stack use across complete call chains. Inputs
are supplied after compilation, and both raw and optimized images are executed.

The [o65 qualification](../../docs/abi/action65816-o65-qualification.json) passes
all 44 native tests in debug and release on 2026-09-21. The o65 adapter consumes
only serialized file bytes, placement and provider contracts; compiler results
are discarded before loading. It preserves the JSON harness's guards and bus
permissions. Code is placed at `$100000` and `$600000`, with independently moved
data/BSS and helper/fault addresses. The o65 context test uses two seeded IRQ/NMI
schedules and six selected reachable instruction addresses per mode/placement;
the existing exhaustive JSON context tests also remain in the full suite.

The corrected CPU suite passes eight tests in each build mode. See
[initial Exec acceptance](../../docs/MIR65816_EXEC_ACCEPTANCE.md) for G1–G6,
compiler regressions and qualification limits.

The [native word arithmetic qualification](../../docs/abi/action65816-word-arithmetic-qualification.json)
passes all **48 native tests in debug and release** on 2026-09-21; the external
comparison remains a separately invoked test. All 118 saved artifacts match
between host builds. The [results report](../../docs/MIR65816_WORD_ARITHMETIC.md)
records measurements and the unchanged public ABI, frames, and guards.

The [native word return qualification](../../docs/abi/action65816-word-returns-qualification.json)
passes all **52 native tests in debug and release** on 2026-09-21, with all 136
saved artifacts identical. The [results report](../../docs/MIR65816_WORD_RETURNS.md)
records 63-byte / 71-cycle identity and 74-byte / 98-cycle add/subtract, all with
zero DP scratch traffic and unchanged ABI, stack traffic and guards.

The [native word comparison qualification](../../docs/abi/action65816-word-comparisons-qualification.json)
passes all **59 native tests in debug and release** on 2026-09-21, with all 166
saved artifacts identical. The [results report](../../docs/MIR65816_WORD_COMPARISONS.md)
records 146-byte / 149-cycle maximum and 213-byte / 2,465-cycle optimized sum loop,
with no DP traffic in either. ABI, allocation, stack writes, peaks and guards
are unchanged; four corpus records read two additional private stack bytes.

The [compare-to-branch qualification](../../docs/abi/action65816-compare-branch-qualification.json)
passes **67 native tests in debug and release**. The
[results report](../../docs/MIR65816_COMPARE_BRANCH_FUSION.md) records maximum at
116 bytes / 117 cycles and optimized sum loop at 183 bytes / 2,030 cycles.
Frame allocation, guards and DP traffic are unchanged; each reached fusion
removes one Boolean stack write and reload. Corpus counts are checked against
predeclared predictions in both host modes and both incoming I states.

General preemption now reaches 2,275 raw / 2,117 optimized enabled addresses.
The targeted fused probe covers 152 task/PC sites and 24 window/truth/task
combinations per mode, including immediate/stack sources and nonempty edges.
It checks the full restored register state immediately after IRQ resumes each
instruction, including both CMP flag outcomes in both tasks. Both seeded IRQ/NMI
schedules pass. The separate materialized probe retains its 96 sites.
`fused-task-preemption-*.json` records armed and restored PCs/status, while
`fused-preemption-mir-*.txt` retains the verified nonempty-edge fixture.
`fused-branch-traffic-*.json` separates comparison reads from edge-copy writes.
The new o65 fused-branch probe executes relocated conditional/JML edges at both
placements; its `.o65`, placement and metrics artifacts accompany the existing
materialized comparison probe.

## Interrupt schedules and memory ownership

The BYTE constant return probe in `preemption.rs` checks full IRQ and NMI
restoration at each reached instruction of a zero-frame constant leaf and a
framed conditional return in both task domains. It also runs the existing seeded
IRQ/NMI schedules. Source instrumentation normalizes and checks LF/CRLF inputs.

The long-equality probe exercises both signednesses in two task domains, checking
full resumed state after IRQ and NMI at every reached instruction of materialized
and fused comparisons. Inputs cover both matching halves, low-word mismatch,
high-word mismatch and zero tests. Both seeded IRQ/NMI schedules also run.

The native-call probe adds three- and four-byte arguments and results through
direct and indirect calls, checking full IRQ/NMI restoration at every reached
instruction in both task domains. It includes caller cleanup and partial result
capture, plus seeded schedules and LF/CRLF source instrumentation. The existing
mixed-call padding probe covers the complete outgoing areas and word results.

The two-task fixture uses task domains `$2000`/`$2100`, task stacks
`$4000..$4FFF`/`$5000..$5FFF`, IRQ domain `$2300` and IRQ stack `$6000..$6FFF`.
The assembled bridge starts at `$008000`; emitted code starts at `$018000`,
data at `$120000`, fault handling at `$048000`, and explicit IRQ/NMI/exit
acknowledgements at `$7800..$7803`. Each image/layout records exact extents.
These are test reservations, not an Atari board memory map.

After native word comparisons, the corpus reaches 2,311 raw and 2,153 optimized
distinct enabled instruction addresses (previously 2,332 / 2,180). At each,
a separate run holds IRQ until assembly dispatch
acknowledges it, then checks output, guards and domain storage. Seeded runs use
`0x81620260916` and `0x5eedcafe`; NMI pulses are separated by at least 250 cycles.
The exhaustive test records seven word-arithmetic windows per mode, including
both stack-relative ADC and SBC. Each window is interrupted before carry setup,
before arithmetic, and before storing the result, exercising live A/P restoration.
It also covers seven word-return tails and all 56 tail boundaries in each mode,
including live results in A/Y and stack restoration around TCS. A supplemental
zero-frame leaf tests LDA/RTL in each task, then both seeded IRQ/NMI schedules.
Three selected comparison windows are covered by the exhaustive test. A targeted
probe interrupts all 96 comparison task/PC boundaries per mode and all 16
predicate/truth/task combinations immediately after CMP with live C/Z, then
runs both seeded IRQ/NMI schedules. It checks exact Boolean-derived results
separately for less, greater and equal input pairs.
The pointer fixture additionally injects once per reached `(task domain, PC)`
inside its leaf, so both tasks are checked even when they share instruction
addresses. IRQ dispatch calls that same leaf using the IRQ domain's scratch.
IRQ masking and the bridge's non-nested NMI policy are respected. Each run has a
finite cycle budget; unmapped accesses and code writes fail immediately.

The simpler call probes use an independent caller at `$040000`, a bootstrap
domain at `$002000` and a guarded stack in `$004000..$005FFF`. Assembly layouts
and host expected results are independent of compiler IR/layout helpers.

## Saved artifacts

Successful native runs create separate `target/qualification/run-*/` directories.
The manifest records compiler revision, source/fixture hashes, VM base and patch
hash, assembler/linker/Rust versions, command, seeds and artifact hashes. Context
runs save `.act`, `.a816.json`, `.bridge.bin` and `.layout.json` files. Filtered
runs contain only artifacts produced by the selected tests; they cannot inherit
stale images from an earlier run.

`word-preemption-false.json` and `word-preemption-true.json` record the reached
ADC/SBC windows and the instruction addresses covered by IRQ injection.
`return-preemption-*.json` and `return-zero-preemption-*.json` record word-return
IRQ coverage; `word-return-tail-*.json` records independently executed tail
traffic, restored-stack checks, code sizes and worker cycles.
`comparison-preemption-*.json` and `comparison-task-preemption-*.json` record
comparison coverage, including live-flag outcomes in both tasks.
`comparison-traffic-*.json` records exact executed source reads and Boolean
writes; `comparison-budget-*.json` records worker code/cycles/stack ceilings.

The o65 tests save `.o65`, `.placement.json` and `.metrics.json` artifacts.
Metrics separate machine code, text, data, BSS, descriptor and total file sizes,
and include relocation counts and cycles. Observed stack use is recorded for
the pointer/call and guard-failure probes; `null` means it was not measured.
The qualification record checks identical file hashes across placements and
identical artifacts between debug/release host builds. Decode the standard wire
records with `python3 tools/inspect_o65.py PATH/TO/PROGRAM.o65`.

Inspect an emitted image from the repository root:

```sh
python3 tools/disassemble65816.py PATH/TO/IMAGE.a816.json > image.asm
```

The disassembler supports the qualified emitted instruction subset and rejects
unknown/truncated encodings. Preserve assembler listings for external bridge
code. The [CPU checkpoint](../../docs/MIR65816_CPU_EXECUTION_CHECKPOINT.md) and
[WDC datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf)
state CPU provenance and the hardware boundary. Emulator acceptance is followed
by a custom-board startup/interrupt smoke test.

The [native word edge-copy qualification](../../docs/abi/action65816-word-edges-qualification.json)
passes **71 native tests in debug and release** on 2026-09-21, with **224 identical
saved artifacts**. Run `--test word_edges --test compare_branch --test preemption
--test o65 -- --nocapture` for the focused probes. Targeted IRQ coverage includes
330 task/PC sites (222 word-copy sites) per mode, full register and frame/staging
restoration, overlapping cyclic copies in both domains, six fused windows and
24 post-CMP truth/task combinations. Both seeded IRQ/NMI schedules pass. New
o65 probes execute nonempty edges at both placements. The
[results](../../docs/MIR65816_WORD_EDGE_COPIES.md) record unchanged ABI, stack/DP
traffic, frames and guards, plus strict corpus counts and measured gains.

The [empty-edge qualification](../../docs/abi/action65816-empty-edges-qualification.json)
passes **74 native tests in debug and release**, with **232 identical saved
artifacts**. Focused tests are `--test empty_edges --test compare_branch
--test word_edges --test preemption --test o65`. General IRQ coverage is
2,245 raw / 2,087 optimized enabled instruction addresses. The targeted probe
covers 300 task/PC sites, all 222 word-copy sites and all 24 post-CMP truth/task
combinations per mode; both seeded IRQ/NMI schedules pass. The reduced site
counts reflect removed SEP/REP instructions. Relocated o65 branches require
compact empty edges at both placements. See the
[results](../../docs/MIR65816_EMPTY_EDGES.md) for instruction-stream proof and
unchanged ABI, stack/DP traffic, frames and guard costs.


The [direct single-word edge qualification](../../docs/abi/action65816-single-word-edges-qualification.json)
passes **78 native tests in debug and release**, with **270 identical artifacts**.
The new targeted probe checks all 34 direct-transfer/successor task/PC sites per
mode, including live A, both branch outcomes and complete CPU/frame restoration.
Existing cyclic-copy coverage retains 300 sites, 222 word-copy sites and 24
post-CMP truth/task combinations. Both seeded IRQ/NMI schedules pass. Relocated
direct edges execute ordinary/fused arms and backedges at both placements.

Direct-copy decoding requires a test-only index built from verified MIR and typed
machine fixups; a load/store/jump pattern alone is insufficient. The index never
participates in CPU execution or changes image/o65 formats. Traces require all
four reserved staging bytes to remain untouched by direct copies. See the
[results](../../docs/MIR65816_SINGLE_WORD_EDGE_COPIES.md) for measured traffic
reductions and unchanged ABI, frame maps and guards.

The [local accumulator qualification](../../docs/abi/action65816-accumulator-forwarding-qualification.json)
passes **84 native tests in debug and release**, with **302 identical artifacts**.
Run `--test accumulator_forwarding --test compare_branch --test word_returns
--test preemption --test o65 -- --nocapture` for focused coverage. Resident-word
decoding uses typed MIR identities, exact homes, nonserialized emission spans
and final instruction boundaries. It validates the retained producer store and
adjacent consumer; absent evidence cannot turn a bare CMP or TAY into a match.

The new IRQ probe checks 98 raw / 76 optimized task/PC sites across both domains,
all four consumer kinds, live carry/compare flags and return teardown, with full
CPU/frame restoration. General coverage reaches 2,237 raw / 2,077 optimized
enabled addresses; fused coverage retains 300 / 296 sites, all 222 cyclic-copy
sites and 24 flag outcomes. Both seeded IRQ/NMI schedules pass. Forty new o65
executions reach every forwarding site at both placements, retaining exact
volatile traces. See the [results](../../docs/MIR65816_LOCAL_ACCUMULATOR_FORWARDING.md)
for exact reload savings with unchanged stores, homes, ABI and guards.

The [state-tracker qualification](../../docs/abi/action65816-state-tracker-qualification.json)
retains all 302 previous artifacts and adds 72 trace/code artifacts. Both native
hosts pass 88 tests with the optional corpus test ignored; all 374 artifacts match.
The isolated workspace enables `native65816-state-proof`. Its immutable snapshots
never drive VM execution; ordinary compiler builds leave tracing disabled. Run
`--test state_tracking` for the four focused tests. The separate external corpus
runs retain the known optimized vbcc `unlink` failure while requiring complete
before/after equality for every record.

The [analysis and checked-rewrite qualification](../../docs/MIR65816_ANALYSIS_REWRITE_QUALIFICATION.md)
passes **131 native tests in each of debug, release and isolated CRLF debug**,
with **658 identical artifacts**. It preserves the prior 657 artifacts and adds
the adjacent-rewrite inventory. Run `--test checked_rewrites --test replay` for
the focused migration checks. The full qualification retains IRQ/NMI, helper/call
clobber, alias/volatile, relocated o65 and stack-guard coverage. Host compilation
cost is measured separately from unchanged generated-code quality.

The opt-in [Dijkstra comparison](../../docs/benchmarks/65816-dijkstra/README.md)
executes public Action CLI images and linked vbcc binaries through the same
qualified CPU. Build with `tools/compare65816/dijkstra.py`, then use
`tools/compare65816/run_dijkstra.py target/dijkstra-65816/manifest.json` to verify
artifact hashes and run `--release --test dijkstra -- --include-ignored`.
All 33 reference vectors run in raw/optimized modes with I clear and set. No
interrupts are injected in this measurement; the existing preemption suite owns
IRQ/NMI qualification. Only this affected backend target is needed when changing
the comparison harness.

The [constant-shift and index-scaling qualification](../../docs/benchmarks/65816-constant-shifts/README.md)
adds all-width/count, ca65 word-chain and full 24-bit index tests. Run
`--test constant_shifts --test preemption` for focused coverage. The shift
preemption probe restores IRQ/NMI state at every reached instruction in both
domains, including live residual carry chains and X16 counters. See the linked
qualification record for full debug/release checks and frozen Exec measurements.

The [captured BYTE return qualification](../../docs/benchmarks/65816-captured-byte-returns/README.md)
checks exact one-byte return reads, zero-extended A16, zero/framed and mutable
parameter homes, full scratch clobbers, relocated o65 and IRQ/NMI restoration.
Run `--test captured_byte_returns` and
`--test preemption captured_byte_returns` for the focused probes.
