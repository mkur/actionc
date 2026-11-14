# ZP homes across ordinary calls

Slice 0 freezes the optimized compiler at `decbc208` (compiler behavior
`a77210d8`). [baseline.json](baseline.json) records sources, library/image/map
hashes, configurations, work counts and traced/untraced equality. No new home
selection is active yet.

| Program | Cart bytes | Standalone bytes |
| --- | ---: | ---: |
| TN | 9,847 | 9,954 |
| Independent controls, inlining disabled | 294 | 294 |
| Independent controls, normal inlining | 295 | 295 |

Both `$3000` and `$41C7` agree. `Ord` and `FindRecord` use 80 code + 5 inline
bytes; `Items` and `CountRecords` use 56 + 3. `KeepWord` uses 44 + 4.

The root `mir6502_cross_call_homes` and VM `cross_call_homes` targets pass.
They check LF/CRLF through the parser and actual file loader, canonical callee
arguments, ordered pointee reads, zero/repeated loops, early exits, conditional
calls, word results, persistent locals with explicitly seeded storage, and stack
balance. The VM covers cart at `$3000` and standalone at `$41C7`; measurement
covers both origins in both runtimes. No compiler code changed in this slice.

Reproduce with `tools/compare6502/parameter_homes.py --api-rlib <selected-rlib>
--output <artifact-directory> --enabled --handoff --suite cross-call`. Local
artifacts and check logs are under ignored `build/cross-call-homes/`. The shared
Cargo target is `build/checked-arena/shared-target`, with opt-level 1, debug 0,
incremental disabled and two jobs. Timing is observational, not a performance
claim.

Baseline fixture development encountered an existing store-consumer diagnostic
for `value=AddOne(value); RETURN(value+keep)`; the independent control uses
`RETURN(AddOne(value)+keep)` instead. A conditional initialization followed by
a call also failed its VM oracle in the unchanged compiler; the persistent-value
control seeds its inline cell explicitly and reads it over successive calls.
These findings are outside the placement change and are not claimed as fixed.
