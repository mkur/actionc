# TN directory handling implementation plan

Status: implementation complete, 2026-09-15. All six slices are implemented
and committed separately. The maintained roots pass focused MyDOS, paging,
selection, location and storage checks in both cartridge backends. Memory is
within the planned budgets; measurements are in the
[sample notes](../../samples/tn/README.md) and can be reproduced with
`measure-directory.py`. Interactive disk-level emulator acceptance remains
outstanding, as permitted in slice 6. The unrelated archived compatibility
stability check currently rejects its existing `RETURN(-1)`; see the backlog.
The [data contract](directory-model-contract.md) records the final boundaries.

## Milestone and scope

Replace TN's directory-as-screen-memory architecture with a directory reader,
a bounded batch cache, independent selection state and a renderer. The first
production source remains MyDOS. Its existing capacity of **64 file entries is
read, sorted and processed as one batch**, with 16 visible rows. FujiNet is the
north star: the shared code must also work when a directory spans many batches
and its full names cannot remain in Atari RAM.

Complete this milestone when TN and TNDBG retain their MyDOS behavior using the
new implementation, and a simulated source exercises the same paging and
selection code with hundreds and thousands of entries. Live FujiNet browsing,
host/device configuration, mount commands and new file operations are subsequent
work. The simulated source is a test fixture, not a new TN menu option.

This plan supersedes directory-row overlays and preservation of the packed path
layout as architectural goals in the
[source modernization audit](source-modernization-audit-2026-09-15.md).
Keep the completed constants/ORG, CASE and PanelState work. Internal layouts may
change; existing valid MyDOS operations, screen output and command behavior are
the compatibility contract. Preserve the archived original sources untouched.

## Starting point

The maintained [TN.ACT](../../samples/tn/modern/TN.ACT) and
[TNDBG.ACT](../../samples/tn/modern/TNDBG.ACT) currently combine several jobs:

- SetWin selects panel buffers, changes MyDOS state, reads directory text,
  sorts it, converts it to screen codes, resets tags and redraws the panel.
- The `v` table contains addresses of 18-byte rows. The first byte stores the
  tag mask; marker/name offsets are interpreted throughout the program.
- Convert reconstructs an operation filename from converted screen bytes.
- The final row is free-space text, accessed as `v(active.files)`.
- Tag/TagAll maintain row bytes and separate counters, while Xloop can clear
  row bytes directly. Copy stores BYTE indexes and assumes every entry remains
  addressable through `v` throughout the operation.
- Paths depend on parallel pointers into packed buffers and the current MyDOS
  directory sector. Range, selection, search and several counters use BYTE.

The [existing VM checks](../../tools/vm-runtime-tests/tests/tn_dispatch.rs)
cover dispatch and panel transitions, including the
[panel fixture](../../tools/vm-runtime-tests/tests/support/tn_panels.rs).
They do not yet characterize a complete 64-file directory, rendering, full
copy continuation or error recovery. Expand those checks before replacing
their source paths; compiling alone is not behavioral acceptance.

## Architecture and ownership

| Component | Owns | Must not depend on |
| --- | --- | --- |
| Directory source | Enumeration, source ordering, exact names, native metadata, source positions | Screen bytes or tag masks |
| Batch cache | Up to 64 compact entries, batch start/count, validity and end information | Total directory fitting in RAM |
| Selection | Tags for the current listing, tagged count or count-known state, tag-all intent | Cache addresses or visible row numbers |
| Panel | Location, listing generation, absolute selection, viewport and cache/selection storage | Another panel's active OS state |
| Renderer | Existing rows, markers, separators, highlighting and summary text | Constructing filenames for I/O |
| Command code | Iterating selected items and performing existing operations | Directory storage offsets |

Use one shared implementation for the two maintained roots. Start with narrowly
scoped shared Action! include files under `samples/tn/modern/`, tentatively
`DIR.ACT` for the model/selection and `MYDOS.ACT` for the source. Rendering can
remain beside the existing display code until its dependencies are small enough
to share. Do not combine this work with a repository-wide module conversion.
Expose explicit routine boundaries; source dispatch can use CASE when a second
production source exists. Procedure-pointer registries are unnecessary.

Records describe panel, batch and entry data. Enums describe finite states.
Use a variant for source-specific location alternatives when adding a second
production source; keep MyDOS sector/path information out of generic panel
logic now. Use unions for actual overlapping representations where useful,
not as an overlay that makes screen rows authoritative again.

### Batches and names

- `BATCH_CAPACITY=64`; `VISIBLE_FILES=16`. A batch is a processing unit, not a
  requirement that a single CIO/SIO transaction return 64 entries.
- MyDOS reads its complete listing before sorting and publishing it. The
  summary is separate from the 64 file slots. Input uses a dedicated 19-byte
  scratch buffer; successive records no longer overlap cache entries.
- A remote batch may be replaced while tags and the other panel survive. Keep
  a global directory ordinal with each cached entry. Cache slot, display row
  and source cursor are distinct values, even if some sources make them equal.
- A remote window need not start at a multiple of 64. Choose its start so the
  current 16-row viewport fits in the cache, including views crossing ordinal
  63/64. Keep MyDOS's complete batch at start zero. Test forward/backward
  scrolling and partial final windows without retaining a second batch.
- Store native text and semantic attributes. The MyDOS cache retains enough
  information to produce the exact existing `D:` filename without decoding a
  rendered row. A remote cache may retain a short label and a truncation flag;
  operations must resolve the complete name into caller-owned scratch storage.
- Never pass a clipped display name to open, copy, rename or delete. A resolved
  name remains valid only for its documented buffer lifetime. Copy must retain
  the current source name across a panel switch, or resolve it again from the
  captured source location before opening the destination.
- MyDOS sector counts retain their units. Missing remote size information is
  unknown, not zero. Preserve MyDOS's displayed sector text and summary while
  separating them from operation identity.
- Globally order the listing at the source boundary. MyDOS can sort all 64
  entries locally. A paged source must supply a consistent global order;
  independently sorting each batch is incorrect. Do not build an all-directory
  address/name index merely to keep the old Sort implementation.
  Assign ordinals in that established order; a private storage-slot order does
  not redefine entry identity.

### Selection, limits and listing identity

Use a bitmap indexed by the ordinal within an established listing. It is
independent of both cached entries and their screen representation.

| Selection capacity | Bitmap bytes per panel |
| ---: | ---: |
| 64 | 8 |
| 1,024 | 128 |
| 4,096 | 512 |

The MyDOS build uses eight bytes per panel. The first paged configuration has a
proposed **4,096-entry selection capacity**, independently of the 64-entry
cache. Storage is caller-owned; do not reserve two 512-byte bitmaps in the
MyDOS-only program. This is a bounded selection design, not unlimited tagging.

- Use CARD for absolute ordinals, selection positions and tagged counts. BYTE
  is sufficient for cache slots/counts (0..64), screen rows and copy chunk
  counts. Audit intermediate arithmetic and function parameters as well as
  declarations. Do not carry the `$FF+1` search-start trick into the new API.
- Provide operations equivalent to ClearTags, SetTagged, ToggleTag,
  ToggleAllTags, IsTagged and NextTagged. NextTagged reports found/end
  explicitly and never wraps an exhausted index back to zero.
- All tag changes, including successful command completion, go through that
  interface. The renderer derives the old `$7F` marker from selection state.
  No command writes cache or screen bytes to change a tag.
- Characterize and preserve the current ToggleAll behavior after mixed manual
  tagging; its remembered all/none intent is not necessarily the same as
  testing whether any tag is set. That intent belongs to Selection.
- For a listing whose length is not known yet, tag-all records the default
  selected state and initializes the bounded bitmap accordingly. Bits for
  undiscovered entries do not prove that those files exist. Iteration resolves
  entries until end; a precise selected count remains unknown until enumeration
  completes. Manual exceptions survive fetching or refetching a batch.
- Once end is known, discard bits beyond the actual entry count. Tag iteration
  never opens an entry merely because an unused bitmap bit is set.
- Test the 4,096-entry limit and the first excess entry. Browsing and operating
  on a single resolved item may continue beyond the selection capacity, within
  the source's address range. Reject tagging an out-of-range ordinal explicitly;
  already selected in-range entries remain usable. For tag-all on an unknown
  length listing, establish end within capacity before any file-operation side
  effects, scanning without retaining all names. If the listing exceeds the
  capacity, report the limit instead of executing a partial tag-all. An unbounded
  selection manifest or spill file is a separate feature.

Each panel owns a listing generation. Changing location, explicitly refreshing,
or changing the enumeration's filter/order starts a new generation and clears
selection. Cache replacement within a generation preserves selection. A local
generation number is bookkeeping, not evidence that the remote listing has
remained unchanged.

For MyDOS, retain all 64 canonical entries through command completion and
resolve operations from those entries. Deletion then cannot retarget the next
tag merely by changing the disk directory's positions. Keep existing disk-swap
and refresh behavior.

For a future remote source, positional tags require a verified repeatable
enumeration. A seek/tell cursor alone is not a stable file ID. Closing/reopening
a listing, switching a shared device cursor between panels, reconnecting,
insertion, deletion and renaming all need explicit treatment. Before enabling
remote batch mutations, prove snapshot/identity behavior or design an exact
name manifest, potentially on disk. Hash-only identity and presumed safety
from processing in reverse order are insufficient. A detected listing change
invalidates selection; do not automatically retry an operation on a new ordinal.

### Reader and operation boundaries

Use small status results and caller-owned output buffers. These are conceptual
contracts; slice 1 confirms their Action! declarations against both backends.

| Boundary | Contract |
| --- | --- |
| OpenListing(location) | Establish the listing/order and reset its enumeration state; no screen updates |
| ReadBatch(listing, start, output) | Fill at most 64 entries and report count/end/error; bounds and partial results are explicit |
| ResolveName(listing, entry, output, capacity) | Return an exact operation name or an error; never return success with truncated text |
| CloseListing(listing) | Release owned source resources without discarding an earlier primary error |
| EnterDirectory / ParentDirectory | Update the source-specific location through the existing navigation rules |
| NextTagged(selection, start, output) | Find the next selected ordinal without requiring it to be cached |
| RenderRow(entry, tagged, output) | Produce the existing 18-byte MyDOS row; no I/O or selection mutation |

Batch replacement must mark the cache invalid before writing it and publish it
only after a successful fill. Avoid a second 64-entry staging buffer solely for
transactional publication. An error may leave the panel needing a reload, but
must not expose half-old/half-new entries to commands. Distinguish an empty
listing, ordinary end, capacity exceeded and an I/O failure.
Invalidation must also survive a CIO error's nonlocal jump through NavError;
cleanup cannot rely solely on the reader returning normally.

Resolve inputs and outputs without a hidden generic current drive. Initially
the MyDOS boundary synchronizes the existing OS drive/current-directory cells
at the same call points as TN. Retain its CIO bridge and nonlocal error recovery
until the new state is explicitly safe under those transfers. This plan does
not replace NavError's stack reset with an unrelated exception/result framework.

### Memory and compiler constraints

Target at most 24 bytes per compact cache entry: 1,536 bytes per 64-entry cache.
Use a local BYTE order table if necessary for MyDOS sorting, rather than moving
large records or keeping a second directory. The exact record layout is frozen
after the source-shape probes and MyDOS characterization in slice 1.

Provisional MyDOS data budget per panel:

| Storage | Budget in bytes |
| --- | ---: |
| 64 compact entries | 1,536 |
| Optional local sort order | 64 |
| Selection bitmap | 8 |
| Summary | 18 |
| Existing-depth MyDOS path storage | 47 |
| Panel/listing/cache/selection control fields | 48 |
| Total ceiling for these components | 1,721 |

Budget at most 512 additional shared scratch bytes for row/input/name handling
and source adapter temporaries, including hidden compiler storage when measuring.
These are design ceilings to verify, not measured outcomes. Remote path and
cursor storage needs a separate budget before adding its production adapter.
The existing comparable per-panel allocation is 1,357 bytes: directory 1,171,
address table 130, path 47, saved state 8 and tag counter 1.

Measure load-file growth, emitted/static/deferred workspace, and the resulting
copy buffer at `SET BUFFER=*` in all four TN/TNDBG cartridge builds. A smaller
source file or load file is not proof of more usable copy RAM. Do not preserve
both the old full directory and the new full cache after integration.

Preserve ORG `$2C00`, screen/allocp zero-page homes, assembly contracts and the
copy-buffer boundary. The [classic record-copy placement issue](../../docs/BACKLOG.md#classic-record-copy-scratch-placement-with-set)
remains backlogged. Avoid whole-cache copies and aggregate value parameters;
probe small record/variant operations in the actual SET context. Use field
updates or existing size-based copies where needed. Do not modify the compiler
or inflate ORG to accommodate hidden storage as part of this plan.
Remember that MovePage's BYTE count of zero means 256 bytes. A larger bitmap
must be initialized in bounded chunks or a CARD-count loop; passing 512 through
that ABI would not clear the requested storage.

Keep the existing path depth. Characterize the inclusive dot-copy loop for
eight-character directory names before replacing packed path storage. If it
overwrites the next slot, add a focused regression and fix it in a separately
identified bug-fix commit; memory corruption is not a compatibility requirement.

## Implementation slices

Each slice should leave both maintained programs buildable and have a separate
reviewable commit. Do not include unrelated workspace changes. Commit/push
actions follow the user's instructions for the implementation turn.

### 1. Characterize MyDOS and freeze the data contract

Extend the existing VM harness using real TN routines with only external input,
screen services or disk operations substituted as appropriate. Record baseline
directory bytes, displayed rows, exact operation names, ordering, tag-all
transitions and I/O traces. Capture 0, 1, 63 and 64 files plus the summary;
mixed files/directories/protection, extensionless files, maximum names and both
observed sector-text/EOL alignments. Add capacity guards and malformed/error
fixtures as new defined cases, without treating an old overrun as an oracle.

Capture batch copy continuation, partial final chunks, disk-swap prompts and
tag clearing, and both MyDOS directory-sector locations. Characterize the path
boundary and document any isolated fix needed. Retain normal/debug differences.

Compile minimal record/index/bitmap/result probes with TN's allocation controls
in both advertised backends. Freeze the record fields, result conventions,
memory budget and dependency order for the shared files. No compiler fix is
part of this slice.

Acceptance: baseline behavior is independently recorded; 64-file capacity and
full-program memory measurements are reproducible; planned source forms build.

Suggested commit: `test(tn): characterize complete MyDOS directory behavior`.

### 2. Implement batch and selection primitives

Add shared types and routines for bounded batches, listing generations, tag
bitmaps and tagged iteration. Make capacity/storage explicit. Exercise them
through a small Action! fixture with a deterministic simulated directory source;
generate its entries on demand rather than allocating the whole source in the
Atari VM. Keep the production roots on their existing reader for this slice.

Test 64-entry batch replacement, tag preservation across eviction, two panels,
unknown count/tag-all exceptions, empty/end/error and invalidated generations.
Use ordinals around 7/8, 63/64, 255/256, 1,023/1,024 and 4,095/4,096. Verify
capacity guards, count-known state and no wrapping at search exhaustion. Test
selection patterns and operation sequences against a host model rather than
mirroring bit-manipulation instructions.

Acceptance: one shared selection implementation works with 8- and 512-byte
bitmaps and a fixed 64-entry cache; LF/CRLF fixture paths are covered.

Suggested commit: `feat(tn): add bounded directory batches and selection`.

### 3. Implement and verify the MyDOS reader and renderer

Build the adapter beside the current production path first, in an isolated
fixture. Parse each 19-byte CIO input record into native directory fields.
Separate and preserve the summary. Fill the complete batch, sort it in the
established MyDOS order, then publish it. Handle excess/malformed input and
failed reads without out-of-bounds stores or a falsely valid directory.

Implement exact filename resolution and row rendering from the new records.
Tags are supplied by Selection, never parsed from a cached row. Compare the
rendered bytes, order and operation names with slice 1's baseline fixtures.
Ensure name rendering cannot modify the canonical operation name.

Acceptance: the reader consumes 64 files plus summary in one batch, reproduces
the existing valid-input outputs, and has bounded scratch/storage on both
backends. The free-space summary is not a taggable entry.

Suggested commit: `feat(tn): parse MyDOS directories into the batch model`.

### 4. Integrate both roots and migrate command consumers

Switch SetWin/Dir to the MyDOS reader; Draw/UpdDis use the renderer. Move
IsTagged, IsProtected, IsDirectory, Tag/TagAll and FindNext onto the model.
Replace Convert-from-screen with exact-name resolution in View, Attrib, Delete,
Rename, Handle and Copy. Remove direct tag writes in Xloop and migrate all
tag-count consumers together. Keep wildcard operations and accepted command
gates exactly as characterized.

Make file ordinals CARD throughout navigation, tag searches, Copy's selected
file references and TNDBG snapshots. Keep chunk counts and viewport rows BYTE.
Introduce TN-specific navigation that supports wider indexes while preserving
Range's boundary-key behavior; do not widen the unrelated LIB menu helper.
Carry source identity/name state safely across Copy's panel switches and partial
file transfers. Preserve the existing transfer algorithm and error behavior.

Delete the old persistent screen-row directories and `v` address tables once
all consumers migrate. Adapt TNDBG's raw-entry diagnostics to the new fields
and rendered-row projection, retaining useful counters, G command and error
diagnostics. Report internal diagnostic layout changes explicitly.

Acceptance: both full programs pass the MyDOS behavioral comparisons, command
I/O traces, stack checks and memory guards. Shared model/reader logic has one
implementation; there is one authoritative tag store and no operation filename
is decoded from screen memory.

Suggested commit: `refactor(tn): use directory batches for panels and commands`.

### 5. Isolate locations and exercise paging through the integrated code

Move MyDOS drive/sector/path ownership behind its location routines. Replace
packed pointer arithmetic with explicit path storage and enter/parent actions,
retaining the existing depth and valid-name behavior. Keep panel switching
separate from refresh, and capture/restore MyDOS OS state in a defined order.
Do not copy a complete cache to activate a panel.

Connect the simulated source to the same integrated browsing and selection
routines through a test fixture or compiled routine hooks. Cross batch
boundaries, return to evicted batches, switch panels and iterate tagged items
outside the cache. Resolve full names with identical clipped prefixes to prove
that operations select the intended item. Include a source that loses its
cursor when another panel opens a listing and one that changes its generation;
the expected behavior must not silently reuse stale ordinal tags.

For known-length fixtures, test first/last navigation. For unknown length, the
existing last-item command can scan bounded batches to end without retaining
them all; test errors and restoration of the displayed selection. Do not add
new paging keys or live source selection UI in this milestone.

Acceptance: the production MyDOS path still loads its complete directory in
one batch; shared code handles a many-batch source with fixed cache memory,
correct full-name resolution and explicit listing invalidation.

Suggested commit: `refactor(tn): isolate locations and support batch navigation`.

### 6. Acceptance, storage checks and documentation

Update the TN-specific storage-analysis expectations to match the new routine
and storage boundaries. Replace hard-coded old directory-array sizes in the
deferred-storage test while retaining code/data/scratch non-overlap assertions
and the copy-buffer boundary. Document measured sizes, peak workspace, remaining
copy RAM, source contracts and selection limits in the sample notes.

Run a MyDOS emulator smoke check for both roots: a full 64-file directory,
subdirectories, tagging, copy, rename/delete and panel switching. Use disposable
test disks for mutating operations. If no interactive emulator is available,
record that check as outstanding; VM service stubs are not a disk-level result.

Acceptance: the checks below pass, memory budgets are met or a specific revised
budget is documented with its copy-RAM impact, and no obsolete directory/tag
storage paths remain. Update this plan's status and the audit's follow-up links.

Suggested commit: `test(tn): validate directory batches and document memory use`.

## Validation scope

Run affected tests for each slice; reuse each compiled full program across
related scenarios rather than recompiling it per case. Cover TN and TNDBG in
modern classic (`--mode optimized`) and MIR6502, with their advertised cartridge
runtime. Additional fixture-only configurations do not imply standalone TN
support. Suggested additional VM target: `tn_directory`, sharing harness code
with `tn_dispatch` rather than duplicating source instrumentation.

Existing focused checks, with new test targets added as they are introduced:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_dispatch
cargo test --locked --test nir_storage_analysis tn_exposes_high_value_scalar_promotion_candidates
cargo test --locked --lib tn_deferred_storage_starts_after_final_mir_bytes
```

Retain exact binary/ATASCII fixtures. Normalize only host text; exercise both
LF and CRLF through any new source/listing parsing path. Use symbol-derived
addresses for VM observation rather than fixed compiler output addresses.
Test one large directory plus focused boundary cases, not a Cartesian product
of every directory size, error and panel permutation. Full compiler/NIR sweeps
are unnecessary for this sample/test work unless a compiler change is separately
undertaken under the repository's required checks.

## FujiNet follow-up requirements

The next production adapter is separate from the current
[ATARI.FUJINET.NET](../../docs/ATARI_FUJINET_NET.md) HTTP/TCP stream API, which
does not implement directory operations. The
[SIO design](../../docs/ATARI_SIO_LIBRARY_DESIGN.md) places host slots, directory
iteration and mounting in subsequent CONTROL work. Confirm whether TN's first
FujiNet source browses those host directories or a network-protocol directory;
these are different APIs and must not be conflated.

The official host-directory protocol documents
[open/filter/sort options](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/SIO-Command-%24F7-Open-Directory),
[bounded entry reads](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/SIO-Command-%24F6-Read-Directory),
and 16-bit [get position](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/SIO-Command-%24E5-Get-Directory-Position)
and [set position](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/SIO-Command-%24E4-Set-Directory-Position).
These are useful paging primitives; the documentation alone is not a snapshot
or stable-identity guarantee. Before implementing the adapter, pin a firmware
revision and verify cursor semantics, name limits/truncation, resource ownership
between two panels, sort behavior, reconnect/reopen behavior and mutations.

Use protocol fixtures and a real FujiNet-capable emulator or hardware acceptance
run. A synthetic paged source proves the TN cache/selection contract, not the
firmware's directory consistency or network behavior. Do not enable remote
batch mutations until their identity contract is established.
