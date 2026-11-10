# TN / Oscar64 C reference qualification

The [implementation plan](../../../docs/TN_C_OSCAR64_TRANSLATION_PLAN.md) defines
all eight slices. Slices 0–1 are complete. The full port is not yet implemented.

[baseline.json](baseline.json) freezes entry/include hashes, Action!/Oscar64
compiler revisions and binary hashes, ROM/MyDOS inputs and four Action! images.
[inventory.json](inventory.json) lists every live routine in the three source
files; implementation and qualification fields advance with each slice.

Slice 0 executes actual Oscar64 `-Os` and `-O3` Atari startup, initialized/BSS
state, counted bytes, all packed-record offsets, little-endian words and a
non-inlined runtime-input scalar call. Six VM executions check byte/word boundary
arguments and preservation of TN's fixed zero-page cells. Actual CRLF C/header
copies compile byte-identically to LF inputs in both profiles. The generated
probe manifest and logs live in `build/tn-c-oscar64/`.

The loader adapter accounts explicitly for Oscar64 RUNAD `$02E0` versus the
pinned Action! VM helper's `$02E2` INITAD convention. Source routines and opaque
machine routines remain unmodified. No inference about full-program stack or
zero-page safety is made from this startup probe.

Slice 1 adds the platform bridge and ten executions across two profiles and
copy lengths 0/1/127/128/255. They check 80 nested error recoveries, all 256
values for each conversion, page-crossing copies including the original
zero-length/256-byte behavior, source/destination guards, arena words, relative
position carry, screen address calculation, counted filename conversion, CIO
commands/lengths, EOF return, keyboard stacked return and terminal transfer.
CIO/keyboard vectors are mocked; real Atari integration remains required.

The pinned compiler folds setjmp's ordinary return to zero. Recovery therefore
uses an explicit volatile pending flag after the library restores CPU/software
stack and saved registers. Indirect terminal transfers use an actual global
vector; an inline-assembly parameter address is not assumed to be its value.
The library's `_LodChn` operation is folded into typed IOCB offset calculations,
without retaining a no-op implementation. No compiler or VM changes were made.
