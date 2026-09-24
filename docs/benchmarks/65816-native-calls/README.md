# Native-width call arguments and result capture

MIR65816 now copies eligible captured arguments and numeric constants using A16
word pairs. Three-byte arguments retain an exact byte tail. A local A8/A16
choice includes mode-switch costs and the width needed for the next transfer
or indirect-target capture, so mixed BYTE/word lists do not grow. Symbolic and
mixed-width operands retain bytewise relocation and extension paths.

After the existing caller cleanup, result capture stores A16 directly for words,
A16 plus X's low byte for three-byte values, and A16/X16 for four-byte values.
BYTE capture writes only its owned byte. There is no DP result staging, fourth
pointer-byte access, overlapping payload store or additional allocation.
Operand/result checks complete before the guard or any call emission.
See the [plan](../../MIR65816_NATIVE_CALL_COPIES_PLAN.md) and
[emission contract](../../MIR65816_EMISSION_CONTRACT.md).

## Frozen Exec measurement

The compiler baseline is `860a02f2`, including native long equality. The workload
remains Exec `c3500c8`, eight tasks, console, MyDOS and stack checks enabled.

| Measurement | Raw before → after | Optimized before → after |
| --- | ---: | ---: |
| Compiler routine bytes | 452,593 → 441,726 | 419,789 → 409,508 |
| Call sites audited / shortened | 1,777 / 1,072 | 1,769 / 1,067 |
| Argument-copy bytes saved | 5,155 | 4,597 |
| Result-capture bytes saved | 5,712 | 5,684 |
| Total code bytes saved | **10,867** | **10,281** |
| All executable bytes | 460,158 → 449,291 | 427,354 → 417,073 |
| XEX file bytes | 476,140 → 465,093 | 442,778 → 432,299 |
| Guards retained | 2,328 | 2,320 |

The optimized reduction is **2.41%** of the previous executable size. Every
call site keeps or reduces its size; no non-call span changes size. There are
no secondary layout or mode-omission savings outside the calls. A captured word
or three-byte result saves seven bytes; a four-byte result saves fourteen.

All 551 routine contracts match in each mode, including frames, temporary homes,
arguments, results and stack costs. The audit checks exact old/new result tails,
unchanged fixup targets, rebasing of retained instructions and metadata, and
guard counts, amounts and order. Full packaged compiler segments match the
inventory images exactly. Generated sources, platform/ABI inputs and bank-zero
budgets match. Added stack, DP and bank-zero reservation: **0 bytes**.
See [totals and hashes](exec-results.json), [call sites](exec-sites.csv) and
[routine sizes](exec-routines.csv).

## Corpus and runtime measurements

Only `direct_calls` and `recursive_sum` change in the small corpus, saving
22 and 11 bytes respectively in each mode. The other 24 Action images are
byte-identical. All routine contracts and guards match the previous baseline.
`direct_calls` falls from 385 to 349 cycles. Recursive vector 3 falls from
3,109 to 2,875 cycles raw and 2,694 to 2,460 optimized. Stack/DP reads and writes
and peak stack use remain equal across all 132 Action records. Both incoming
I states pass, giving 264 Action executions per host mode; debug and release
produce identical records. See [sizes](corpus-sizes.json),
[execution deltas](corpus-execution.csv) and [execution status](corpus-execution.json).

The external comparison command retains exit 101 in both host modes because
the existing optimized vbcc `unlink` vector 0 fails. Its result is unchanged;
all Action records pass. This failure is recorded, not waived.

Dijkstra shrinks from **5,438 → 5,338 raw** and **4,850 → 4,751 optimized** code
bytes, preserving all 22 guards and its routine contracts. The targeted
`original-0-50` graph passes for both compilers, raw/optimized modes and both I
states. The original full benchmark and remaining vectors were not rerun.
See [sizes](dijkstra-sizes.json) and [execution scope/results](dijkstra-execution.json).
Corpus and Dijkstra generators both verify actual LF/CRLF compilation equality.

The independent assembly argument probes retain exactly one write per outgoing
byte, with unchanged holes, tail padding and complete incoming payloads.
Their [before/after records](call-probes.csv) cover direct and indirect calls,
empty lists, mixed widths, pointers, ADDRESS/SIZE and maximum displacements.
Existing probes also cover nested calls, volatile reads and mutable parameters.

## Qualification

**205 native library tests** pass (one opt-in test ignored), along with **69 root
integration/CLI checks** and **181 runtime tests in each of debug and release**
(four external inventory/comparison tests ignored in each). All **496 input
hashes and 1,033 artifact hashes** match between runtime runs. The
[qualification record](qualification.json) binds sources, binaries, tools,
the pinned VM/patch and run manifests.

New unit checks compare the selected encoding against all byte/native choices
for four-argument width sequences, constants/captured homes and both following
widths. They cover preflight failure without partial emission, authoritative
parameter homes, complete stack extents with outgoing deltas, the byte-255 tail
boundary and exact result lanes. Runtime tests use independent ca65 callees
that clobber all 64 DP scratch bytes, check exact result tails and owned-byte
writes with neighbor preservation, and execute discarded results. All scalar
types round-trip through mutable parameters, direct/indirect calls and two o65
placements. Actual LF/CRLF source parsing agrees.

The new IRQ/NMI probe checks every reached task/PC/status site in the call and
echo routines in both task domains, including construction, cleanup and partial
A/X capture. ADDRESS covers 480 raw / 424 optimized sites; LONGCARD covers
460 / 376. Both seeded schedules also pass. The existing mixed-call probe
continues to cover outgoing padding, complete payloads and word results.

The reviewed emission snapshot intentionally changes four routine records:
accumulator-forwarding `Main` saves 30 bytes per mode and recursive-sum `Work`
saves 11. Frame records are identical; byte positions, labels and fixups rebase.
The [snapshot changes](snapshot-changes.json) record this emission improvement;
there is no NIR or printer contract change.

```sh
cargo test --lib mir65816::
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission --test mir65816_state_boundary --test mir65816_o65 --test mir65816_arithmetic --test actionc_65816_cli --test actionc_65816_o65_cli
python3 tools/native65816-runtime-tests/qualify.py --test call_copies --test call_padding --test interop --test indirect
python3 tools/native65816-runtime-tests/qualify.py --test preemption native_call_arguments
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

Local measurement scripts and full inventories are under `target/native-calls/`.
Packaged Exec uses the existing audit adapter, which removes only the obsolete
`stack_checks:true` layout field; the compiler emits mandatory guards. Live Exec
sources and its compiler pin are unchanged. Hosted adoption remains a separate
qualification step. No public ABI, shared IR or other backend changed.
