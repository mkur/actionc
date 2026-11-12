# Checked arena qualification

The checked arena and range-based pointer retention are qualified. Optimized
MIR6502 enables the consumer; conservative defaults keep it off. The neutral
program saves **40 emitted bytes and 40 load-file bytes**, with **28 cycles**
saved on each accepted call. TN gains no compiler saving: its checked library
costs **451 load-file bytes** in MIR6502, and generic `Strcat` stays **113 bytes**.

## Complete load-file sizes

The unchecked column is pinned to `dedd35ab` and the historical compiler hash
in [unchecked.json](unchecked.json). Checked off/on use the same checked source
and final compiler, with only the independent range-retention flag changed.
All MIR6502 off/on outputs, including maps and relocations, compare equal.

| Backend | Program | Runtime | Unchecked | Checked off | Checked on | Library cost | Compiler saving |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| optimized | TN | cart | 10590 | 11061 | n/a | +471 | 0 |
| mir6502 | TN | cart | 9857 | 10308 | 10308 | +451 | 0 |
| optimized | TN | standalone | 10818 | 11289 | n/a | +471 | 0 |
| mir6502 | TN | standalone | 9982 | 10433 | 10433 | +451 | 0 |
| optimized | TNDBG | cart | 13835 | 14302 | n/a | +467 | 0 |
| mir6502 | TNDBG | cart | 12952 | 13403 | 13403 | +451 | 0 |
| optimized | TNDBG | standalone | 14063 | 14530 | n/a | +467 | 0 |
| mir6502 | TNDBG | standalone | 13077 | 13528 | 13528 | +451 | 0 |

The library cost includes the checked primitives, fault sink, captures and
conservative `MovePage` effect correction. It is distinct from compiler savings.
Classic optimized has no range-retention consumer. Its `Strcat` remains 118 bytes.

## Storage and runtime behavior

| Checked MIR6502 | Runtime | Routine bytes | Mapped frame-symbol bytes | Deferred reserved bytes | Source-storage zero-page bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| TN | cart | 9461 | 3022 | 2794 | 13 |
| TN | standalone | 9584 | 3024 | 2794 | 13 |
| TNDBG | cart | 12021 | 3061 | 2794 | 13 |
| TNDBG | standalone | 12144 | 3063 | 2794 | 13 |

These storage figures are unchanged by range retention. The full per-routine
frame map, physical reservations and source zero-page addresses are in
[checked.json](checked.json). Source-storage zero-page counts exclude implicit
compiler scratch. The regional transaction preserves every frame/allocation;
it continues to use the same two fixed pairs and introduces no new scratch.

The runtime arena is still `$2000..$2800`, exclusive end: 2,048 bytes of RAM,
not an emitted data reservation. Nested TN UI flows reached `$2583` (1,411
bytes); TNDBG reached `$274C` (1,868 bytes), leaving 180 bytes. Rewind leaves
released bytes intact, preserving `MkDir` and `CloseWindow` scratch use.

Valid `Alloc(1)` costs 69 → 223 cycles in MIR6502 and 75 → 235 in classic
optimized, in both runtimes and both programs. These totals include the same
17-cycle call wrapper. All six measured successful request/cursor combinations
are recorded; invalid requests use a separate bounded sink runner. The short
`Strcat` parameter-home alias case (lengths three and two) still returns the
expected result, at 247 MIR6502 cycles including the wrapper.

## Neutral benefit and rejection controls

For `checked_pointer_ranges.act`, the complete emitted image changes from
655 → 615 bytes for raw NIR and 654 → 614 for optimized NIR. The complete
load-file delta is also 40 bytes. `Guarded` and `FromReturn` each lose 20 bytes;
`Unrestricted` and the copy helper do not change. Both runtimes and origins
`$3000`/`$41C7` agree. A representative guarded 3+2 call changes 309 → 281
cycles including the wrapper. Every accepted case saves four setup stores.

The VM matrix executes 8,664 calls across all counts 0–255, forward/reverse
overlap, equal pointers, page carries, raw/optimized NIR, actual LF/CRLF files,
both origins and runtimes. Memory, ordered LDA/ADC indirect reads, A/X/Y/flags
and stack agree. Zero count still copies 256 bytes, and final lengths are read
after copying. Hardware, wrapping, code overlap, either source-home lane
overlap, changed generations/bodies, mixed word lanes, published entries and
explicit scratch observations retain the baseline.

A driver regression confirms that the ordinary post-home rewrite API still
rejects the removed-definition dependency. Only the sealed regional proof
permits remapping both pairs. A whole-image cost test cancels the 40-byte
region saving with 40 bytes of data and rejects the tie. Applications are
reported only after final-layout replay and strict cost acceptance.

## TN admission census

`ArenaCursor` and `Alloc` derive `[8192,10240]` on every normal return in both
raw and optimized NIR. That is a numeric value range, including zero-size
allocation at the end; it is neither freshness nor a capacity/lifetime promise.
Original `Strcat` has zero bounded entry cells. No caller census narrows it.

| Call in TN and TNDBG | Actual reaching argument facts | Admission |
| --- | --- | --- |
| `MkDir`, append `linebuf` | Both full `[0,65535]`; intervening effects lose the earlier stored allocation fact | Unknown original entry; no private clone |
| `Rename`, append comma | Destination full range; static literal address has no numeric range receipt | Unknown entry and unsupported literal-address query |
| `Rename`, append new name | Both full ranges; mutable destination/global and input pointer are not entry invariants | Unknown original entry |

These are fact/entry blockers before physical-overlap, exit-state or cost
selection. They are not failed physical proofs. The generic parameter-home
overwrite remains legal input, including the short 3+2 case. Specializing TN
requires the separate private-entry/storage-identity design described in the
[design note](../../TN_CHECKED_ARENA_AND_POINTER_RANGES_DESIGN.md).

## Analysis and generation cost

TN uses 96 linked routines: 2,981 transfer evaluations for raw NIR and 3,002
for optimized NIR. TNDBG uses 104: 3,020 raw and 2,961 optimized. Each routine
keeps its 4,096-evaluation ceiling. Exact point receipts and final physical
footprints are replayed; exhaustion or missing facts keep the barrier.

`checked.json` records range-analysis and generation wall times. Measurements
use Rust dev/test opt-level 1, debug off, incremental off; they time generation,
not rustc builds. TN generation is approximately 8 seconds and TNDBG 26 seconds
on this host. Range analysis takes tens of milliseconds. Individual off/on
differences are scheduling noise, not a speed claim. Programs without machine
blocks skip the source-MIR clone, and those without the final region shape
skip numeric analysis. The optimized default is qualified; disabling parent
pointer-call retention also disables this consumer with CFG retention enabled.

## Reproduction and validation

Fresh bindings include normalized source/compiler hashes, image/listing hashes,
routine maps, frames, reservations, cycles and all three caller records.
VM cycles are accepted only when the VM-loaded images hash-match the fresh
compiler outputs. The VM dependency is pinned at `7ec0cc454ebf43b088b7bcd11515533085ea1964`.

```sh
python3 tools/compare6502/tn/checked_arena.py --compiler build/checked-arena/unchecked-actionc --check
python3 tools/compare6502/tn/checked_ranges.py --vm
python3 tools/compare6502/tn/checked_ranges.py --check
```

The first command requires a qualified historical compiler (or a build from
the pinned unchecked revision). The new generator uses its own opt-level-1
Cargo target directory. `--workspace PATH` optionally validates from an isolated
checkout; every compiler source must match. Wall times are observations and
are excluded from deterministic check comparison; hashes and cycles are not.

Checks completed for the implementation:

- Shared NIR snapshots, the 51-fixture NIR sweep and full `cargo test`: 4,053 passed, 44 ignored at the final NIR-contract slice.
- Final MIR6502 library suite: 1,250 passed; 23 affected integration targets: 111 passed. The 169-fixture `--emit-load` sweep passes.
- Enabled-region MADS assembly: both runtimes, both origins and actual LF/CRLF listings; relocation equals fresh direct output. Existing bounded-call MADS controls also pass.
- Checked TN runtime: 1,120 calls (464 returning, 656 fault-sink cases), both source backends/runtimes and actual LF/CRLF.
- New range VM: 8,664 calls; existing bounded-call VM: 2,952 calls; pointer retention, argument staging/forwarding, entry flags, pointer consumers, compound accesses and absolute readback pass.
- TN dispatch/panels pass against fresh checked images. Frozen idiomatic `Strcat` VM controls pass; four historical images and 80 LF/CRLF assemblies match recorded hashes.
- Both historical and fresh measurement generators pass their check commands.

The shared/backend checks ran in an isolated checkout containing the task
changes. The initial main-workspace full run encountered the unrelated
untracked `samples/vbxe/shared/lines.act` and its unconfigured `SHARED.SCREEN`
import. That file and all unrelated C-port/sample changes were preserved.
