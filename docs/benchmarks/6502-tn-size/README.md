# TN source size audit

2026-11-13. Space takes priority over execution time. The baseline is the
maintained TN/TNDBG/LIB source at `5d002c27`, compiled with the same compiler
as the candidate. This change edits the application and its private library;
it adds no compiler optimizations or TN-specific compiler rules.

## Changes

- Read directory tags through the existing `Value` routine, and share marker
  addressing between `IsProtected` and `IsDirectory`. Keep field offsets tied
  to the `DirectoryRow` type through `OFFSETOF`.
- Select the source marker before calling `Internal`, and select the last
  sector-text index before its three-column loop. Extract row conversion into
  `FormatDirectoryRow`; it remains Action! source.
- Initialize the five path-name pointers for each panel at load time. Switching
  panels now selects its table, replacing the loop that rebuilt a shared table.
  Deferred path and directory buffers retain their capacities.
- Dispatch through a guarded callable-entry table. Zero and every ordinal
  beyond the table return without calling a handler. Valid entries tail-call
  the existing routines through `$A0/$A1`, without constructing return addresses.
  Byte pairs hold the little-endian addresses without an array descriptor.
- Share the block-I/O setup between `Bget` and `Bput`. Restore A from ICCOM
  before CIO, including zero-length requests where A matters.
- Share the pointer-advance tail between `GetImage` and `PutImage`, preserving
  the row count in X and the existing screen/buffer carry handling.
- In TNDBG, share hex conversion together with character output. This avoids
  duplicating the conversion through automatic leaf inlining. All 256 byte
  values retain their two-digit output and final `h` value.
- Share decimal digit extraction and output using repeated subtraction for
  divisors 100 and 10. Compose the calls directly instead of assigning their
  results back to the parameter home. This removes TN's only use of division
  and remainder, allowing MIR6502 to omit both arithmetic helpers entirely.

Decimal output preserves its existing zero suppression, including the quirk
that 100 prints `10`. Correcting that behavior would be a separate change.

TNDBG retains the explicit compatibility profile on `SetWin` and on the
extracted part of that routine. Menus, messages, commands, directory limits,
the 2 KB arena and its failure checks remain present.

## Measurements

All figures compare complete XEX files, including initialized data and tables.
The [scorecard](results.json) records all eight configurations, routine sizes,
source hashes and image hashes. Reserved RAM is the distance from `$2C00` to
the copy-buffer start patched by `SET BUFFER=*`, including deferred storage.
Reducing that distance gives the file copy operation more buffer space.

| Program | Backend/runtime | Before XEX | After XEX | Saved | RAM freed |
| --- | --- | ---: | ---: | ---: | ---: |
| TN | MIR6502 cartridge | 10,290 | 10,000 | 290 | 300 |
| TN | MIR6502 standalone | 10,415 | 10,107 | 308 | 318 |
| TNDBG | MIR6502 cartridge | 13,373 | 13,045 | 328 | 338 |
| TNDBG | MIR6502 standalone | 13,498 | 13,152 | 346 | 356 |
| TN | Classic optimized cartridge | 11,061 | 10,635 | 426 | 436 |
| TN | Classic optimized standalone | 11,289 | 10,863 | 426 | 436 |
| TNDBG | Classic optimized cartridge | 14,302 | 13,872 | 430 | 440 |
| TNDBG | Classic optimized standalone | 14,530 | 14,100 | 430 | 440 |

For TN MIR6502 cartridge, the copy buffer moves from `$5F10` to `$5DE4`,
giving it 300 more bytes below the cartridge window. TNDBG gains 338 bytes.

The following isolated source experiments informed the selection. These are
TN MIR6502 cartridge deltas against the original source; they are not additive
because combining changes affects code layout and storage.

| Isolated change | XEX bytes saved |
| --- | ---: |
| Shared row-marker access and `Value` reads | 64 |
| Precomputed path-name pointers | 36 |
| Select marker before conversion | 18 |
| Callable-entry dispatch table, initial guard | 17 |
| Shared image-pointer advance | 15 |
| Shared block-I/O setup | 13 |
| Extract directory-row conversion | 7 |
| Select sector-text end index before loop | 3 |

The final dispatch guard saves a further two bytes. In the combined TN
MIR6502 candidate, the shared subtraction routine saves 106 bytes; nesting its
calls saves another eleven, and byte-pair dispatch entries save four. The
isolated TNDBG hex change saves 47 bytes on MIR6502. These figures include
the helper/table costs, rather than counting only instructions removed from
callers.

Rejected alternatives include inlining filename comparison (52 bytes larger),
moving short-lived scalars into an existing zero-page scratch byte (14 larger
alone, five larger in the combined candidate), and preserving Y inside
`Internal`/`Ascii` (no saving, or four larger). Disabling leaf inlining, constant
unrolling and tiny-fill expansion produced no TN saving. On the original
TNDBG source, disabling leaf inlining saved 41 bytes; sharing hex output saved
more without changing compiler options. The earlier
[combined window frame experiment](../6502-tn-window-frames/README.md) remains
rejected because it increased complete image size by 363–364 bytes.

After these changes, `Handle`, `Copy` and `SetWin` remain the largest TN
MIR6502 routines. They are the next candidates for source sharing and analysis
of generated addressing and argument code. This audit does not establish a
global minimum, and removes no functionality to reach its result.

## Reproduction and validation

```sh
python3 tools/compare6502/tn/size_audit.py --check --ui --mads
```

The tool reconstructs frozen baseline sources with `git show`, copies current
maintained sources into an isolated directory, and compiles all 16 before/after
images. `--source-dir` can substitute another candidate. Bulk sources, images,
listings and execution logs remain under ignored `build/tn-size/`.

Qualification covers:

- Classic optimized and MIR6502, cartridge and standalone, for TN and TNDBG.
- 5,120 block-I/O calls with independent IOCB/register expectations: all eight
  channels, buffer boundary addresses, zero/byte/word/max lengths, one CIO
  entry per call and balanced CPU stack. CIO is intercepted before actual I/O.
- 4,096 dispatcher calls: every byte value in every image, checking the selected
  handler or absence of a call, plus balanced CPU stack.
- 2,104 hex-output calls, checking actual converted screen bytes and guards.
- 4,096 decimal-output calls, covering all 256 inputs in every image and
  checking the original output, cursor, guards and final shared `i` value.
- The maintained checked-arena oracle on both source versions: 1,120
  success/failure calls each, both backends/runtimes and actual LF/CRLF source
  parsing. Windows retain their screen restoration, arena marks and UI peaks.
- The maintained cartridge UI oracle on both source versions and backends:
  keyboard dispatch, menus, panel state, directory conversion/navigation and
  copy behavior. Executed images are bound to the measured image bytes.
- Unchanged `[8192,10240]` normal-return ranges for `ArenaCursor` and `Alloc`
  in raw and optimized NIR.
- 32 byte-identical MADS roundtrips, covering every image with LF/CRLF listings.
- The TN storage-placement unit test and the `nir_storage_analysis` and
  `tn_checked_arena_controls` integration targets. The storage test checks
  initialized path-table relocations against the final deferred-buffer addresses.

The changed deferred-range expectation removes the former ten-byte mutable
path-name table. It is an intentional application layout change; the compiler
IR contracts and printer output formats are unchanged.
