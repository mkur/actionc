# MIR6502 word load sharing measurements

The frozen baseline is compiler revision `7f7778f6`, with source, dependency,
compiler executable and image hashes in [baseline.json](baseline.json).
The [implementation plan](../../MIR6502_WORD_LOAD_SHARING_IMPLEMENTATION_PLAN.md)
specifies four slices and an 18-byte ALLOCATE target.

The retaining client is
[`allocate_word_load_sharing.act`](../../../fixtures/runtime/allocate_word_load_sharing.act).
It keeps all four routines of the maintained library reachable. VM probes
initialize a valid sentinel and heap before entering each routine directly;
the inspection client's entry is not the execution oracle.

| Cartridge baseline at `$3000` | Bytes |
| --- | ---: |
| `Alloc` | 287 |
| `Free` | 358 |
| `AllocInit` | 96 |
| `PrintFreeList` | 112 |
| Complete retaining client XEX | 952 |

The neutral fixture is 313 bytes in both runtimes at `$3000` and `$41C7`.
Baseline images, listings, materialized MIR and maps are retained under ignored
`build/word-load-sharing/before/`. The future feature-off compiler must
reproduce these images; the committed manifest preserves their identities.

The original hand schedule removes two ordinary absolute loads at each of
three sites, predicting 934 XEX bytes and eight cycles saved per affected call.
Final compiler selection must establish the actual saving with physical replay,
unchanged machine/memory behavior and the same capacity/reservation facts.
The separate comparison-block layout and the source library's coalescing
behavior remain outside this optimization.

Slice 1 validation: five typed-proof unit tests passed. The neutral and
retaining clients reproduce all six frozen XEX files byte-for-byte; their
frame, storage and reserved-range facts are also hashed in the manifest.
The existing pointer-load-sharing integration matrix remains green.
