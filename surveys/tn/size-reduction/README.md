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
