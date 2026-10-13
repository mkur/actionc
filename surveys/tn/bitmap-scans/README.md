# Bitmap traversal and compiler priorities

The first qualified change computes the byte limit once in `TagsExtent`, from
its existing capacity snapshot. It halves the exclusive tail-clear cost and
reduces image size. The generic scans and isolated application alternatives
show that byte-wise traversal has much greater execution potential, with a
size cost and a stricter memory contract.

## Baseline and experiments

The compiler is unchanged from `98fe6cb9`; its executable SHA is pinned in
[measurements.json](measurements.json). Baseline TN MIR is 16,705 load-file
bytes. Publishing a 1,088-entry prefix takes 289,381 cycles; `TagBitInRange`
accounts for 221,952 exclusive cycles. End publication with extent 1,089 and
all bits initially set takes 407,371 cycles. These include the call wrapper,
substituted services and executed runtime helpers.

The [generic fixture](../reproducers/bitmap-scans.act) compares four ordinary-RAM
implementations with distinct descriptor and bitmap storage:

| Scan | Bitmap reads for 1,088 bits | MIR cycles, clear / set / mixed |
| --- | ---: | ---: |
| Per-bit record helper | 1,088 | 288,496 / 357,040 / 322,831 |
| Cached base and local count, per bit | 1,088 | 205,867 / 214,591 / 210,237 |
| Byte read plus bit shifts | 136 | 8,848 / 49,124 / 44,457 |
| Byte read plus two nibble lookups | 136 | 17,280 / 17,300 / 17,295 |

The nibble table occupies 16 bytes. The shift scan needs no new table and is
especially cheap for zero bytes. The cached-base variant also accumulates the
count in a local, so its gain cannot be attributed entirely to address reuse
or removing calls. These results are source counterfactuals, not ceilings for
compiler savings.

Isolated TN sources keep the public directory layout and all other routines:

| Alternative | MIR load bytes | Prefix cycles | Complete cycles |
| --- | ---: | ---: | ---: |
| Baseline | 16,705 | 289,381 | 407,371 |
| Cached base and local count | 16,807 | 206,752 | 264,791 |
| Byte read plus shifts | 16,853 | 9,733 | 99,210 |
| Byte read plus nibble table | 16,907 | 18,165 | 67,411 |
| Cached tail-clear bound | 16,693 | 289,381 | 382,607 |
| Nibble table and cached bound | 16,895 | 18,165 | 42,647 |

The per-bit helper already omits capacity validation; `TagsRecount` clamps
its captured limit once. This hotspot does not justify removing provider or
public entry validation.

## Qualified source change

`TagsExtent` computes `byteLimit=capacity RSH 3` once inside the existing
tail-clear guard. The loop compares its CARD ordinal with this captured bound.
The public parameter ABI, record fields, tag masks, partial-byte handling,
bitmap writes and recount remain unchanged. The new local takes two bytes;
MIR removes two spill bytes, keeping the named storage budget unchanged.
`TagsExtent` instruction bytes fall from 515 to 503 in MIR.

For the end-publication vector, exclusive MIR `TagsExtent` cycles change from
49,431 to 24,667, and executed spill operands from 2,267 to 9. Whole-publication
cycles change from 407,371 to 382,607. Prefix and unrelated workload cycles
remain unchanged. Final TN and TNDBG image sizes, classic measurements and
LF/CRLF equality are retained in the report.

| Qualified image | Before | After |
| --- | ---: | ---: |
| TN classic | 18,090 | 18,076 |
| TN MIR | 16,705 | 16,693 |
| TNDBG classic | 21,579 | 21,563 |
| TNDBG MIR | 20,442 | 20,430 |

## Memory and observation boundaries

The generic scans cover zero limits, partial final bytes, capacity clamping,
page crossings, clear/set/mixed bytes, 4,096-bit spans and the largest valid
multiple-of-eight CARD capacity, 65,528. Every visited bitmap byte and outer
guard is checked against an independent host oracle. Byte scans read each
visited byte once; per-bit scans read it once per visited bit.

If bitmap storage aliases the output count, the baseline clears the bitmap
through its initial count store. A cached-byte scan reads the previous byte
instead. The VM test establishes this difference in both backends and runtimes.
Thus the byte-wise alternatives are not general compiler rewrites for arbitrary
pointers. Adopting one in production needs an explicit separate-storage contract
or an alias-safe fallback. Absolute/hardware memory and intervening effects also
require their existing conservative treatment. The qualified bound change
does not depend on a new bitmap alias assumption.

The VM target runs classic and MIR6502, cartridge and standalone, and actual LF
and CRLF compilations. Its production-bound comparison checks both source
forms against complete expected metadata and guarded bitmap contents, including
zero-trip clearing, partial bytes and wide capacities. Application qualification
uses the existing directory, MyDOS reader, dispatch and CPU-profile targets.

## Qualified compiler memory shifts

Optimized MIR6502 now selects native BYTE LSR/ROR on captured private scratch
when shared liveness proves A dead afterward. The exact home value and C/N/Z
are preserved, including live carry through a WORD shift. Source-visible
memory and live-A cases retain fallback. The selection adds no scratch home,
runtime helper, source traversal change or ABI change. Default and explicit
feature-off configurations keep the old sequence.

[memory-shifts.json](memory-shifts.json) pins the new compiler executable,
source hashes, matched roots, direct feature-on/off vectors and check logs.
Both roots retain the qualified cached bound from the source slice above.

| MIR measurement | Source slice | Compiler slice |
| --- | ---: | ---: |
| TN load-file bytes | 16,693 | 16,627 |
| TNDBG load-file bytes | 20,430 | 20,364 |
| `TagBitInRange` instruction bytes | 140 | 125 |
| `TagsExtent` instruction bytes | 503 | 479 |
| Prefix-publication cycles, both roots | 289,381 | 273,061 |
| Complete-publication cycles, both roots | 382,607 | 366,248 |
| Exclusive prefix `TagBitInRange` cycles | 221,952 | 205,632 |

Prefix publication takes 5.6% fewer cycles and complete publication 4.3% fewer
than the source slice. Character/bitmap observations and source operand counts
are unchanged; memory RMW still has one semantic read and write. Classic images
remain byte-identical at 18,076 and 21,563 bytes. All four full-root LF/CRLF
images match.

Qualification passes 1,060 MIR6502 unit tests (two existing ignores), the
emission-tracker dependency test, all 96 MIR6502 integration tests, the
169-fixture emission sweep and all 401 VM/runtime tests. The new differential
VM target covers both runtimes, LF/CRLF, feature off/on, cross-byte carries,
zero/overshifts and snapshots whose source is overwritten before use. Unit
checks retain live A across successors and reject unsupported phases, widths,
indexed/register variants and source-owned targets. The
[pseudo-machine contract](../../../docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md)
defines the new forms; the shared rewrite inventory includes their proof.

## Qualified masked index cleanup

Optimized MIR6502 removes dead register AND/OR/XOR using shared A and N/Z
liveness across the complete CFG. Removing the high-lane AND exposes an unused
parameter reload to the existing conservative load cleanup. WORD source
captures, observable absolute reads, public argument homes and the full
`ordinal RSH 3` address computation retain their semantics.

[dead-high-indexes.json](dead-high-indexes.json) pins the compiler and checks.
`TagBitInRange` falls from 125 to 120 instruction bytes. TN and TNDBG MIR images
shrink by another 10 bytes, to 16,617 and 20,354. Both prefix vectors change from
273,061 to 266,533 cycles (2.4% fewer); complete publication changes from
366,248 to 359,714. Classic images stay byte-identical and all full-root LF/CRLF
images match. The direct masked-index feature-off/on vector takes 42/38 cycles
in both runtimes. Snapshot tests check both ordered source-byte reads, an
intervening overwrite, result guards and wide indexes through 65,535.

Qualification passes 1,063 MIR6502 unit tests, 96 integrations, the 169-fixture
emission sweep and the masked-index, bitmap, private-shift and TN profile VM
targets. Live A, live flags, memory operands and carry arithmetic retain fallback.

## Qualified captured bound motion

Optimized MIR6502 moves total header arithmetic to a proven unconditional
preheader, using complete temp lanes and the existing CFG, dominance, reaching
definitions and machine liveness. Scalar capture reloads need a unique complete
dominating store, an unchanged unexposed local and no loop barrier. Source
record/pointer captures keep their original sites. Calls, pointer writes,
absolute writes, unknown effects, aliases and mutable bounds retain fallback.
Operation values and CFG stay unchanged; the generation-scoped transaction
rechecks the whole proof before committing. Final instruction, home, image and
occupied zero-page budgets cannot increase.

[captured-bounds.json](captured-bounds.json) pins the compiler, corpus and
measurements. The [fixture](../reproducers/captured-bounds.act) separates stable
captures from mutable bounds, repeated pointer reads, calls and pointer writes.
For a 1,088-bit bound, the direct counted-loop vector changes from 13,742 to
4,886 cycles in both runtimes, 64.4% fewer. A chained shift/mask bound changes
from 14,701 to 4,893. The mutable-bound vector remains 13,820 cycles.

The combined fixture image changes from 985 to 943 bytes. `Count` changes from
95 to 83 instruction bytes and from four to two spill bytes; `Snapshot` and
`Repeated` each save the same 12 instruction and two spill bytes. Their local
and parameter budgets do not grow. Calls, mutable bounds and pointer-write
fallbacks retain their instruction and home budgets. All fixture images match
across actual LF/CRLF compilations.

TN and TNDBG stay byte-identical to the masked-index slice in both backends
and both newline formats. Their production tail loop still uses the explicit
cached bound: this first compiler slice conservatively rejects pointer writes
inside loops. The measured counted-loop gains therefore describe the generic
captured-bound vectors, not an additional TN speedup.

Final qualification passes 1,070 MIR6502 unit tests (two existing ignores),
all 96 MIR6502 integrations, the 169-fixture emission sweep and all 403 VM/runtime
tests. Checks stay within the affected backend; SemIR and NIR contracts do not
change.

The backend contract and rewrite workflow document the motion boundary.
Qualification covers zero-trip loops, both WORD result lanes through 65,535,
ordered source reads followed by source overwrite, call side effects, bitmap
guards, multiple backedges, side entries, stale plans and live flags. The new
selection has an independent feature-off configuration for differential runs.

## Further scope

Moving record loads across pointer writes or calls needs stronger storage and
effect proofs. The much larger byte-wise bitmap traversal gain remains a
source-level option whose RAM/code and alias contracts must be assessed
independently.

## Reproduction

The original source tree is retained under `target/tn-bitmap-scan/baseline`.
It can also be extracted from `98fe6cb9`. The frozen compiler is retained under
`target/tn-comparison-loops/final/actionc-qualified`.

```sh
python3 surveys/tn/measure-bitmap-scans.py \
 --compiler target/tn-comparison-loops/final/actionc-qualified \
 --source-dir target/tn-bitmap-scan/baseline \
 --out-dir target/tn-bitmap-scan/recheck

CARGO_TARGET_DIR=target/tn-bitmap-scan/build CARGO_INCREMENTAL=0 \
 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=1 \
 cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml \
 --test bitmap_scans -- --nocapture
```

Use `--roots TN TNDBG --variants cached_bound --crlf` for the selected full-root
image check. To profile an isolated alternative, set
`ACTIONC_TN_PROFILE_DIRECTORY` to its generated source directory,
`ACTIONC_TN_PROFILE_SOURCE` to TN or TNDBG and `ACTIONC_TN_PROFILE_ROUTINES=1`,
then run `tn_size_profile` with `--nocapture`. Images, listings and raw logs stay
under `target/tn-bitmap-scan`; only the disposable Rust cache is removed.

To rerun the two compiler slices with feature-off/on execution and real newline
conversion:

```sh
CARGO_TARGET_DIR=target/tn-arithmetic/build CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=1 \
  cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test dead_high_indexes --test captured_bounds -- --nocapture
```

The qualified masked-index and captured-bound executables, complete root images,
fixture images, listings and logs remain under `target/tn-arithmetic`. Their
reports pin compiler SHA256 values and the relevant source hashes.
