# Checked arena controls

The implementation starts from `dedd35ab`. `unchecked.json` freezes TN and
TNDBG in classic optimized/MIR6502 and cartridge/standalone configurations.
It records source and compiler hashes, image sizes/hashes, routine ranges and
listing placement records. Full images/listings are regenerated in
`build/checked-arena/`; the source is read from the pinned Git revision.

```sh
python3 tools/compare6502/tn/checked_arena.py
python3 tools/compare6502/tn/checked_arena.py --check
cargo test --test tn_checked_arena_controls
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_checked_arena
```

Use the baseline compiler binary when reproducing these historical images.
Future checked-library measurements must have separate bindings.

The resolved storage audit finds `allocp` at `$E8/$E9` and direct cursor stores
in `Alloc`, `Free`, `Push`, `Pull`, `RestorePos` and `Init`. Pointer writes and
machine effects can still alias it; the analysis must invalidate cached facts
at unproved effects. The runtime validates each captured cursor independently.

The three calls to `Strcat` have different reaching inputs:

| Call | Destination | Source | Remaining obligations |
| --- | --- | --- | --- |
| `MkDir` | `s = Alloc(20)`, then `Free(s)` | mutable `linebuf` | Preserve the numerical return range across rewind and subsequent calls; prove the loaded global was initialized and not changed |
| `Rename`, append comma | mutable `linebuf` | literal comma | Prove reaching initialization of the destination across calls; classify the literal source |
| `Rename`, append new name | mutable `linebuf` | mutable `s`, pointing at input storage | Prove both reaching stores and intervening effects |

These facts at callers do not restrict all incoming entries of `Strcat`.
Its generic MIR6502 body is 113 bytes. The existing short-string parameter-home
alias probe remains valid even without length overflow. Private variants are
outside the first delivery.

`checked_pointer_ranges.act` supplies neutral checked/unchecked routines with
a non-page-aligned arena, a guarded concatenation and a checked-return case.
The runtime harness executes the real TN library for successful allocations,
released-byte scratch and a full window save/draw/close cycle. It recompiles
actual LF and CRLF files and compares their images. Fault cases exercise the checked API; unchecked overflow is not used as an
oracle.

The checked runtime keeps the 2 KB reservation. `ArenaCursor` validates one
captured cursor before capacity arithmetic; each primitive captures its request
or rewind mark separately. `ArenaStop` records a stable reason and cannot
return. `MovePage` now declares its complete possible write range, including
zero page, and does not claim unconditional preservation of indirect targets.

The runtime matrix executes 1,120 calls (464 returning and 656 terminal).
It includes 41 faults per build configuration, full/exact/zero
allocations, odd/current rewind marks, exact push/pop space, released scratch,
real window restoration and the short-string parameter-home alias example.
The nested NewDrive/Format/confirmation window geometries after `SwapScr`
reach `$2583` (1,411 bytes). TNDBG's largest debug window reaches `$274C`
(1,868 bytes). These observations cover the executed scenarios, not all MyDOS
interactions. High-water sampling observes complete primitive results at
returns, excluding transient half-written cursor words.

`Pull` and `RestorePos` retain their `=*` entries and use separate ordinary
snapshot cells. This avoids adding local frame data to classic current-location
entries. The VM test normally uses the linked compiler crate; local runs can
select a freshly built release CLI through `ACTIONC_ARENA_COMPILER`.

The inert unsigned-range slice passes its eight focused tests, unchanged NIR
snapshots, the 51-fixture NIR sweep and the full Rust suite in an isolated
checkout. The checkout contains only committed arena work and the range slice.
The original workspace's full suite reaches an unrelated sample-parser failure:
untracked `samples/vbxe/shared/lines.act` needs an unconfigured `SHARED.SCREEN`
module path. That workspace work was preserved. Shared checks use stripped
debug information and Rust optimization level 1 to keep the complete sample
materialization matrix practical; assertions remain enabled.
