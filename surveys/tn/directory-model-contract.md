# TN directory model contract

This implements the [directory migration plan](directory-handling-implementation-plan.md).
The source remains MyDOS during this milestone; a simulated source supplies
larger listings in tests.

## Data and memory

The initial compact entry is 20 bytes: CARD ordinal/position, BYTE flags/marker,
11 native name bytes and three native detail bytes. MyDOS stores the padded
eight-character stem and three-character extension in name; detail preserves
the displayed sector count. The renderer inserts separators and converts text
to screen codes. Remote previews may be truncated, explicitly flagged, and
never used as operation names.

DirBatch contains 64 entries, first/count/valid/end state, a 64-byte order table
and an 18-byte summary: 1,367 bytes. Selection contains a bitmap pointer, CARD
capacity/extent/tagged/generation and four state bytes: 14 bytes. The bitmap is
caller-owned: eight bytes for MyDOS, 512 for the large-source fixture. Large
backings are uninitialized byte arrays with typed pointers; no whole-directory
copy is required to switch panels.

The source-shape probe compiled in modern classic and MIR6502 with the existing
zero-page allocation SETs and ORG $2C00. Aggregate fields precede BYTE field
lists to avoid a parser ambiguity; backing addresses use explicit CARD casts.
Runtime fixtures must validate these addresses and extents, not rely on the
successful compile alone.

The MyDOS adapter now passes independent VM checks for 0/1/63/64 files,
directory/protection ordering, both record alignments, exact names and rendered
rows, and separate summary rendering. Guarded outputs reject short filename
buffers before writing. A 65th file, a missing summary and missing record EOL
leave the cache invalid. These checks run both backends with LF and CRLF text.
Both full roots now use these routines. Their command dispatch, panel
transitions, rows, names, tag-all and copy continuation pass in both backends.
File ordinals and copy references are CARD. TNDBG snapshots render a row from
the native entry; the saved pointer now points at a DirEntry, not screen bytes.

The shared model now passes execution checks in both cartridge backends with
LF and CRLF source: 8/512-byte guarded bitmaps, tag/search boundaries through
4,096 entries, cache eviction/refetch, unknown-length tag-all and exceptions,
end trimming, capacity errors and reset. A 64-entry fixture window is generated
on demand. CARD function results are observed through the Action! result cells
at $A0/$A1; A/X are not a universal public return convention.

Keep local snapshots for pointer-based CARD comparisons. Classic currently
clobbers a comparison operand while preparing the indirect field address.
Compute a compound bit operation's mask before applying it to a pointer target;
the classic path can flatten parentheses in a compound assignment. Also avoid
routine/parameter name collisions in shared source. These limitations are
tracked separately in the compiler backlog; no compiler implementation changes
are included here.

## Characterization baseline

The complete TN/TNDBG programs pass the following independent host-oracle
checks in both cartridge backends before directory migration:

- 0, 1, 63 and 64 file records, followed by summary and EOF.
- Mixed directories, protection, short and eight-character names, empty and
  three-character extensions, and both sector-text/EOL alignments.
- Exact 18-byte rows, summary, first/last screenful and `D:` operation names.
- Mixed manual tagging from both the all-selected and none-selected states.
- Real Copy execution against stateful input/output services, with 256-byte
  transfer memory: 700-byte continuation, a 64-byte file and an empty file.
- A 20-file copy forcing the 16-chunk table boundary and source/destination
  disk prompts. Exact output bytes and one destination open per file are checked.

These checks extend the existing command and panel transition tests. Screen
services and disk I/O are substituted; they are not disk-level emulator results.
The old inclusive path-name copy is an isolated bug fix: for `D:ABCDEFGH.`
the dot is at index 11, so copying through it writes slot offset 9, beyond
the nine-byte counted-name slot. Copying through index 10 writes offsets 1–8
only. The typed-location fixture will guard this boundary at every depth.

Baseline load-file bytes: TN classic 10,549; TN MIR6502 9,943; TNDBG classic
13,794; TNDBG MIR6502 13,048. The existing per-panel backing totals 1,357 bytes.
Subsequent measurements must also account for code and shared scratch growth
before claiming any improvement in copy-buffer space.

## Integrated browsing and locations

PanelState now contains two CARD fields (files and selected) and a BYTE row:
five bytes. Each panel separately owns a 49-byte MyDosLocation with its drive,
directory, four parent sectors, root label and four counted nine-byte name slots.
SetWin captures live OS state before selecting the next location, copies only
the five-byte view state, then activates that location. Refresh reads a new
listing and resets its tags; an ordinary panel switch does neither.
EnterDirectory commits its new path only after CHDIR returns successfully.
Guarded location tests cover maximum-length names at all four depths, parent
navigation, a fifth-level refusal and malformed input without writes.

ReadWindow is the replaceable provider boundary. FetchWindow invalidates the
cache first, checks the result, publishes through BatchPublish and updates the
known extent. A provider must report DIR_CHANGED when it cannot preserve the
listing identity/order, including after cursor loss. That result resets the
selection generation. A simple I/O failure leaves the cache invalid. SetWin's
MyDOS path still reads all 64 entries once; it never pages or sorts partial
windows. A future adapter owns reopening/seeking and identity verification.

The integrated provider fixture exercises real Draw, GoTo, FileRange, GoLast,
PrepareSelection, FindNext and panel activation. It covers 1,089 entries,
forward/backward eviction, viewport crossings, independent panels, wide keys,
unknown-length end discovery, tag-all exceptions and scan failures. A 4,097-entry
listing remains browsable but cannot run a partial tag-all. Repeated cursor
restoration is distinguished from an unrepeatable listing, which invalidates
tags. No whole listing or name table is allocated in Atari RAM.

ResolveEntry receives an output buffer and capacity. The fixture distinguishes
identical previews using both MyDOS-sized operation names and longer exact
names in separate caller-owned storage; too-small outputs remain untouched.
Production Convert still uses a 15-byte MyDOS filename buffer and CIO commands.
A live FujiNet command adapter must supply its own full-name buffer and I/O
path before those commands can accept remote long names. This milestone adds
no remote mutation capability or new source-selection menu.
