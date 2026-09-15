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

The proposed [directory handling implementation plan](../../surveys/tn/directory-handling-implementation-plan.md)
preserves MyDOS's existing 64-file batch while introducing a bounded cache and
independent selection for future FujiNet directories. The directory/path
architecture described there has not been implemented; the layout below is
still the current source layout.

Both maintained programs declare `ORG $2C00`. `LIB.ACT` still uses the legacy
allocation `SET`s to place `screen` at `$E6` and `allocp` at `$E8`, then restore
normal allocation at `$2C00`. `ORG` selects the program origin; it does not
replace those sequential allocation controls. The final `SET BUFFER=*` binds
the copy buffer after generated code and deferred storage.

Typed constants name the existing CIO/MyDOS commands, open modes, status bytes,
screen dimensions, tag/marker encodings and buffer capacities. The packed panel
state remains eight bytes; directory reads remain 19 bytes with an 18-byte
stride and 1,171 bytes per panel. These constants document the current layout,
not configurable limits: the screen templates and packed storage still depend
on those dimensions.

Raw menu blocks retain literal `$00` terminators, and `quickmul+24` retains its
numeric assembly relocation offset. These older forms do not accept typed
constants in those positions. The constants/ORG slice alone preserved the
original cartridge load files byte for byte in both modes, verified with LF and
CRLF source copies.

`MakeJmp` now uses `CASE` to call each command directly. `Handle` retains its
key strings, command indexes, empty-directory restrictions and Copy's inner-loop
exit. TNDBG keeps its additional G/Dbg command before Q/Quit.

`PanelState` groups the active file count, directory depth, drive, tag mask,
selection, screen row and directory sector. Both saved panels are records too;
each state remains eight bytes. Consumers use named `active` fields, while
tagged-file counters and MyDOS drive/current-directory bindings remain separate.
SetWin preserves the existing save-before-restore order, including same-panel
switches and refreshes.

The two copies use `MovePage` with `SIZEOF(PanelState)`. Ordinary whole-record
assignment currently introduces classic compiler scratch storage before the
legacy allocation `SET`s, causing a [backlogged placement conflict](../../docs/BACKLOG.md#classic-record-copy-scratch-placement-with-set);
the size-based copies avoid that existing compiler issue. The saved records
now live in the load segment instead of two deferred byte arrays, with the same
16-byte payload.

| Program | Optimized classic bytes (change) | MIR6502 bytes (change) |
| --- | ---: | ---: |
| TN.ACT | 10,549 (+61) | 9,943 (+23) |
| TNDBG.ACT | 13,794 (+77) | 13,048 (+69) |

Changes are relative to the constants/ORG checkpoint. Counts include load-file
headers and INITAD, not total runtime memory.

The [dispatch VM test](../../tools/vm-runtime-tests/tests/tn_dispatch.rs) compiles
both full programs in their two advertised cartridge modes and runs 96 scenarios.
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
