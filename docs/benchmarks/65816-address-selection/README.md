# Native address selection measurements

The [implementation plan](../../MIR65816_ADDRESS_SELECTION_PLAN.md) records scope
and the frozen compiler/Exec baseline. JSON snapshots include source and artifact
hashes, raw/optimized code/file/descriptor sizes, relocations and routine frames.
Code bytes include unchanged debug guards; o65 files also include loader metadata.

| Checkpoint | HELLO optimized code / file | CAT optimized code / file |
| --- | ---: | ---: |
| Baseline | 929 / 2,359 | 3,749 / 7,403 |
| Slice 1: direct symbolic AddressOf | 881 / 2,311 | 3,689 / 7,343 |

Slice 1 saves 48/60 bytes in HELLO/CAT in both raw and optimized modes. Frames,
guard bytes, relocation counts, initialized data, BSS and DP reservations are
unchanged. Reserved bank-zero delta: **zero**, including per-task reservations.

Development validation: two selector unit tests; 17 integration/CLI tests across
`mir65816_address_selection`, `mir65816_o65`, `actionc_65816_o65_cli`; nine native
VM tests across `address_selection`, `pointer_values`, `replay`. Runtime cases
cover raw/optimized compilation, fixed/o65 placements, exact address lanes and
canaries; the existing pointer cases include IRQ/NMI. CLI checks include LF/CRLF.
The native runner uses the pinned CPU and enables state-proof/replay checking.

Command size measurements use real command sources and interfaces. Diagnostic
fixed links used to inspect code have placeholder provider addresses and are
not executed. Full backend/Exec qualification is reserved for the final slice.
