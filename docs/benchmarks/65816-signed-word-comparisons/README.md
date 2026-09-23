# Native signed word comparisons

Implemented on 2026-09-23: signed 16-bit `<`, `<=`, `>` and `>=` now use A16
subtraction and overflow correction, with compact Boolean materialization or
adjacent sole-use branch fusion. Raw and optimized emission use the same rule.
Optimized Dijkstra shrinks **5,779→5,290 bytes (8.46%)**, including
**2,132→1,887 bytes in `Find`**. The small corpus and frozen Exec shell remain
byte-identical. ABI v1, image v3, o65, frame/home allocation and all guards are
unchanged.

The [plan](../../MIR65816_SIGNED_WORD_COMPARISONS_PLAN.md) and
[planning snapshot](planning.json) remain the original scope and forecasts.
[baseline.json](baseline.json) records fallback execution and the physical homes
of all eight Dijkstra candidates. [forms.json](forms.json) records the typed
instruction slice with unchanged images; [selection.json](selection.json)
records the selector's focused qualification. Implementation commits are
`bfa740b0` (baseline), `18fd7bd4` (typed forms) and `7afa719f` (selection).

## Selection contract

The existing checked word-condition preflight admits captured stack words,
parameters, U8/U16 literals and already valid scalar DP word homes. It checks
both inputs and the exact BYTE result home before emitting anything, even when
fusion omits the Boolean write. Scalar DP admission remains unchanged.

SEC/SBC establishes `N xor V`. An ordinary typed long BVC skips EOR #$8000 when
there is no overflow. Lt/Ge subtract left minus right and select BMI/BPL; Gt/Le
swap captured operands and use BMI/BPL. Corrected Z is never used: unequal
`32767,-1` can correct to an accumulator of zero. The correction join retains
conservative value facts and the existing operation barrier. Only the final
BMI/BPL is a relaxable MIR dispatch; internal BVC retains its inverse BVS/JML
encoding. Equality and unsigned word comparison sequences are unchanged.

See the [emission contract](../../MIR65816_EMISSION_CONTRACT.md). No new DP
reservation, helper, push, ABI rule, forwarding opportunity or optimizer pass
was added. Signed BYTE ordering, four-byte comparisons, pointer ordering and
broader branch relaxation remain outside this slice.

## Measured code and traffic

[Probe deltas](probe-deltas.json) compare the same runtime-input sources and
routine contracts, with exact guard-pattern checks. Routine sizes include entry
guards, parameter captures and return code:

| Probe | Raw before → after | Optimized before → after | Frame, unchanged |
| --- | ---: | ---: | --- |
| Each returned predicate | 157 → 123 | 157 → 123 | 6 bytes |
| Each branch-only predicate | 202 → 140 | 202 → 140 | 6 bytes |
| Immediate `a < 0` | 161 → 131 | 153 → 120 | 6 / 4 bytes |

Both ceilings (130 returned, 150 branch) are met. The branch probe is slightly
larger than the plan's 134–138-byte forecast because conservative correction
joins require an A16 request before the false edge. Both branches preserve
A16 successor state; no mode-proof relaxation was added to meet a forecast.

The consumer matrix includes all four predicates as values and branches,
returned/stored/passed/reused Booleans and mutable parameters. Counters below
include its caller and all routines, for inputs `32767,-1`:

| Mode | Code bytes | Cycles | Stack reads / writes | DP reads / writes |
| --- | --- | --- | --- | --- |
| Raw | 3,037 → 2,471 | 3,132 → 2,955 | 124 / 121 → 146 / 117 | 209 / 210 → 196 / 197 |
| Optimized | 3,019 → 2,451 | 3,099 → 2,916 | 122 / 115 → 142 / 111 | 209 / 210 → 196 / 197 |

Whole-word reads increase private input traffic relative to the former high-byte
early exit. Exact executed windows read both captured words, use no DP scratch,
and write only the materialized BYTE; fused windows omit that write and reload.
Original volatile/aliased reads and bank-crossing accesses remain ordered.
The matrix retains all seven guards; the isolated size probes retain all ten.

## Dijkstra and equality controls

[Dijkstra sizes](dijkstra-sizes.json) and the
[per-routine table](dijkstra-routine-sizes.csv) attribute all savings:

| Routine | Raw bytes saved | Optimized bytes saved |
| --- | ---: | ---: |
| Init | 124 | 122 |
| Enqueue | 61 | 61 |
| Find | 244 | 245 |
| Benchmark | 58 | 61 |
| Module | **487** | **489** |

All 22 guards and all routine/frame/home contracts match baseline. No other
routine grows or changes size. vbcc remains 1,697 / 1,477 bytes; this slice does
not close the remaining address-generation and calling-convention gap. The
original Dijkstra fixtures and matched DIV/MOD driver adaptation are unchanged.

The [full Dijkstra execution](dijkstra-execution.json) passes all 33 cases in
both compiler modes and both incoming I states: 132 paired records / 264
executions across Action and vbcc. The release VM harness checks the emitted
artifacts; both host profiles are covered by the full native suite described below.
Original benchmark cycles fall **1,971,316,896→1,817,441,150 raw (7.81%)** and
**1,841,244,249→1,690,355,361 optimized (8.19%)**. Stack peaks remain 96 / 86;
all case peaks and vbcc counters match baseline. Stack and DP traffic decrease
for this workload. Guard execution remains 2,877,984 cycles in each Action mode.
The [complete counters](dijkstra-results.csv) and
[routine profiles](dijkstra-routine-profile.csv) preserve the attribution.

All [28 Action corpus images](corpus-images.json) match baseline exactly. The
known optimized vbcc `unlink` vector-0 failure remains reported; it is not an
expected-success exemption. Debug and release comparison commands both return
101 after retaining all 264 records, including both incoming I states. All 132
Action records pass.

[Exec controls](exec-control.json) use the isolated frozen `8e1ff57` source,
original dirty version banner, eight-task shell, console and DOS configuration.
Both JSON images, XEX files and generated sources are byte-identical:

| Mode | Executable bytes | XEX bytes | Guards |
| --- | ---: | ---: | ---: |
| Raw | 533,672 | 550,794 | 2,287 |
| Optimized | 502,845 | 519,393 | 2,279 |

This workload has no signed word ordering sites, so no size saving was promised.
All 547 routine contracts and bank-zero budgets are unchanged. Live Exec sources
and compiler pin were untouched. These checks do not claim hosted Exec execution
qualification.

## Qualification

The [qualification record](../../abi/action65816-signed-word-comparisons-qualification.json)
records **150 native VM tests in debug and 150 in release**. Their source hashes
and all 736 artifact hashes match. Root checks pass **193 MIR65816 unit tests**
with state-proof enabled, **61 integration/CLI/o65 tests**, and **8 decoder tests**.
Four manual native tests and one external root inventory are ignored by ordinary
suites; corpus/Dijkstra execution and the Dijkstra home inventory are invoked
separately. Unrelated 6502/68k suites and repository-wide NIR checks were not run.

Signed runtime tests cover the eleven-value boundary cross-product plus 128
seeded ordered pairs, all four predicates in both modes and incoming I states.
Independent ca65/VM probes vary incoming C/V/I and execute both overflow paths,
including corrected-zero/non-equal subtraction. Typed effect, flag-liveness,
tracker and replay tests cover SEC→SBC carry, SBC→BVC overflow, and sign through
the correction join. Exact linked spans check operands, correction targets,
Boolean arms and final dispatches independently of the existing CMP decoder.

Further tests cover unsupported/malformed homes, result/input overlap,
nonzero stack delta, parameters and DP word operands, fusion exclusions,
nonempty and same-target parallel edges, backedges, full assembly call clobbers,
volatile/bank-crossing captures and two serialized o65 placements. Signed task
probes restore full CPU/frame/domain state at **590 reached instruction/status/
domain sites per mode**, separately with IRQ and NMI, plus masked NMI and two
seeded schedules. Existing stack-fault/headroom suites remain green.

Large machine images, listings and run manifests are retained under ignored
`target/signed-word-comparisons` and the native suite's `target/qualification`.
The committed JSON/CSV evidence records hashes, inputs and compact counters.
Reproduction commands are in the plan and [comparison tool instructions](../../../tools/compare65816/README.md).
