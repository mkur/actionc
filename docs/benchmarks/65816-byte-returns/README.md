# Native BYTE constant returns

MIR65816 now prepares a typed U8 constant directly with A16 `LDA #$00xx`
when the authoritative result home is `A8ZeroExtended`. This defines both A
bytes, including hidden B, and uses the existing A-preserving frame teardown
and RTL. X is unspecified for a BYTE result and needs no initialization.
Captured BYTE operands and other result homes retain their existing paths.
See the [emission contract](../../MIR65816_EMISSION_CONTRACT.md).

Each selected return removes 19 bytes of DP result preparation. No source
load is widened or removed; frames, guards, call layouts and physical ABI v1
are unchanged. Added stack, DP and bank-zero reservation: **0 bytes**.

## Frozen Exec measurement

The workload is Exec `c3500c8`, eight tasks, console, MyDOS and stack checks
enabled, as in the [guard-branch baseline](../65816-guard-branches/README.md).
Both modes were rebuilt with the compiler at `e1d4e8c3` before this change and
with the working implementation. The old compiler routine totals reproduce
the guard-branch baseline, despite the intervening arithmetic-helper work.

| Measurement | Raw before → after | Optimized before → after |
| --- | ---: | ---: |
| Compiler routine bytes | 483,721 → 475,095 | 452,577 → 443,153 |
| Eligible constant returns | 454 | 496 |
| Code bytes saved | **8,626** | **9,424** |
| All executable bytes | 491,286 → 482,660 | 460,142 → 450,718 |
| XEX file bytes | 507,860 → 499,090 | 476,142 → 466,556 |
| Guards retained | 2,328 | 2,320 |

The optimized saving is 2.05% of the previous executable size and exactly
matches the audit forecast. Raw and optimized modes expose different numbers
of typed constant returns. There are no secondary branch-relaxation savings.

All 551 routine contracts match in each mode, including frames, temporary homes,
arguments, results and calls. A final-byte comparison checks the exact old
22-byte preparation against the new three-byte load, then verifies every other
instruction after relocation rebasing. Labels, fixups, MIR spans, short branch
targets and PER continuations follow the same position mapping. Guard counts,
amounts and order are unchanged. See [totals and hashes](exec-results.json) and
the [changed routine sizes](exec-routines.csv).

The full packaged builds match the inventory compiler segments exactly, with
matching generated sources, platform/ABI inputs and bank-zero budgets. XEX
totals include packaging overhead; the compiler saving remains exactly 19
bytes per selected return.

The 28 Action images in the small corpus and both Dijkstra images are
byte-identical to the qualified guard-branch baseline. Dijkstra remains 5,438
raw / 4,850 optimized code bytes; this slice has no eligible returns there.
Actual LF/CRLF builds agree. See [corpus sizes](corpus-sizes.json) and
[Dijkstra sizes](dijkstra-sizes.json). Their unchanged binaries retain the
previous execution measurements; the external vbcc comparison was not rerun.

## Validation and reproduction

Native checks pass: **197 library tests** (one opt-in test ignored), **69 root
integration/CLI checks**, and **172 runtime tests in each of debug and release**
(four external inventory/comparison tests ignored in each). All 491 input
hashes and 861 artifact hashes match across the runtime runs. The existing
emission snapshot passes without modification. The
[qualification record](qualification.json) retains manifest locations/hashes,
the pinned VM/patch, changed source hashes and measurement-tool provenance.

The selector tests cover all 256 constants with incoming A8/A16 state,
zero/nonzero frames, unchanged allocation and result-home/operand rejection
without partial emission. Independent ca65 callers execute all 256 results
and check both bytes of A in raw/optimized fixed images and two relocated o65
placements, with both incoming I states. Additional probes check conditional
returns, captured BYTE values, direct/indirect calls, framed teardown and exact
RTL stack reads, with no tail writes or DP scratch accesses.

The preemption probe executes zero-frame and framed conditional returns in both
task domains. Every reached instruction receives IRQ and NMI probes that check
full resumed state; both seeded IRQ/NMI schedules also pass. LF/CRLF source
instrumentation produces identical input, and the constant source is compiled
through the actual parser with both newline forms.
The focused interrupt probe covers 110 raw / 102 optimized task/PC/status sites,
with both IRQ and NMI checked at each site.

```sh
cargo test --lib mir65816::
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission --test mir65816_state_boundary --test mir65816_o65 --test mir65816_arithmetic --test actionc_65816_cli --test actionc_65816_o65_cli
python3 tools/native65816-runtime-tests/qualify.py --test byte_returns --test preemption byte_constant
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

The measurement probe and byte comparison are retained locally in
`target/byte-constant-returns/`. Full Exec builds use the existing audit adapter
that asserts `stack_checks:true` and removes only that layout field; main emits
mandatory guards. Live Exec sources and its compiler pin are unchanged. These
compiler checks do not qualify hosted Exec adoption. No NIR/printer contract or
other backend changed; the existing emission snapshot is unchanged.
