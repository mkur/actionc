# TOMS Navigator samples

The maintained sources in `modern/` target actionc's modern classic and MIR6502
backends with the Action! cartridge runtime. Archived sources, the original ATR
and original `TN.COM` remain under `corpora/tn/`.

The [directory migration plan](../../surveys/tn/directory-handling-implementation-plan.md)
is implemented. MyDOS still reads and sorts up to **64 files as one batch**,
with 16 visible rows. The free-space summary is separate. Directory data keeps
native names and attributes; rendering converts them to screen bytes. Commands
resolve exact names without decoding the display.

| Shared source | Responsibility |
| --- | --- |
| `DIR.ACT` | 64-entry cache, caller-owned tag bitmap, listing generation and iteration |
| `MYDOS.ACT` | Complete MyDOS listing, ordering, exact filenames and row rendering |
| `LOCATION.ACT` | MyDOS drive, sector and four-level path |
| `PANELDIR.ACT` | Cache fetch/publication, viewport, end discovery and name-resolution boundary |
| `LIB.ACT` | Existing screen, menu, allocation and CIO services |

`PanelState` contains CARD file/selection ordinals and a BYTE screen row. Each
panel separately owns its cache, location and selection. Switching panels saves
live MyDOS state before activating the next location, and copies only the
five-byte view state. Refresh starts a new listing generation and clears tags.
The eight-character path-name overflow is fixed; the dot is never copied into
a counted path slot.

Tagging uses **eight bytes per MyDOS panel**. A synthetic directory source tests
512-byte bitmaps for 4,096 entries without enlarging the production bitmap.
Tags survive cache eviction. Unknown-length tag-all must discover the end before
file operations can proceed, and reports a capacity error if the complete
selection cannot fit. Browsing can continue beyond the selection capacity.
A changed or unrepeatable listing invalidates its generation and tags.

There is no live FujiNet directory source or new menu option yet. A production
adapter must establish directory identity, restore its cursor between panels,
and provide exact names and its own I/O path. The existing MyDOS commands retain
their 15-byte filename buffer. The source-resolution boundary accepts a separate
caller-owned buffer for longer names. Details are in the
[directory contract](../../surveys/tn/directory-model-contract.md).

`MakeJmp` uses CASE; command gates, key bindings and Copy's transfer algorithm
remain. TNDBG retains G/Dbg and its counters. Its saved entry pointer now refers
to native directory data; its raw-entry display is a rendered projection.

Both roots retain `ORG $2C00`. `LIB.ACT` places `screen` at `$E6` and `allocp` at
`$E8` with the existing allocation SETs. Final `SET BUFFER=*` follows generated
code, static homes and deferred arrays. Small state copies use
`MovePage(..., SIZEOF(PanelState))` because whole-record assignment still has a
[backlogged classic placement issue](../../docs/BACKLOG.md#classic-record-copy-scratch-placement-with-set).

## Memory

Measured after the [size reduction plan](../../surveys/tn/size-reduction/README.md),
using compiler `1d67c984` and the cartridge runtime:

| Program/backend | Load file | Deferred arrays | Code + static/deferred workspace | Copy start | Bytes to `$A000` |
| --- | ---: | ---: | ---: | --- | ---: |
| TN classic | 18,095 | 2,798 | 20,881 | `$7D91` | 8,815 |
| TN MIR6502 | 16,807 | 2,854 | 19,649 | `$78C1` | 10,047 |
| TNDBG classic | 21,586 | 2,798 | 24,372 | `$8B34` | 5,324 |
| TNDBG MIR6502 | 20,544 | 2,854 | 23,386 | `$875A` | 6,310 |

Compared with the plan's baseline, MIR6502 TN/TNDBG save 458/481 load bytes
and gain the same amount of copy RAM. Classic saves 257/255 bytes. CI load-file
ceilings are 18,300/21,750 bytes for classic TN/TNDBG and 17,000/20,750 for
MIR6502. CPU observations and exact instruction counts are separate from these
size budgets.

The metadata, order table and summary occupy the first 87 bytes of DirBatch;
the 64 entries follow. The earlier layout migration saved 300 load-file bytes
and restored 300 bytes of copy space in both MIR6502 programs. Classic saved
three bytes in TN and one in TNDBG. Capacity and record sizes are unchanged.

Standalone MIR6502 TN now uses **17,106 bytes**. Its
copy buffer starts at `$79EC`, leaving 9,748 bytes below the same `$A000`
comparison ceiling.

Load-file counts include headers and the execution vector. Workspace is the
entire reservation from `$2C00` to the copy buffer, including compiler homes
and spills. Copy space is `MEMTOP-buffer` at runtime; `$A000` is the comparison
ceiling here. OS/runtime memory, stack, screen and popup allocations are outside
that reservation and retain their existing allocation scheme.

The per-panel model uses **1,443 bytes**: cache 1,367, bitmap 8, selection 14,
location 49 and view state 5. This is 86 bytes more than the old layout, below
the planned 1,721-byte ceiling. The conservative measured shared buffers and
adapter homes/spills occupy 428–441 bytes, below the 512-byte budget. Most of
the overall growth is code for the new boundaries, validation and wide indexes.

Reproduce the measurements and create ignored build artifacts with:

```sh
cargo build --locked --bin actionc
python3 surveys/tn/measure-directory.py
```

The script writes listings, executables and `memory.json` beneath
`target/tn-directory/final/`. `--source-dir` and `--out-dir` support comparisons;
`--reuse` measures existing artifacts without compiling them again.

## Verification

The three VM behavioral tests below run in normal CI with the reordered
DirBatch layout. Their independent host-side layout constants check the header
at byte 0 and entries at byte 87. Case-insensitive symbol lookup is also checked
with LF and CRLF listings.

The focused checks compile both complete programs and reuse each build for
command dispatch, panel transitions, 0/1/63/64-file listings, exact rows/names,
tag-all behavior, copy continuation, disk prompts, guarded paths and a simulated
large directory. Paging tests cover 63/64, 255/256, 1,023/1,024, the 4,096-entry
selection limit, cursor loss and failed end scans. Model and reader fixtures
also compile LF and CRLF source versions.

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_dispatch --test tn_directory_model --test tn_mydos_directory
cargo test --locked --test nir_storage_analysis tn_exposes_high_value_scalar_promotion_candidates
cargo test --locked --lib tn_deferred_storage_starts_after_final_mir_bytes
```

The behavioral checks pass, including header publication, entry addresses,
ordering, rendering and backing-store guards. Full-root LF/CRLF builds produce
identical XEX images in all four configurations. The separate CPU profile
measures sorting, drawing, tag mutation and publication with substituted services.

A [MyDOS smoke check](../../surveys/tn/size-reduction/mydos-smoke.json) in
Atari800 exercised real OS/CIO root enumeration and rendering, tagging and panel
switch/restoration with MIR6502 TN on a disposable ATR. The two filenames match
the raw directory and the ATR remained byte-identical. File modification commands,
TNDBG disk behavior and live FujiNet still need release acceptance. Before a
release, check both roots with 64 files, four subdirectory levels, tags, copy,
rename/delete and panel switches on disposable disks.

The separate legacy `check-stability.sh` currently stops while compiling the
unchanged archived source: BYTE-returning `Fnamecmp` rejects `RETURN(-1)` at
line 336. Its modern size budget has been adjusted for this intentional source
migration; the maintained classic build is within that budget. This does not
replace the focused behavioral and storage checks above.
