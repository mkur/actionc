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

## Parameter qualification (slices 2–3)

[Gate off](parameters-off.json) and [gate on](parameters.json) receipts use the
same compiler and sources, both runtimes and both origins. The independent
fixture shrinks **294 → 230 bytes** with inlining disabled and **295 → 234** with
normal inlining. `FindRecord` becomes 54 code + 1 inline local + 4 ZP bytes;
`CountRecords` becomes 40 + 1 + 2; `KeepWord` becomes 35 + 0 + 4. The initialized
counters remain inline. Pointer results still copy explicitly from `$A0/$A1`.

TN stays **9,847 / 9,954 bytes**. Its trial would reduce `Ord` from 85 to 55 total
code/ordinary bytes and `Items` from 59 to 41, but `MoveMenuBar`, `Draw`, `Tag` and
`TagAll` each grow two bytes. The whole trial is rejected despite a net 40-byte
image reduction. Image, map and relocation hashes exactly equal gate-off output.
No acceptance policy was relaxed and no per-routine retry was added.

Five root controls and the independent VM target pass, including raw/optimized
NIR, feature off/on, actual LF/CRLF loading, page crossings, wrapping word values,
canonical call arguments, ordered pointee reads and persistent local storage.
Fallback controls cover multiple/nested/recursive calls, a helper, machine code,
exposure and exhausted/source-owned ZP. Manual MIR tests from slice 1 cover
partial-word/absolute clobbers, changed bindings and stale allocation assumptions.
[Check receipts](slice3-checks.json) include TN's exact rollback. Full backend
and MADS qualification is reserved for the combined implementation in slice 6.

Reproduce the enabled measurement by adding `--cross-call` to the baseline
command. Each admitted batch adds one continuation; empty batches add none.

Inspecting the parameter trial's MIR identifies the growth mechanism: terminal
cleanup retains extra ZP temporary stores when newly resident source objects
share those addresses in other routines. For example, `MoveMenuBar` keeps its
`store zp1, a` before `x = a`; it does not call `Ord`. The current global physical
observation inventory cannot separate these lifetimes. This is a conservative
cleanup limitation, not evidence that the new call-preservation proof failed.
The initial implementation retains that inventory and the strict cost guard.
