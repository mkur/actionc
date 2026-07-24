# TN MIR6502 final-listing audit

Date: 2026-07-24.

Revision: `e7b0cab0a8400eacb916b435f83b38cb511ba446`
(`test: execute dual-pointer word transfers`).

Scope: `samples/tn/modern/TN.ACT`, modern profile, comparing the MIR6502
backend with the modern/classic backend.

Source SHA-256:
`097df477534d50b9aaec1d733b8d6a66f6792e00cd7703e46331c2d5425f8797`.

`Cargo.lock` SHA-256:
`02e7e9e564916b19fe9aad0dc7d2efd44fe9fc58cf56132022ebddbbc4a754fd`.

## Result

MIR6502 remains 10 load-file bytes larger than modern/classic: 10,455 bytes
versus 10,445 bytes. This is a real emitted-payload difference, not XEX
headers, padding, or an extra segment.

Both files have the same two-segment structure:

| Backend | Main segment | Main bytes | Run vector | `RUNAD` | XEX bytes |
| --- | --- | ---: | --- | --- | ---: |
| MIR6502 | `$2C00-$54CA` | 10,443 | `$02E2-$02E3` | `$5469` | 10,455 |
| Modern/classic | `$2C00-$54C0` | 10,433 | `$02E2-$02E3` | `$545F` | 10,445 |
| Difference | same origin | +10 | identical | +10 | +10 |

The final-listing rows reconcile the main segment exactly:

| Listing form | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| Instruction-form rows | 4,271 | 4,236 | +35 |
| Bytes in instruction-form rows | 9,565 | 9,408 | +157 |
| Bytes in explicit `.BYTE` rows | 878 | 1,025 | -147 |
| Main-segment payload | 10,443 | 10,433 | +10 |

This is a syntactic listing decomposition. Some `.BYTE` rows are embedded
machine or routine data, so it should not be read as a complete semantic
code/data classification. It is nevertheless byte-exact and explains the XEX
delta without estimates.

MIR6502 still performs materially more memory traffic:

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| `LDA` + `STA` instructions | 2,061 | 1,910 | +151 |
| `LDA` + `STA` bytes | 4,863 | 4,562 | +301 |
| Share of instruction-form rows | 48.3% | 45.1% | +3.2 points |
| Share of instruction-form bytes | 50.8% | 48.5% | +2.3 points |
| `JMP` | 162 | 158 | +4 |
| `JSR` | 367 | 368 | -1 |

The output is at parity because MIR6502's 147-byte `.BYTE` advantage currently
offsets almost all of its 157-byte instruction-form deficit. Future work should
therefore measure the whole XEX after every slice; a routine-only saving can be
hidden or amplified by storage and layout changes.

## Measurement correction

`actionc-listing-quality` currently reads only columns 6 through 13 as the raw
byte field and starts instruction text at column 16. That is sufficient for a
6502 instruction, which is at most three bytes, but not for listing rows with
four to eight `.BYTE` values.

Long `.BYTE` rows are consequently truncated and their remaining hex bytes are
interpreted as pseudo-mnemonics such as `0`, `2`, or `A`. The current tool
reports 4,337/4,338 instructions, 9,763/9,714 code bytes, and 395/285 data
bytes. Those top-line totals are not valid for the final size comparison.

This audit instead parses the complete variable-width byte field up to the two
spaces preceding the listing text. Its instruction-form and `.BYTE` totals sum
exactly to each main XEX segment. Real-mnemonic counts such as `LDA`, `STA`,
and `JMP` remain useful in the existing report, but percentages, total
instruction counts, data totals, and per-routine byte totals can be polluted by
long byte rows.

The measurement parser should be fixed before the next optimization audit and
covered by a row containing at least eight `.BYTE` values.

## Routine concentration

The following comparison counts instruction-form bytes within matched `PROC`
ranges and excludes explicit `.BYTE` rows. The largest positive MIR6502
differences are:

| Routine | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| `Handle` | 849 | 819 | +30 |
| `Window` | 324 | 301 | +23 |
| `PopUp` | 276 | 253 | +23 |
| `Draw` | 156 | 136 | +20 |
| `Strcpy` | 51 | 39 | +12 |
| `MakeJmp` | 45 | 34 | +11 |
| `Next` | 40 | 30 | +10 |
| `Xloop` | 221 | 213 | +8 |
| `Tag` | 99 | 91 | +8 |
| `SwapScr` | 198 | 190 | +8 |
| `Ord` | 77 | 69 | +8 |
| `Copy` | 668 | 660 | +8 |

Several large routines are already smaller in MIR6502:

| Routine | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| `Init` | 105 | 146 | -41 |
| `Convert` | 148 | 171 | -23 |
| `InputLine` | 197 | 210 | -13 |
| `Rename` | 180 | 192 | -12 |
| `Delete` | 154 | 165 | -11 |
| `SavePos` | 24 | 34 | -10 |
| `Format` | 267 | 274 | -7 |
| `NewDrive` | 201 | 208 | -7 |
| `Sort` | 258 | 265 | -7 |
| `Value` | 11 | 17 | -6 |

Across the 104 matched routines, positive differences total 281 bytes and
negative differences total 173 bytes, for a net 108-byte MIR6502 deficit. The
MIR-only `<program>` procedure contributes another 49 instruction-form bytes,
giving the exact 157-byte instruction-form difference above.

Routine size alone is not a priority signal. `SetWin`, for example, remains
the largest spill-pressure routine but is only one instruction-form byte
larger than classic. Conversely, `PopUp` is 23 bytes larger without any final
temp home, so a general spill pass cannot address its gap.

## Ranked opportunities

### 0. Repair the listing-quality parser

This does not change generated output, but it is a prerequisite for trustworthy
iteration. Parse the variable-width raw byte field, add a long-`.BYTE`
regression, and keep instruction and data rows distinct even inside a
procedure.

### 1. Elide write-only private formal backing

The strongest bounded code-generation opportunity is no longer a broad
instruction pattern. It is formal-parameter storage whose materialized MIR has
only stores and no load, address use, or indirect access.

Seven routines already have direct modern/classic evidence that their unused
entry captures may be omitted:

| Routine | Write-only parameter bytes | Entry stores | Direct removable payload |
| --- | ---: | ---: | ---: |
| `PrintE` | 2 | 2 | 8 |
| `Next` | 2 | 2 | 8 |
| `IsTagged` | 1 | 1 | 4 |
| `IsProtected` | 1 | 1 | 4 |
| `IsDirectory` | 1 | 1 | 4 |
| `FindNext` | 1 | 1 | 4 |
| `GoTo` | 1 | 1 | 4 |
| Total | 9 | 9 | 36 |

Each entry store is a three-byte absolute `STA`/`STX`; each otherwise unused
formal byte also occupies one loadable data byte. The 36-byte total is the
direct static payload, before any secondary branch or layout effect. It is
already more than the current 10-byte XEX gap.

This should be implemented as a MIR6502 counterpart to the existing
proof-guided classic optimization, using routine identity, parameter storage
identity, address observability, and machine/effect barriers. It must not be
confused with the corrected Action ABI: A/X/Y are incoming argument locations,
not `$A0-$A2` shadow homes.

There is a second tier after parity:

- `Key` has two write-only formal cells and four stores to them. Removing the
  stores and cells has a 14-byte direct ceiling.
- `SetWin`'s `n` cell has one write and no later physical consumer, for another
  four-byte ceiling.

These require a stronger whole-routine proof because `Key` assigns the formal
again and `SetWin` has substantial local storage. They should follow, not be
folded into, the first conservative slice.

`Free` and `Push` also contain write-only formal cells in materialized MIR, but
classic retains those cells and their current routine/observability contracts
do not provide the same proof. `MakeJmp` has a write-only formal byte followed
by an opaque machine block. These are explicit non-targets unless their
observability contracts are strengthened separately.

### 2. Plan surviving values across A, X, Y, and pointer pairs

The final materialized MIR is unchanged from the preceding dual-pointer
reanalysis: 60 temp homes remain, 43 in zero page and 17 in RAM. Their emitted
traffic is 46 ZP stores, 48 ZP reloads, 21 RAM stores, and 35 RAM reloads. The
approximately 375-byte encoded burden plus RAM backing is gross cost, not an
achievable saving; many values cross calls, joins, or clobbers.

After formal backing elision, inspect the concentrated positive routines:

- `Window` is +23 bytes and has three ZP homes plus one RAM spill with four
  accesses.
- `Draw` is +20 bytes and has four ZP homes.
- `Strcpy` is +12 bytes and has two ZP homes.
- `Handle` is +30 bytes, but its dual-pointer work has already removed all RAM
  spill accesses; only three ZP homes remain.
- `PopUp` is +23 bytes with no final temp home and needs a separate call/local
  access comparison rather than spill work.

The next selector should use producer and consumer constraints to keep values
in A, X, Y, an ABI result location, or one of the two private pointer pairs.
A generic allocator or cross-routine home pool is not the first response:
pooling changes backing bytes but leaves the expensive access instructions.

### 3. Revisit high-pressure homes only with a specific consumer

`SetWin` still has 36 spill accesses across ten RAM spill labels and 16 total
temp homes. `Sort`, `Window`, `Draw`, and `Copy` are the next concentrations.
This is worth revisiting only when a concrete producer/consumer family can
remove a home. `SetWin` is only one instruction-form byte larger than classic,
so its raw pressure score does not explain the remaining parity gap.

## Closed or lower-priority families

- The 28 MIR6502 branch-over-`JMP` veneers have no branch-reachable final
  target. Forward-branch relaxation has no current TN site.
- Four `JSR; RTS` pairs remain in both backends and do not explain the gap.
- Twenty-one adjacent same-home `STA`/`LDA` pairs remain versus sixteen in
  classic. Several are loop headers with incoming backedges, so adjacency is
  not proof of redundancy.
- Scaled `(zp),Y` selection already applies at 30 sites. The two remaining
  distinct blocks (`MakeJmp`, flags live; `Handle`, home live) do not justify
  weakening their proofs.
- The remaining binary-to-compare origins are home-free numeric bitwise
  conditions and are closed.
- Cross-routine RAM-home pooling has a small static ceiling and does not remove
  load/store traffic.
- Further deferred-data work is not indicated: explicit `.BYTE` payload is
  already 147 bytes smaller than classic.

## Recommended order

1. Fix `actionc-listing-quality` and lock the exact main-segment accounting in
   a regression test.
2. Port conservative private parameter-storage elision to MIR6502 for the seven
   classic-proven routines, then remeasure the XEX.
3. Extend the same proof to write-only reassigned formals only after the first
   slice is stable.
4. Reaudit `Window`, `Draw`, `Strcpy`, `Handle`, and `PopUp` using normalized
   routine comparisons and location-aware producer/consumer facts.
5. Return to the general 60-home census only when those comparisons expose a
   reusable value-location family.

## Artifact manifest

Fresh artifacts are under `target/tn-final-listing-audit-20260724/`:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `TN-pre.mir` | 130,090 | `58f8a8488af6086e88e6652bdd7ee569fa7b7154d216f89850263b2e727e9207` |
| `TN-materialized.mir` | 151,263 | `cfff7883791ede25edce3bbc9a938f39b84004954eb8ccf6a84a579aa8d7f567` |
| `TN-mir6502.lst` | 139,097 | `5f447d3abd62ab9b07ad6149d889f03175458532a5b922e3f54428e34707ffd4` |
| `TN-mir6502.map` | 10,967 | `68f38e56001bece4f5a411f25dcc6889d74b52504a89df59b31cec418901928d` |
| `TN-mir6502.peepholes` | 274,886 | `ffef0e6207e04338f0411f1ba18eb242694dedeff3ea6e19718b8c2c64cd1865` |
| `TN-mir6502.quality` | 3,261 | `cd24e849088d5b354637d7db5b0d1433d4b5e1b8d90b89cdfadbe9efa484e2be` |
| `TN-mir6502.xex` | 10,455 | `fae5549f58c41227825fe41eb6ab26e80bd6cdb615b5d82e176ac1e4e273a6ae` |
| `TN-classic.lst` | 136,233 | `668fb6ad3376a4449c6d79d2ae3050ca4deb415325002642cc244c9404750552` |
| `TN-classic.map` | 60,393 | `ee61c594c666290be91bd2deb7754f0132296c26f44161e41c15044540cf0cec` |
| `TN-classic.quality` | 2,721 | `62cc59f313283e5ac72f27674aa7dd26071c8197a2443ff49c7e49f756a7a794` |
| `TN-classic.xex` | 10,445 | `3caefd677ab3d1489e39fcc0200126b442a15278b26a9cb5351434a1c8674f39` |

## Reproduction

```sh
mkdir -p target/tn-final-listing-audit-20260724

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/tn/modern/TN.ACT \
    > target/tn-final-listing-audit-20260724/TN-mir6502.lst \
    2> target/tn-final-listing-audit-20260724/TN-mir6502.peepholes

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-listing \
  samples/tn/modern/TN.ACT \
  > target/tn-final-listing-audit-20260724/TN-classic.lst

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-final-listing-audit-20260724/TN-pre.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-final-listing-audit-20260724/TN-materialized.mir

for backend in mir6502 classic; do
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend "$backend" --emit-map \
    samples/tn/modern/TN.ACT \
    > "target/tn-final-listing-audit-20260724/TN-$backend.map"

  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend "$backend" --emit-load \
    samples/tn/modern/TN.ACT \
    > "target/tn-final-listing-audit-20260724/TN-$backend.xex"

  cargo run --quiet --bin actionc-listing-quality -- \
    "target/tn-final-listing-audit-20260724/TN-$backend.lst" \
    > "target/tn-final-listing-audit-20260724/TN-$backend.quality"
done
```
