# TN / Oscar64 C reference qualification

The [implementation plan](../../../docs/TN_C_OSCAR64_TRANSLATION_PLAN.md) defines
all eight slices. Slice 0 is complete. The full port is not yet implemented.

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
