# Remaining local jump and branch relaxation

All typed internal conditionals now participate in the existing checked layout
pass, including comparisons, casts, shifts, signed overflow correction and
helper loops. Local JMLs select BRA or BRL when their signed displacement fits.
The common fixed point includes each candidate's own shrink. Longer transfers
retain their original forms; calls and external fault exits remain unchanged.
See the [plan](../../MIR65816_LOCAL_RELAXATION_PLAN.md) and
[emission contract](../../MIR65816_EMISSION_CONTRACT.md).

## Frozen Exec measurement

Baseline: compiler `7a627e58`, including native-width call arguments and result
capture. Workload: frozen Exec `c3500c8`, eight tasks, console, MyDOS and all
mandatory stack checks.

| Measurement | Raw before → after | Optimized before → after |
| --- | ---: | ---: |
| Compiler routine bytes | 441,726 → 423,824 | 409,508 → 392,251 |
| Conditional bytes saved / sites | 7,236 / 1,809 | 6,812 / 1,703 |
| BRA bytes saved / sites | 9,768 / 4,884 | 9,640 / 4,820 |
| BRL bytes saved / sites | 898 / 898 | 805 / 805 |
| Total code bytes saved | **17,902** | **17,257** |
| All executable bytes | 449,291 → 431,389 | 417,073 → **399,816** |
| XEX file bytes | 465,093 → 446,867 | 432,299 → **414,736** |
| Guards retained | 2,328 | 2,320 |

Optimized executable size falls **4.14%**. All admitted local conditionals and
jumps fit relative forms in this workload. The compiler retains long fallbacks
for other routines; conditional compounds without short reach remain six bytes.
Guard-local JMLs contribute two bytes saved per guard, included in the BRA total.

All 551 routine contracts match in each mode, including frames, homes, arguments,
results and stack costs. The audit checks every shortened transfer, exact label
identity and mapping, retained fixups, MIR spans, PER continuations and all bytes
outside the shortened sites after relocation normalization. Guards preserve
counts, amounts and order. Full packaged compiler segments equal the inventory
images. Generated sources, platform/ABI inputs and bank-zero budgets match.
Added stack, DP and bank-zero reservation: **0 bytes**.
See [totals and hashes](exec-results.json), [sites](exec-sites.csv) and
[routine sizes](exec-routines.csv).

## Corpus and runtime measurements

All 28 Action corpus images shrink. Per-routine savings range from the two-byte
guard jump to 54 bytes in `wide_shift`; each instruction stream retains its
original operations and memory accesses. All 132 Action records pass in both debug and release,
covering both incoming I states (264 executions per host mode). Both host
modes produce identical records. Stack/DP traffic and peak stack
use match the preceding call-copy baseline; cycle counts never increase.
Optimized `wide_shift` drops 643→611 cycles and sum-loop vector 4 drops
2,453→2,421. See [sizes](corpus-sizes.json), [routine deltas](corpus-routines.csv),
[execution deltas](corpus-execution.csv) and [execution status](corpus-execution.json).

The comparison command retains exit 101 because the existing optimized vbcc
`unlink` vector 0 fails. Its result is unchanged; all Action records pass.

Dijkstra shrinks **5,338→5,213 raw** and **4,751→4,625 optimized** code bytes,
preserving all 22 guards and routine contracts. See [sizes](dijkstra-sizes.json)
and [routine deltas](dijkstra-routines.csv). The targeted `original-0-50` graph
passes for both compilers, target modes and incoming I states; the original full
benchmark and remaining vectors were not rerun. See
[execution scope/results](dijkstra-execution.json) and [cycle deltas](dijkstra-deltas.json).
Raw/optimized cycle counts fall by 781,574 / 781,673 with unchanged stack/DP
traffic and peak stack use. Corpus and Dijkstra generators
both verify actual LF/CRLF compilation equality.

## Qualification

**210 native library tests** pass (one opt-in test ignored), along with **69
integration/CLI checks**, three guard-inventory checks and **182 runtime tests in
each of debug and release** (four external inventory/comparison tests ignored).
All **496 input hashes and 1,033 artifact hashes** agree between runtime runs.
The [qualification record](qualification.json) binds source inputs, compiler
binaries, measurement tools and the pinned VM/patch. The focused 29-test rerun
also agrees with the final runtime artifacts.

Unit coverage includes signed-byte/word limits in both directions, candidate
forward shrink, conditional/jump cascades including BRL→BRA, deterministic
layout, bank/address bounds, corruption, overlapping relocations and interior
metadata rejection. Selected actions, effects, environment facts and CFG edges
remain identical after remapping. Runtime coverage checks emitted transfers
against ca65 at two origins with every status byte, all conditional flags,
image/o65 placement, indirect calls/PER, helper loops, and guard success/fault
paths. Guard IRQ/NMI checks include the local BRA fault path; the complete
preemption suite covers both task domains and seeded schedules.

The reviewed emission snapshot changes 24 routine records, saving 188 bytes
with identical complete frame records. This is an intentional machine encoding
change, with rebased labels, fixups and spans; no NIR or printer contract changes.
See [snapshot deltas](snapshot-changes.json).

```sh
cargo test --lib mir65816:: --features native65816-state-proof
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission --test mir65816_state_boundary --test mir65816_o65 --test mir65816_arithmetic --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -m unittest discover -s tools/compare65816 -p 'test_guard_ranges.py'
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

Local scripts and full inventories are retained under `target/local-relax/`.
Packaged Exec uses the existing adapter that removes only the obsolete
`stack_checks:true` layout key; mandatory guards remain enabled. Live Exec
sources and compiler pin are unchanged. Hosted adoption remains separate.
