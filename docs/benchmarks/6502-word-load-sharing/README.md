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

Slice 2 opt-in selection passes nine proof/transaction unit tests and the
frozen-image integration matrix. It also passes the existing pointer-sharing,
argument-X and callee-entry-flag matrices. Both configuration defaults remain
disabled pending VM/corpus qualification.

The general local-source rule finds an additional six-byte copy in `Free`'s
traversal loop. Four sites therefore save 24 bytes: `Alloc` 287→281, `Free`
358→346, `AllocInit` 96→90, and complete cartridge/standalone images
952→928 / 1,427→1,403. The neutral images shrink 313→301 at both origins and
in both runtimes. Frames, globals, reservations, block order and terminators
are unchanged. The accepted batch uses seven additional emissions.

Slice 3 qualification passes 13 proof/transaction tests, including all 65,536
word values under four incoming status patterns, and six VM tests. The VM
checks 4,704 exact emitted fragments across storage classes, raw/optimized NIR,
both runtimes/origins, and actual LF/CRLF files. Partial aliases use an
independent sequential-byte oracle; volatile/absolute reads retain fallback.
Later indirect reads may alias the completed copies. Real allocator scenarios
retain the existing two-sided-coalescing behavior. Existing pointer, home-value,
word-store, argument-X, entry-flag and checked-pointer VM targets also pass.

The cycle probe now stops after the wrapper's final JMP. Earlier analysis
executed a seven-cycle BRK at the destination; subtracting only the 20-cycle
wrapper overstated absolute times by seven. Savings are unchanged. Corrected
cartridge routine measurements (return included, wrapper excluded) are:

| Case | Disabled | Enabled | Saved cycles |
| --- | ---: | ---: | ---: |
| Empty Alloc | 95 | 87 | 8 |
| Exact Alloc | 241 | 233 | 8 |
| Split Alloc | 318 | 310 | 8 |
| Boundary/skip Alloc | 407 | 399 | 8 |
| Insert Free | 407 | 391 | 16 |
| Left-merge Free | 345 | 329 | 16 |
| Right-merge Free | 474 | 466 | 8 |
| Two-neighbor Free | 521 | 505 | 16 |
| AllocInit | 163 | 155 | 8 |

The extra eight-cycle saving in traversing Free cases comes from the fourth
match in its loop body, and repeats for each visited node.
