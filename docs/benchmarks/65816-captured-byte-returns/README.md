# Captured BYTE returns

Baseline: compiler `2a3e90d1`; frozen Exec `c3500c8`, eight tasks, console,
MyDOS and mandatory stack checks. This continues the
[constant-shift baseline](../65816-constant-shifts/README.md).
The [selection contract](../../MIR65816_CAPTURED_BYTE_RETURNS.md) defines the
exact source extent, preflight, mode and ABI invariants.

An exact BYTE stack temp or parameter now loads in A8, restores A16 and applies
`AND #$00FF`. The source read remains one byte, while hidden B is always zero.
X needs no initialization under the BYTE ABI. The existing terminal-boundary
mode restoration, frame release and RTL remain intact. Unsupported homes and
other result widths retain their prior paths. No allocation or public ABI
changes are included.

| Frozen Exec | Raw | Optimized |
|---|---:|---:|
| Eligible captured BYTE returns | 181 | 160 |
| Return preparation bytes saved | 2,353 | 2,080 |
| Further branch/jump bytes saved | 39 | 3 |
| Total executable bytes saved | **2,392** | **2,083** |
| Executable bytes before | 427,908 | 395,040 |
| Executable bytes after | 425,516 | 392,957 |
| XEX bytes before | 443,350 | 409,872 |
| XEX bytes after | 440,904 | 407,735 |
| Guards retained | 2,328 | 2,320 |

Every eligible preparation shrinks 22→9 bytes. Each executed return removes
four DP byte reads and five DP byte writes, with no change to stack traffic or
stack peak. The optimized primary saving matches the 160×13-byte forecast.
Secondary savings come only from existing typed local transfer relaxation.
The XEX reduction additionally reflects packaging overhead.

All 551 routine contracts compare equal, including frames, temporary homes,
argument/result placement and stack peaks. All guards retain identical bytes
and order. Added stack, DP and bank-zero reservation is zero. The final-byte
audit checks each exact old/new return sequence, then proves equality of all
other instructions after explicitly accounting for transfer relaxation and
relocation operands. Labels, fixups, MIR spans, relative targets and PER
continuations follow the same checked position mapping.

The ordinary packaged builds match the inventory compiler segments exactly.
Generated Action sources, platform/ABI inputs and bank-zero budgets match the
baseline. See [totals and hashes](exec-results.json),
[routine sizes](exec-routines.csv) and [selected sites](exec-sites.csv).

The 28 Action corpus images and both Dijkstra images are byte-identical to the
previous qualified baseline. Dijkstra remains 5,122 raw / 4,535 optimized code
bytes. Existing cycle, stack and DP traffic measurements therefore remain
applicable; unchanged benchmark binaries were not executed again. The known
external vbcc optimized `unlink` failure retains its prior status. All external
benchmark binaries are also unchanged; path differences in host listings/maps
and build logs are checked separately. Both generators compile LF/CRLF inputs.
See [corpus equality](corpus-equality.json) and
[Dijkstra equality](dijkstra-equality.json).

Focused native checks cover all 256 byte values through direct and indirect
calls, immutable/mutable parameters, real zero/nonzero frames, and two o65
placements in raw/optimized modes with both incoming I states. Independent ca65
tails match actual emitted bytes. Execution with poisoned hidden B checks full
A16 results, exact source and RTL reads, no neighbor or DP scratch reads, no
tail writes, unchanged X and correct frame restoration. Separate volatile and
alias tests preserve captured values across direct/indirect callees that clobber
all 64 scratch bytes. LF/CRLF source compilation and instrumentation agree.

The targeted preemption probe checks 196 raw / 188 optimized task/PC/status
sites across both task domains. Each site receives IRQ and NMI restoration
checks, and both seeded schedules pass. Native unit checks pass 217 tests (one
opt-in ignored); all 69 tests in eight integration/CLI targets pass, including
the unchanged emission snapshot. Full native debug/release qualification passes
190 tests per host (four opt-in tests ignored), with all 1,113 saved artifacts
and 501 input hashes identical. It covers call/helper clobbers, recursion,
effects/replay, relocated o65, volatile
accesses and stack guard success/failure. Counts, source hashes and artifact
agreement are recorded in [qualification.json](qualification.json).

No SemIR/NIR contract or other backend changed. Exec's live source and compiler
pin remain unchanged; compiler measurement does not qualify hosted adoption.
