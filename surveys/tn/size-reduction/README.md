# TN size reduction qualification

The [implementation plan](../size-reduction-implementation-plan.md) defines
the stages. [baseline.json](baseline.json) pins the Stage 0 four-build memory
measurements and CPU workloads, reproduced on 2026-10-12.

Memory includes code, emitted homes and deferred arrays. Copy RAM uses `$A000`
as a comparison ceiling, not a replacement for OS MEMTOP. CPU measurements
execute the maintained TN root with substituted screen/disk services. They
include the fixed host call wrapper and service-return instructions; they do
not measure device latency, real CIO work, ANTIC DMA or disk-level behavior.

The profile checks reversed-order sorting and exact visible rows against the
independent MyDOS oracle, tag counts for 0/1/64 entries, and a guarded 512-byte
bitmap when publishing a 1,088-entry prefix. There are no exact cycle assertions.
Reproduce it with:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test tn_size_profile -- --nocapture
python3 surveys/tn/measure-directory.py --compiler target/debug/actionc \
  --out-dir target/tn-size-reduction/current
```

## Preserved provider acceptance

| Contract | Independent checks |
| --- | --- |
| Publication, bounds, failure invalidation | `tn_directory_model`, `tn_mydos_directory` |
| Cache eviction, wide ordinals, unknown end, capacity errors | `tn_directory_model`, `tn_dispatch` / `support/tn_paging.rs` |
| Listing identity loss and two-panel cursor restoration | `tn_dispatch` / `support/tn_paging.rs` |
| Exact long names, identical previews, short-output no-write | `tn_dispatch` / `support/tn_paging.rs` |
| MyDOS rows, sorting, names, panels, copy continuation | `tn_dispatch`, `tn_mydos_directory` |
| Deferred storage, copy boundary, promotion consumers | Root `tn_deferred_storage_starts_after_final_mir_bytes`, `nir_storage_analysis` |

Keep `DirEntry.position`: it is provider metadata, distinct from a logical
ordinal and physical storage slot. No live FujiNet firmware contract has been
qualified by this work. The existing simulated provider exercises cursor loss
and stable versus changed listings; it does not establish device behavior.

## Classic correctness evidence

The [minimal indexed-mask reproducer](../reproducers/indexed-mask.act) retains
the pre-fix failure independently of TN. In classic optimized output, `Apply`
loads `masks[ordinal & 7]` but its later set operation substitutes `TXA` for
reloading the mask. X still contains the index. Setting ordinals 0 and 7 in a
zeroed bitmap therefore yields 7, instead of the expected 129. A descriptor
array or a byte-only index can hide the failure; the reproducer deliberately
uses an inline array and a CARD ordinal.

Stage 2 activates this case as a passing execution regression with its fix.
Stage 0 does not add a known failing test to normal CI or disable TN coverage.
The original experiment remains under ignored target storage, but this source
and the pinned expected behavior survive disk cleanup.

## Stage 1: shared entry addressing

[stage1.json](stage1.json) records the accepted helper slice. Both MIR roots
save 250 load bytes and gain 250 bytes of copy RAM. Classic TN/TNDBG save
311/315 bytes. The reversed 64-entry MIR sort rises from 855,767 to 877,794
cycles (2.6%); the 16-row draw rises from 58,664 to 58,952 (0.5%). This is a
code-space tradeoff, not a speed improvement. Reassess helper expansion after
the compiler slices.

All three behavioral targets, the CPU audit and both storage checks passed.
All four full-root LF/CRLF XEX images match byte for byte. The batch/entry
layout, checked public lookup and provider paging contracts are unchanged.

## Stage 2: classic indexed capture

The inline-array scalar-index fast path now uses the tracked indexed-load
wrapper. That wrapper obtains its fact through the existing conservative slot
query: an indexed addressing template does not identify a stable byte. A table
load can no longer leave the previous index recorded as A's value.

`classic_indexed_mask` fails before the fix on ordinal zero and passes after
it. It checks all eight bit positions and wide ordinals through 4,095, four
initial bitmap patterns, both set/clear branches, complete bitmap/guard bytes,
unchanged table data, cartridge/standalone and actual LF/CRLF compilation.
The three TN behavioral targets and 836 classic codegen checks passed. The
final slot-query version also passed the five indexed-table arithmetic checks
and the full mask execution matrix. No application workaround was removed.

## Stage 3: typed address proof foundation

MIR address uses explicitly distinguish captured pointer values from mutable
pointer-cell reads. The byte-window proof covers complete accesses, all 65,536
constant displacements for both widths, dynamic-index bounds and 16-bit wrap.
The existing rewrite context now proves exclusive word flow using both lanes,
reaching definitions, dominance and all routine users. Later high-byte uses,
redefinitions, incomplete words and successor uses prevent removal.

The three address-analysis tests and four rewrite-context tests passed. No
selector uses the new queries in this slice, so target strategy is unchanged.
There is no NIR, SemIR or public ABI contract change.

## Stage 4A: complete constant field windows

Pointer field consumers now select a complete byte/word window. Fields beyond
Y's range, and words beginning at displacement 255, rebase the scratch pointer
before accessing the field. A fixed source pointer cell is never advanced in
place. Small fields retain their existing direct displacement strategy.

The execution matrix covers offsets 0/254/255/256, page crossings, 16-bit wrap,
RAM and fixed zero-page pointer cells, both runtimes, default/optimized MIR and
actual LF/CRLF source. The four affected integration targets also passed. This
slice establishes correct consumer selection at the boundary; it does not
claim smaller TN output before the address-chain slices are integrated.

## Stage 4B: captured field displacement and byte indexes

Exclusive word copies and constant additions now fold into typed address
consumers. The original base load remains captured; aliases, redefinitions,
multiple users and live arithmetic flags block producer removal. Embedded
byte fields rebase their constant displacement and use Y for the unrestricted
byte index. No array capacity is treated as an index bound.

[stage4b.json](stage4b.json) records an additional 87/110 load bytes saved in
TN/TNDBG MIR, with classic unchanged. The 64-entry MIR draw falls from 58,952
to 57,822 cycles; reversed sorting rises from 877,794 to 893,196. These are
measured code-space/carrier tradeoffs; neither slice is claimed as a universal
speed win. Three proof tests, two address execution matrices, the rewrite
inventory and all five selected VM targets passed. Effectful indexes execute
once; boundary indexes, page/address wrap and neighbor guards remain covered.

## Stage 4C: record consumers and construction cost

Exclusive captured multiply/add chains can feed a typed record-element load
or store directly. Strides 1–255 use the existing indexed materializer;
wider strides, escaping addresses, extra users, barriers and live flags retain
explicit arithmetic. No new multiplication expansion policy or ABI is added.
Constant field rebasing now uses one combined address construction, whose
constant emission adds the complete scaled displacement to the captured base.

[stage4c.json](stage4c.json) records another 189 bytes saved in each MIR root.
The 64-entry reversed MIR sort is 805,176 cycles and drawing is 55,120, compared
with baseline 855,767 and 58,664. Classic is unchanged. The generic
[consumer fixture](../reproducers/record-consumers.act) makes a remaining
tradeoff visible: `ReadByte` grows from 27 to 29 bytes; `ReadElement` stays at
104 because its address escapes into a source local. Stage 5 reviews parameter
carriers rather than promising that every removed virtual definition saves code.

Four address proof tests, both complete field-window execution matrices, the
materialization/inventory integration checks and the real record-boundary,
embedded-record and TN CPU VM targets passed. The new record VM matrix checks
strides 2/20/255/256, word indexes through 65,535, field offset 255, guards,
single effectful index evaluation, cartridge/standalone and actual LF/CRLF.

## Stage 5A: proven block-local base availability

The existing address-availability analysis now recognizes repeated plain
pointer preparation, including registers with exact captured values. It removes
preparation only while the same pointer pair retains that value and the removed
A/flag effects are dead. Final physical placement excludes source cells that
preparation would overwrite. Calls, pointer-cell changes, aliasing writes and
machine barriers invalidate availability. No cross-block reuse was introduced.

Two focused proof matrices, the existing SSA-lite cases, all three TN behavioral
targets and the CPU profile passed. TN's emitted images and shared-home counts
are unchanged from Stage 4C. This foundation has no demonstrated TN size win.
The profile now reports executed operand accesses to emitted spill storage,
including a checked read/write counter. It excludes canonical parameters,
locals, stack traffic, pointer-fetch bytes and resident services; it is not a
census of every private bus access.

The final Stage 4C CLI image matches Stage 5A byte for byte. A fresh profile of
that integrated image measures the 64-entry MIR draw at 54,586 cycles; the earlier
55,120 timing came from an intermediate Stage 4C build. The difference is not
attributed to base reuse. Sorting remains 805,176 cycles.

## Stage 5B: parameter carriers at field consumers

[stage5b.json](stage5b.json) records the accepted consumer slice. Canonical
parameter captures can feed plain or constant-index pointer preparation from
still-available registers. A block-local capture and final physical write ranges
prove each substituted byte; calls, opaque operations, overlapping homes and a
high byte carried in A block selection. Public parameter/result stores remain.
Existing captured-value, call-plan and spill forwarding continue to own other
argument/result flow; no parallel alias-sensitive forwarding pass was added.

The generic embedded-byte reader shrinks from Stage 4C's 29 bytes to 24,
compared with the original 27. Its escaping record-element reader remains 104.
TN/TNDBG size, shared homes and executed spill accesses are unchanged. This
bounded rule has a demonstrated generic consumer, but no additional TN saving.
The focused positive/negative matrix and aggregate call execution checks pass;
mutable arguments and indirect callee snapshots remain independent. The final
MIR draw/sort timings remain 54,586/805,176 cycles for 64 entries.

## Stage 6A: checked tag mutation ownership

[stage6a.json](stage6a.json) records the shared checked mutation path. Toggle
computes its pointer, mask and old bit once. Public `SetTagged` normalizes every
nonzero input to set, so internal toggle mode never changes its contract.
Both entry points reject capacity/extent violations before bitmap mutation.

The slice costs 36 MIR bytes and five shared-home bytes per root (classic
22/28 bytes). The 64-entry MIR toggle falls from 1,074 to 755 cycles and emitted
spill operand accesses from 14 to 4; classic falls from 1,213 to 822 cycles.
The full behavioral and profile targets pass, including added nonzero public
values, repeated toggles and guarded invalid-ordinal no-write cases under
actual LF/CRLF compilation. The extra code buys measured command work savings.

## Stage 6B: bit reads within a clamped iteration limit

[stage6b.json](stage6b.json) records the internal `TagBitInRange` consumer.
`TagsRecount` and `NextTagged` establish `ordinal < min(extent, capacity)` and
perform no capacity mutation, provider call or panel switch in the loop. Public
`TagBit` retains its capacity check. Selection counting and bitmap layout are
unchanged; provider publication and navigation checks remain checked.

This slice adds 32 bytes and four shared-home bytes to each root. The 64-entry
MIR tag-all workload falls from 29,558 to 26,294 cycles; publishing a 1,088-entry
prefix falls from 410,697 to 355,209. The MIR draw rises from 54,586 to 55,178
because it uses the checked wrapper. All three behavioral targets and the CPU
profile pass; additional checked-read boundaries and extent-only invalid-toggle
no-write checks pass through LF/CRLF model compilation. Cache access in `Draw`
and post-I/O `MyDosPush` remain unchanged: this work does not establish a stable
provider lifetime proof that would justify their trusted alternatives.
