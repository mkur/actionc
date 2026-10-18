# TOMS Navigator Samples

This directory contains the maintained TOMS Navigator sources used for current
compiler work.

```text
modern/
  maintained source copies for the modern profile and MIR6502 backend work
```

The original TN ATR, extracted archived sources, and prebuilt `TN.COM` live
under `corpora/tn/`. Stability scripts, fixtures, and original compiler symbol
dumps live under `surveys/tn/`.

The [source modernization audit](../../surveys/tn/source-modernization-audit-2026-09-15.md)
identifies refactors using current language features without changing TN's
functionality.

The maintained sources use the directory implementation from `8ccdc683`,
immediately before the large-directory refactor began at `06cef61f`. They retain
typed constants, `ORG`, CASE dispatch and `PanelState`. The subsequent small
state/typing cleanup is measured in the [current modernization review](../../surveys/tn/source-modernization-audit-2026-09-15.md#current-review-2026-10-18).
The batch cache, wide directory ordinals, independent tag bitmap and paging work
are preserved on **`tn-ng`**, initially at `4baaac89`. Continue that work there.
The [directory migration plan](../../surveys/tn/directory-handling-implementation-plan.md)
and its measurements describe `tn-ng`, not the maintained implementation here.

Both maintained programs declare `ORG $2C00`. `LIB.ACT` still uses the legacy
allocation `SET`s to place `screen` at `$E6` and `allocp` at `$E8`, then restore
normal allocation at `$2C00`. `ORG` selects the program origin; it does not
replace those sequential allocation controls. The final `SET BUFFER=*` binds
the copy buffer after generated code and deferred storage.

Typed constants name the existing CIO/MyDOS commands, open modes, status bytes,
screen dimensions, tag/marker encodings and buffer capacities. The packed panel
state is nine bytes; directory reads remain 19 bytes with an 18-byte
stride and 1,171 bytes per panel. These constants document the current layout,
not configurable limits: the screen templates and packed storage still depend
on those dimensions.

Raw menu blocks retain literal `$00` terminators, and `quickmul+24` retains its
numeric assembly relocation offset. These older forms do not accept typed
constants in those positions. The constants/ORG slice alone preserved the
original cartridge load files byte for byte in both modes, verified with LF and
CRLF source copies.

`MakeJmp` takes a `CommandKind` enum and dispatches with named CASE labels.
`Handle` converts the existing key-string lookup into that type and retains
empty-directory restrictions and Copy's inner-loop exit. TNDBG keeps its
additional G/Dbg command before Q/Quit. `GetAnyKey` explicitly declares its
POKEY keyboard-status cell `VOLATILE`. These typing changes add no binary bytes.

`PanelState` groups the active file count, directory depth, drive, tag mask,
selection, screen row, directory sector and tagged-file count. Both saved panels
are records too; each state is nine bytes. The tagged count at byte offset eight
replaces the separate `tagged(2)` array, so it travels with the rest of the state.
MyDOS drive/current-directory bindings remain separate live OS cells.
SetWin preserves the existing save-before-restore order, including same-panel
switches and refreshes.

The two copies use `MovePage` with `SIZEOF(PanelState)`. Ordinary whole-record
assignment currently introduces classic compiler scratch storage before the
legacy allocation `SET`s, causing a [backlogged placement conflict](../../docs/BACKLOG.md#classic-record-copy-scratch-placement-with-set);
the size-based copies avoid that existing compiler issue. The two saved records
occupy 18 bytes in the load segment. A switch copies one extra byte in each
direction, while direct tagged-count accesses make the programs smaller overall.

`DirectoryRow` gives the existing 18-byte display row named fields for its tag,
marker, name, extension, separators and sector text. `IsTagged`, `IsProtected`
and `IsDirectory` use typed pointers into the existing directory buffer through
the `v` address table. There is no row copy or additional per-file storage.
The fields retain their screen-byte encoding, and `IsTagged` still returns the
raw zero/`$7F` mask used by Tag's XOR. Applied after the panel-state cleanup at
`db34013a`, these views add 38 bytes to each MIR6502 build, 53 bytes to classic
TN and 63 bytes to classic TNDBG. Reserved memory grows by the same amounts.

`PopUp` in shared `LIB.ACT` captures `FindItem(menu, ch)` in a scoped
`LET CARD item`, then checks `item # NO_ITEM` and reuses the address. Each key
returned by Range gets one fresh lookup; a matching shortcut no longer scans
the menu twice.
The explicit `BEGIN`/`END` block keeps the immutable result local to that part
of the loop. Relative to `68687a80`, this adds 3 bytes in classic TN and 1 byte
in each MIR6502 build; classic TNDBG shrinks by 9 bytes. Reserved memory changes
by the same amounts.

Measured with the compiler implementation at `4baaac89`, against the restored
source baseline `1bfb8ea7`:

| Program/backend | Load-file bytes | Change | Reserved bytes from `$2C00` | Copy buffer |
| --- | ---: | ---: | ---: | --- |
| TN classic | 10,590 | +41 | 13,222 | `$5FA6` |
| TN MIR6502 | 9,968 | +25 | 12,750 | `$5DCE` |
| TNDBG classic | 13,835 | +41 | 16,467 | `$6C53` |
| TNDBG MIR6502 | 13,073 | +25 | 15,855 | `$69EF` |

Counts include load headers and INITAD. Reserved memory also includes static
homes and deferred arrays; it changes by the same amount as the load file in
each build. LF and CRLF source copies produce identical XEX images in all four
configurations. Reproduce current memory totals with
`python3 surveys/tn/measure-directory.py --out-dir target/tn-state-review`.

The [dispatch VM test](../../tools/vm-runtime-tests/tests/tn_dispatch.rs) compiles
both full programs in their two advertised cartridge modes and runs 96 dispatch
scenarios.
It executes the real key lookup, dispatcher and Handle loop, with UI and command
entry points stubbed: every command, unknown keys, empty-directory gates,
repeated dispatch, Copy's exit/cleanup ordering and stack balance are checked.
It also checks symbol lookup with LF and CRLF listings, and reuses each build
for the [panel transition checks](../../tools/vm-runtime-tests/tests/support/tn_panels.rs).
Those execute initialization, selection/tagging, panel and same-panel switches,
reloads, drive selection and OS-state restoration, with screen output and CIO
directory input stubbed. They cover both MyDOS directory locations, RAM-disk
selection and TNDBG counters, and passed against both the pre-record sources
and this refactor. Actual disk operations and full emulator interaction are
outside these tests. The focused TN storage-promotion and deferred-storage
checks also pass; the latter verifies all three record extents and placement.

The restored MyDOS characterization also covers 0/1/63/64-file directories,
exact rendering and operation names, tag-all behavior, copy continuation and
disk-swap prompts. It directly checks all three row predicates, including the
raw tag mask after tagging, and verifies that predicate reads leave the row
bytes unchanged. The [menu checks](../../tools/vm-runtime-tests/tests/support/tn_menus.rs)
run nine cases per build through the actual PopUp, lookup, ordinal and menu-bar
routines, with Range input and drawing stubbed. They cover matching shortcuts,
Return/Escape, unknown keys, navigation results, repeated calls and different
menus. They assert one lookup per input key, highlight transitions, position
restoration and balanced CPU/arena stacks. Run the focused checks with:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_dispatch --test tn_codegen_reproducers --test call_predicates --test bitmap_scans
cargo test --locked --test nir_storage_analysis --lib tn_
```

Compiler regressions discovered during the batch migration remain covered by
standalone reproducers. The selection fixture under
`tools/vm-runtime-tests/fixtures/tn/selection-regression.act` is a frozen compiler
input; it is not included by either maintained TN program. Batch/paging-specific
integration tests and profiling remain on `tn-ng` with their implementation.
