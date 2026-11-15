# ZP homes across ordinary calls

The extension is implemented and enabled in the optimized configuration. The
[final qualification](#final-qualification-and-default-slice-6) records the
default decision, complete check scope and remaining limitations.

## Baseline (slice 0)

Slice 0 froze the optimized compiler at `decbc208` (compiler behavior
`a77210d8`). [baseline.json](baseline.json) records sources, library/image/map
hashes, configurations, work counts and traced/untraced equality. Cross-call
home selection was disabled for these baseline measurements.

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

## Combined parameter and local qualification (slice 5)

[Gate off](locals-off.json) and [gate on](locals.json) cover both runtimes/origins,
with inlining enabled and disabled. Locals add **8 bytes** of saving to the
original independent fixture: **294 → 222** with inlining disabled and
**295 → 226** normally. `FindRecord` is 51 code bytes, no inline storage, five ZP
bytes; `CountRecords` is 37 code bytes, no inline storage, three ZP bytes.

A separate local fixture measures **248 → 202** bytes with inlining disabled
and **254 → 211** normally. It includes initialized word/pointer locals, a routine
with no parameters, a parameter whose capture already disappeared, a persistent
entry-read local and an address-exposed local. The latter two retain inline
storage. Six root controls and two VM targets pass, including raw/optimized NIR,
both feature settings, actual LF/CRLF loading, canonical argument observations,
ordered pointer reads, repeated calls and independent arithmetic/stack oracles.
[Check receipts](slice5-checks.json) record this scope.

TN's combined trial makes `Ord` **51 code + 0 inline** and `Items` **37 + 0**,
but the same four other routines each retain an extra two-byte store. The net
48-byte image saving is rejected by the unchanged no-routine-growth policy.
Final TN remains **9,847 / 9,954**, with exact image/map/relocation rollback.
No result-home coalescing or cleanup ownership relaxation was added.

## Final qualification and default (slice 6)

`Mir6502Config::optimized()` now enables cross-call home selection. The
conservative default leaves it disabled, as do disabled parent peephole or
parameter-home gates. One batch places eligible parameters and definitely
initialized locals; it adds at most one core continuation. Missing authoritative
local bindings remain unresolved in layout and callee facts, even before the
verifier rejects malformed MIR.

[Qualification receipts](qualification.json) record these passing checks:

| Scope | Passed | Coverage |
| --- | ---: | --- |
| MIR6502 units | 1,404 | Full MIR6502 unit filter; 15 existing ignored checks excluded |
| MIR6502 integrations | 158 | All 35 root MIR6502 targets |
| VM execution | 108 | 39 affected runtime, ABI, storage and optimization targets |
| MADS assembly | 13 | Explicit execution of assembly tests ignored by the normal run |
| Final default artifacts | 20 | Exact image, map and relocation equality with explicit-on measurements |

The final [default comparisons](default-equality.json) cover TN and both
independent fixtures, both runtimes and both origins, with control-fixture
inlining enabled and disabled. VM coverage includes raw/optimized NIR and actual
LF/CRLF source loading. MADS reconstructs and relocates LF/CRLF listings. The
existing unknown-TN-destination manual experiment remains explicitly skipped;
the receipt lists ignored unit checks. No shared NIR/SemIR contract or other
backend changed.

Two integration test setups needed correction. The leaf-only trial counter now
explicitly disables the new extension; combined controls keep it enabled. The
parent-gate test uses a branch-free call because the existing no-peephole path
does not lower abstract Boolean branches when other optimized flags remain set.
Both entire affected targets passed their rechecks. No compiler behavior was
changed in response to these test failures.

Final sizes are **294 → 222** and **248 → 202** bytes for the independent
fixtures with inlining disabled; normal inlining gives **295 → 226** and
**254 → 211**. The zero-parameter local case qualifies when its real call remains;
normal inlining removes that call and leaves it outside this call-only scope.
TN stays **9,847 cart / 9,954 standalone** bytes with exact rollback. Its rejected
trial saves 48 bytes overall, but four other routines grow two bytes each due to
the global cleanup observation inventory described above.

Compilation work increases by one materialization for a nonempty batch: TN
**5 → 6**, the original control **2 → 3** without inlining and **5 → 6** normally,
and the local control **7 → 8** normally. Across the four TN runtime/origin lanes,
the median observed untraced compilation time is **16.03 s off → 20.38 s on**.
These are single measurements under concurrent host work, not a statistical
benchmark. The complete measurement process, including traced runs, peaks at
approximately **447 MB off / 502 MB on**. A fresh default-only untraced run of
all 20 cases takes 84.84 seconds and peaks at 89 MB; the receipt includes its raw
time report. This is a code-size feature with a measured compile-time cost.

Nested/recursive/indirect calls, unsupported callee effects, exposed or
entry-dependent locals, and result-home coalescing remain outside this scope.
The strict image-saving and no-routine-growth guards remain unchanged.
