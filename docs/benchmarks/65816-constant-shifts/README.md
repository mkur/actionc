# Final index shifts and constant shifts

Baseline: compiler `74731616`; frozen Exec `c3500c8`, eight tasks, console,
MyDOS and stack checks. These measurements continue the
[local relaxation baseline](../65816-local-relaxation/README.md).
The [selection contract](../../MIR65816_CONSTANT_SHIFTS.md) describes scratch,
width, overlap and flag invariants. No Exec pin or live Exec sources changed.

| Frozen Exec | Raw | Optimized |
|---|---:|---:|
| Executable bytes before | 431,389 | 399,816 |
| Executable bytes after | 427,908 | 395,040 |
| Executable bytes saved | **3,481** | **4,776** |
| XEX bytes before | 446,867 | 414,736 |
| XEX bytes after | 443,350 | 409,872 |
| Removed final index shifts | 394 | 394 |
| Index bytes saved | 2,364 | 2,364 |
| Specialized constant shifts | 32 | 52 |
| Constant-shift bytes saved | 1,079 | 2,356 |
| Following mode bytes saved | 36 | 52 |
| Further local jump bytes saved | 2 | 4 |
| Guards retained | 2,328 | 2,320 |

All 551 routine contracts compare equal, including frames, temporary homes,
argument/result placement and stack peaks. Every guard retains identical bytes
and order. Bank-zero reservations are unchanged. The span audit accounts for
each changed instruction group, proves exact deletion of ASL/ROL/ROL from
indexed addresses, and compares unaffected instructions after normalizing
relocation operands and local transfer encodings. Ordinary packaged image
segments match the independently built inventory image. Generated Action
sources, ABI and platform inputs match the baseline.

The [totals](exec-results.json), [routine inventory](exec-routines.csv) and
[changed sites](exec-sites.csv) retain hashes and byte accounting. Container
overhead changes explain the additional XEX savings beyond executable bytes.

The 14-pair comparison corpus retains 132 correct Action records, executed with
both incoming interrupt-mask states in both debug and release hosts. Both hosts
produce identical records. Optimized `wide_shift` shrinks 307→189 code bytes and
611→458 cycles on all five inputs. Its byte traffic and stack peak are unchanged.
All other corpus records retain their sizes, cycles and memory traffic. The
known external optimized vbcc `unlink` vector-zero failure is unchanged; both
comparison invocations exit 101 for that failure. See
[sizes](corpus-sizes.json), [execution checks](corpus-execution.json) and
[per-vector traffic](corpus-execution.csv).

Dijkstra shrinks 5,213→5,122 raw and 4,625→4,535 optimized code bytes, with all
22 guards and routine contracts intact. The targeted `original-0-50` graph
passes for both compilers, modes and incoming interrupt-mask states. Action raw
cycles fall 113,703,790→107,757,271; optimized cycles fall
105,508,809→99,562,389. Both modes remove 1,189,284 DP byte reads and the same
number of writes. Stack traffic, stack peaks, metadata reads and mode switches
are unchanged. Other Dijkstra graphs were not rerun. See
[sizes](dijkstra-sizes.json) and [execution](dijkstra-execution.json).

Both corpus generators verify LF/CRLF builds. New native regressions compile
both newline forms through the real frontend, test every residual and width
boundary, full-width large counts, signed logical right shifts, mutable
parameters, destination canaries, 24-bit index carries/wrap and exact volatile
accesses. Independent ca65 code validates native ASL/ROL and LSR/ROR encodings.
The new preemption probe interrupts every reached instruction in two shift
routines in both task domains, checking full IRQ/NMI restoration and seeded
schedules while residual carries and X16 counters are live.

The targeted probe covers 551 raw / 402 optimized task/PC/status sites, applying
both IRQ and NMI at each. Final mode-selection cleanup removes five additional
four-byte mode round trips in each Exec build. Rebuilt corpus and Dijkstra
artifacts are byte-identical to those executed in the recorded measurements
(224 corpus files and 24 Dijkstra files), including LF/CRLF equality.

Compiler checks pass 214 native unit tests and 69 tests across eight native
integration/CLI targets. The emission snapshot change is intentional: only optimized
`wide_shift` instruction bytes, labels and spans change; frame maps remain
identical. Full native debug/release qualification passes 186 tests in each
host (four opt-in tests ignored), with all 1,043 saved artifacts identical.
All 499 compiler/fixture input hashes also match. Coverage includes calls, helpers,
recursion, state/effect replay, relocated o65, volatile accesses and stack guard
success/failure. Exact run counts, source hashes and saved-artifact agreement
are in [qualification.json](qualification.json). Exec integration and hardware
adoption remain separate from these compiler measurements.
