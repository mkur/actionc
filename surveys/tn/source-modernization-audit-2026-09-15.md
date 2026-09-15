# TOMS Navigator source modernization audit

Audit baseline: `1514c41`, 2026-09-15. Scope: existing TN behavior only.
No application, library or compiler source was changed by this audit.

Follow-up: constants/ORG, CASE command dispatch and PanelState are now implemented. See the
[sample notes](../../samples/tn/README.md) for the retained allocation controls,
assembly restrictions, measured sizes and dispatch VM coverage. Constants/ORG
alone preserved the audit binaries; CASE changes their size. The findings below
describe the audit baseline; the other proposed refactors remain unimplemented.

Design follow-up: the [directory handling implementation plan](directory-handling-implementation-plan.md)
supersedes the directory-row overlays and packed-path preservation proposed
below. FujiNet requires bounded 64-entry batches with selection independent of
the cache; MyDOS retains its complete 64-file batch. These changes are planned,
not implemented. The original findings remain as migration evidence.

Reviewed the complete maintained [TN.ACT](../../samples/tn/modern/TN.ACT),
[TNDBG.ACT](../../samples/tn/modern/TNDBG.ACT), and shared
[LIB.ACT](../../samples/tn/modern/LIB.ACT): 1,300, 1,574 and 879 lines.
TN identifies itself as 1.25; TNDBG retains its 1.23 identity and additional
diagnostic behavior. At the baseline, neither root nor LIB declared TYPE, CONST,
CASE, VOLATILE or USE. Much of the useful modernization is making existing
data layouts and control flow explicit.

This is a source audit. The existing [NIR optimization note](../../docs/NIR_TN_OPTIMIZATION_NOTE.md)
and MIR listing audits address compiler optimization separately.

Implementation update: the directory migration is complete in both maintained
roots. The [sample notes](../../samples/tn/README.md) document the shared native
data model, source-owned paths, synthetic paging acceptance and memory costs.
The historical shapes and source line numbers below describe the audit baseline.

## Findings, ranked by value

### 1. Make panel state an explicit record

Locations: TN.ACT:33–40, 379–411, 478–485; corresponding SetWin code in TNDBG.

SetWin saves/restores eight bytes starting at `@files` using MovePage. These
bytes span six BYTE declarations and the following CARD declaration. The
current classic listing confirms their adjacency; it is an implicit source
layout contract, not an arbitrary eight-byte scratch buffer.

```action
TYPE PanelState=[
  BYTE files,nestLevel,drive,tagMask,selected,row
  CARD directory
]
PanelState active
PanelState ARRAY saved(2)
```

This shape is eight bytes on Atari and supports ordinary whole-record
assignment in both backends. Replace the accidental declaration adjacency with
named fields and explicit state copies. Keep the separate OS drive/current
directory synchronization visible. Do not copy an entire panel's directory
storage when only these eight bytes need saving.

Preserve SetWin's persistent selected/previous panel state, initialization path,
and ordering of directory refresh versus state save/restore. In current source,
`winnum=0` selects the `rp_*` buffers and `winnum=1` selects `lp_*`; keep that
mapping even if new names are clearer. The independent `tagged(2)` counters
must remain coordinated with the selected panel.

Implemented follow-up: both roots now have `PanelState active` and two saved
record values, accessed through typed pointers in SetWin. Callers use named
fields; Copy's local file count and other shadowing parameters remain local.
The copy length comes from `SIZEOF(PanelState)`. Whole-record assignment exposes
the classic/SET placement issue documented below, so this slice retains MovePage
for the two copies. Focused VM transition checks pass on both the pre-record
sources and the refactor in both backends. See the sample notes for coverage
and measured sizes.

### 2. Replace executable-looking command data with CASE

Locations: TN.ACT:988–1009 and 1026–1038, 1124–1128; TNDBG:1258–1306.

TN maintains three coupled representations: the `VADRSCMFQ` key string,
the low/high-byte address data in the fake Jmp procedure, and MakeJmp's CARD
array plus assembly indirect jump. TNDBG inserts G/Dbg before Q/Quit.

The smallest first change is a CASE in MakeJmp that retains the existing
1-based command indexes and calls the same nine routines. A later step can
use named command constants or an enum at the key-decoding boundary. It does
not need a callback registry or a new command system.

Preserve Handle's existing gates: with no files, only indexes greater than 6
are dispatched; Copy's C key exits the inner loop after dispatch. Handle also
processes movement and global keys in successive phases. Replacing its whole
IF chain with one CASE would require retaining that ordering explicitly.

An isolated full-TN CASE replacement compiled in both advertised backends:
**+45 load-file bytes in classic, +7 in MIR6502**. It has not been execution
tested. Direct calls through a typed procedure-pointer array compile in MIR6502
but currently fail in classic; CASE is the available common path.

Implemented follow-up: both roots now use CASE in MakeJmp, while Handle is
unchanged from the constants/ORG checkpoint. Focused VM checks cover all command
routes, empty-directory gates, unknown keys, repeated calls, Copy's exit and
stack balance in both advertised backends. UI and command routines are stubbed
at their compiled entry points; the test executes the actual dispatcher and
Handle code. See the sample notes for final sizes, including TNDBG.

### 3. Give directory rows a record/union view

Historical option, superseded by the directory handling plan linked above.

Locations: TN.ACT:26, 287–298, 310–329, 344, 366, 424–468.

The sorted `v` array holds addresses of 18-byte rows. Consumers use Value,
Store and numeric offsets to interpret tag, marker, name, extension and sector
text. The same backing is first filled with CIO directory text and then
converted in place to screen codes.

```action
TYPE DisplayFields=[
  BYTE tag BYTE marker BYTE ARRAY name(8)
  BYTE separator1 BYTE ARRAY extension(3)
  BYTE separator2 BYTE ARRAY sectorText(3)
]
TYPE DirectoryRow=UNION [DisplayFields fields BYTE ARRAY bytes(18)]
TYPE DirectoryStorage=[DirectoryRow ARRAY rows(65) BYTE readTail]
```

The union gives an explicit raw view for loading/conversion and named fields
for the converted display row. It is not a tagged VARIANT: receiving file data
must not overwrite a language-managed tag. `$0A` and `$1A` are the markers
tested by the existing converted-row code, not a new DOS attribute-bit model.

Keep the existing CARD address table initially and load an address into a
`DirectoryRow POINTER` before accessing fields. That combination compiled in
both backends. The more ambitious array of `DirectoryRow POINTER` values
exposed a compiler issue; see the verified limitations below.

Important layout detail: Input requests **19 bytes**, while successive row
addresses differ by **18**. The existing 1,171-byte allocation is
`65*18+1`, including the extra last byte. Preserve this overlap, the trailing
free-space summary row at `v(files)`, row ordering and exact screen bytes.
IsTagged returns the existing 0/`$7F` mask; changing it to a normalized boolean
would alter Tag's XOR behavior.

### 4. Express fixed panel/path storage with embedded and multidimensional arrays

Historical layout-preserving option. The directory handling plan instead makes
path storage source-specific and separates the directory cache from selection.

Locations: TN.ACT:382–410 and 1060–1071.

The two panels duplicate 47-byte path buffers, 1,171-byte directory buffers
and 65-entry address tables. The path layout is five CARD sector numbers,
one empty root-string length byte and four nine-byte counted names:

```action
TYPE PathStorage=[
  CARD ARRAY sectors(5)
  BYTE rootLength
  BYTE ARRAY names(4,9)
]
PathStorage ARRAY paths(2)
```

This remains 47 bytes per panel on Atari. It replaces `pathBuf+=10`, `+=1`
and repeated `+=9` with named members and indexes. A record array for each
panel's backing storage is preferable to repeatedly selecting parallel lp/rp
variables. Keep active access through pointers or indexes; avoid large
aggregate-value parameters and copies.

Preserve depth four, 64 file slots plus the summary slot, and all existing
capacities. Do not enlarge buffers as part of modernization. Characterize
eight-character directory names before changing layout: Handle copies through
the filename's dot inclusively, one byte beyond its recorded string content
length. That write needs explicit attention when separating the old packed
storage; this audit does not change its behavior or claim an execution diagnosis.

### 5. Replace parallel copy tables with an array of records

Locations: TN.ACT:814–819, 872–881, 896–920; TNDBG's corresponding Copy/MarkCopy.

```action
TYPE CopyChunk=[BYTE fileIndex CARD length]
CopyChunk ARRAY chunks(16)
```

The record is three bytes on Atari, so its payload remains 48 bytes, matching
`copytab(16)` plus `lentab(16)`. `chunks(k).fileIndex` and `.length` make the
association explicit and remove the requirement to update two arrays together.
The shape and indexed accesses compile in both backends. Addressing a
three-byte stride can cost more than the existing separate byte/word arrays;
measure full Copy code and workspace before adopting it.

Preserve chunk order, actual transferred lengths, continuation of a partly
read file, destination-open state across batches, tag removal and source/destination
disk prompts. `flag=ioerr!$88` preserves a byte value; TNDBG displays that raw
derived value. A new enum must not silently normalize it to 0/1.

### 6. Use enums/variants for internal choices with real alternatives

Locations: FindNext (TN.ACT:372–377), SetWin/SwapWin/Dir (379–499), callers in
Xloop/Copy/Handle, and LIB.ACT:780–786.

- FindNext can return `NONE | ITEM [BYTE index]` rather than `$FF`. Index 0
  remains a valid result. All callers must migrate together, including the
  intentional `$FF+1` wrap that starts the Copy search at zero.
- SetWin's `drvnum` combines three operations: 0 means switch without refresh,
  `$FF` means reload, and another value selects a drive. An internal variant
  `SWITCH_ONLY | RELOAD | SELECT_DRIVE [BYTE unit]` states that distinction.
  Keep the same accepted drive values and side effects; add no unit validation.
- An enum suits command identity or a proven finite copy phase. It does not
  suit screen tag masks or arbitrary CIO status bytes without retaining their
  current representation.

A representative FindNext variant return and CASE consumer compiled in both
backends. Variant parameters/results have storage and copy costs, so follow
the simpler data/dispatch work with measurement. There is no need to introduce
generic Option/Result frameworks throughout TN for these few boundaries.

### 7. Adopt constants, volatile access and scoped scratch selectively

Locations: LIB.ACT:2–24, 90–92, 678–695; TN.ACT:97–130, 194–202, 617–650,
727–755, 1194–1226 and 1271–1288.

- Replace textual DEFINE constants and repeated CIO commands, status codes,
  geometry and capacity literals with typed CONST declarations. Keep BYTE/CARD
  widths and encodings: EOL 155, EOF 136, a `$7F` tag mask, and CIO commands
  are different domains. Retire `nil="0"` as pointer/sentinel consumers become
  typed; do not mechanically substitute pointer NIL into CARD-returning APIs.
- GetAnyKey's `$D20F` read is a direct VOLATILE candidate. Existing
  `ATARI.POKEY.SKSTAT` and `ATARI.OS` bindings can be reused after introducing
  a named module; a local VOLATILE declaration works without that migration.
  Retain the `$04` test and key-wait behavior. Do not confuse raw OS key codes
  with Getchar's returned ATASCII characters.
- Store/Value can use a BYTE POINTER and ordinary dereference. An isolated
  whole-TN replacement compiles but adds **46 bytes in classic / 23 in MIR6502**.
  Prefer replacing their directory consumers with named fields; readability
  does not establish a code-size or speed improvement.
- IsProtected/IsDirectory and the final Fnamecmp predicate can use comparison
  values to express existing 0/1 results. Fnamecmp's directory ordering and
  comparison start positions must remain unchanged.
- Introduce local loop variables and LET for genuine snapshots after tracing
  the shared i/j/h/s scratch dependencies. In PopUp, for example, one saved
  FindItem result can replace two identical searches. Preserve nested-call
  clobbers, loop widths and values observed by TNDBG.

Do not turn persistent locals into per-call initialization: Format's `last`,
SwapScr's `firstTime`/screen buffer, InitPanels' `times`, and SetWin's panel
storage survive calls deliberately. Likewise, Sort's BYTE j underflow followed
by `j<$80` is an intentional termination idiom, not an interchangeable signed
comparison without checking the full loop domain.

### 8. Share implementation between TN and TNDBG through named modules

Locations: both roots' INCLUDE and bare MODULE sections; duplicated routines
throughout TNDBG; LIB.ACT's memory, display, CIO, window and menu sections.

The bare MODULE directives do not provide namespaces. Named modules can make
panel state, directory operations, UI/menu routines and the CIO bridge private
behind explicit exports, while leaving small normal/debug root programs.
Start with one cohesive component rather than moving all 3,753 lines at once.

Preserve existing debug counters, snapshots, G command, error-time debug window,
banners and compatibility annotations on TNDBG.Sort/SetWin. The roots are not
currently identical programs with only logging switched on. Identify these
differences before sharing implementations.

Module extraction must retain fixed zero-page cells used by assembly, explicit
Init ordering, deferred array storage and the final `SET BUFFER=*` relationship
to available copy memory. A module import does not initialize state implicitly.

## Low-level code to preserve during the first slices

- LIB.MovePage copies backward and interprets a zero BYTE length as 256 bytes.
  A conventional zero-length copy or forward loop is not an equivalent replacement.
- Keep CalcAdr, Block, GetImage/PutImage, keyboard-vector entry and their declared
  machine effects until targeted execution/cost comparisons justify replacements.
- TN uses the CIO bridge at `$E456`, including MyDOS file and directory commands.
  The new SIO/NET library does not replace that file API. A later typed IOCB
  record/union could name the existing offsets while retaining a small assembly
  bridge for the register ABI.
- `_Cio` branches to ErrorHandler, and NavError resets the hardware stack before
  re-entering navigation. EOF returns through a different path. Quit and file
  launch also have deliberate nonlocal transfers. Replacing these with ordinary
  Result returns would be a separate control-flow migration needing execution
  coverage, not a mechanical reuse of the new SIO result type.
- Keep the MyDOS version-specific addresses, file/screen encodings, load/exit
  paths and source/destination I/O ordering. Bug fixes discovered during
  characterization should be separated from behavior-preserving refactors.

## Verified compiler limitations

Temporary independent reproducers were compiled at the audit baseline:

| Shape | Classic | MIR6502 | Audit recommendation |
| --- | --- | --- | --- |
| CASE dispatcher in full TN | Compiles | Compiles | Suitable first dispatch change |
| `PROC POINTER ARRAY handlers(1)()=[@Choose]`, then `handlers(0)()` | Codegen rejects call | Compiles | Do not choose this as the common-backend dispatcher yet |
| `Item POINTER ARRAY refs(1)`, assigning/reading a record pointer element | Codegen treats assignment as record copy and rejects it | NIR extent/aggregate-store diagnostics | Keep CARD address tables with typed pointer views initially |
| Records, inline multidimensional arrays, union row views, CARD address table and variant result combined | Compiles | Compiles | Available building blocks; not a full-program behavior proof |
| USE in a legacy root without a named MODULE | Rejected | Rejected | Introduce a named module before importing embedded hardware bindings |
| Whole-record assignment after legacy zero-page allocation SETs (found during PanelState implementation) | Scratch storage conflicts with the restored code pointer | Compiles | Retain size-based MovePage copies in TN until placement is fixed |

The record-pointer-array failure also reproduces with a simple record containing
BYTE tag and CARD length, independently of TN and UNION. An attempted local
procedure-pointer intermediary in TN did not resolve the classic dispatch
failure. No compiler changes were made as part of the audit.

The two independent compiler-gap reproducers are small enough to retain here:

```action
BYTE value
PROC Choose() value=7 RETURN
PROC POINTER ARRAY handlers(1)()=[@Choose]
PROC Main() handlers(0)() RETURN
```

```action
TYPE Item=[BYTE tag CARD length]
Item data
Item POINTER p
Item POINTER ARRAY refs(1)
PROC Main() p=@data refs(0)=p p=refs(0) p.tag=7 RETURN
```

Compile each separately with `--mode optimized --runtime cart` and
`--mode mir6502 --runtime cart` to reproduce the table's results.

The record-copy/SET interaction has this independent reproducer. Classic
prepends the aggregate-copy scratch area, then rejects the restored allocation
cursor because it points back into that emitted area. The fix and acceptance
checks are tracked in the [compiler backlog](../../docs/BACKLOG.md#classic-record-copy-scratch-placement-with-set):

```action
ORG $2C00
SET $E=$E6
SET $F=0
BYTE POINTER screen
CARD POINTER allocp
SET $E=$2C00
SET $491=$2C00
TYPE State=[BYTE a,b,c,d,e,f CARD g]
State first,second
PROC Main()
  first=second
RETURN
```

## Baseline and validation

Both maintained roots compiled using their advertised cartridge-runtime matrix:

| Root | Modern classic load-file bytes | MIR6502 load-file bytes |
| --- | ---: | ---: |
| TN.ACT | 10,488 | 9,920 |
| TNDBG.ACT | 13,717 | 12,979 |

Commands used `target/debug/actionc --mode optimized|mir6502 --runtime cart`,
with output and listing paths under `target/tn-modernization-audit/`. The source
retains its `$2C00` origin. Counts above include load-file headers and INITAD;
they are not total runtime RAM requirements. Isolated source copies/probes are
also in that ignored directory. A type probe's emitted SIZEOF values confirmed
8-byte panel state, 47-byte path storage, 18-byte rows, 1,171-byte directory
storage and three-byte copy chunks.

This audit performed compilation and listing inspection, not interactive TN
execution or proof of behavioral equivalence. No full compiler suite was needed
because only this note and its index links were added.

Before implementation, establish focused TN behavior tests for panel switching,
directory rows/sorting/tagging, menu command gates, copy continuation and EOF,
window restoration and error recovery. Include both TNDBG-specific behavior and
normal TN, and normalize host text before LF/CRLF instrumentation. Existing
build/size tests and manual [VM recipes](../../docs/ACTIONC_VM_USAGE.md) do not
provide that complete interaction coverage.

Source refactors will also require reviewing the TN-specific assertions in
`tests/nir_storage_analysis.rs` and
`mir6502::tests::tn_deferred_storage_starts_after_final_mir_bytes`: the latter
currently names every deferred-array size explicitly. Update intentional source
layout expectations while retaining the guarantees against code/storage overlap.

## Suggested order

This was the audit's original order. Follow the directory handling plan for the
next directory/path work; its characterization and migration slices replace
step 2's overlays and packed-layout constraint.

1. Capture the relevant behavior, then adopt typed constants, volatile keyboard
   polling and CASE dispatch. Keep this first slice small.
2. Introduce PanelState, path storage and directory row views, preserving all
   capacities, byte layouts and state transitions.
3. Evaluate copy-chunk records and the small search/refresh variants against
   copy continuation, error behavior, static workspace and emitted size.
4. Extract the proven components into named modules and share them between the
   normal/debug roots while preserving their existing differences.

None of these steps adds commands, filesystem support, networking, larger
directories or new UI behavior.
