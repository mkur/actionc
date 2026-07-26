# MIR6502 WARPDEM Optimization Implementation Plan

Status: complete

Date: 2026-07-26

Planning baseline: `51d5b3a`

Primary source:
`corpora/toolkit/original/extracted/WARP.DEM`

Scope: modern profile, MIR6502 backend

## Objective

Close the current WARPDEM size and load/store-traffic gap by preserving typed
MIR facts through materialization and selecting native 6502 addressing for
ordinary byte arrays. Follow-up work should target only the residual patterns
that remain after that structural correction.

No optimization may inspect WARPDEM routine names, source paths, variable
names, or literal table addresses. Classic output is a strategy comparison,
not a correctness oracle.

## Baseline Artifacts

Generate the comparison with:

```sh
tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/warpdem-audit-20260726 \
  --no-diffs \
  corpora/toolkit/original/extracted/WARP.DEM
```

The audit artifacts are under:

```text
target/warpdem-audit-20260726/warp/
```

Baseline load hashes:

| Backend | SHA-256 |
| --- | --- |
| modern/classic | `0ccdd7f1dd93e9a7de657571b049c75c49b1961deb583488013fc08db28902c9` |
| modern/MIR6502 | `6c9560a01cb2c1ebf16d0f77749e82e22a91941e9f203918c8c5e2589906158e` |

## Baseline Measurements

| Metric | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| XEX bytes | 7,102 | 7,402 | +300 |
| Recognized instruction bytes | 5,980 | 6,282 | +302 |
| Data and inline machine bytes | 1,110 | 1,108 | -2 |
| Recognized instructions | 2,687 | 2,855 | +168 |
| `LDA` | 753 | 874 | +121 |
| `STA` | 658 | 738 | +80 |
| `LDA` + `STA` instruction share | 52.5% | 56.5% | +4.0 points |

MIR6502 reports 98 final logical frame bytes: 80 virtual-zero-page bytes and
18 RAM bytes. Residual home traffic includes 132 RAM reloads, 44 RAM stores,
130 virtual-ZP reloads, and 108 virtual-ZP stores. The final byte deficit is
therefore an instruction-selection and transient-traffic problem, not emitted
data growth.

### Routine concentration

The figures below are recognized instruction bytes.

| Routine | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `MissileMove` | 423 | 566 | +143 |
| `MissileFire` | 246 | 340 | +94 |
| `HITBALL` | 282 | 352 | +70 |
| `BlownAway` | 722 | 760 | +38 |
| `Align` | 400 | 433 | +33 |
| `HStick` | 45 | 70 | +25 |
| `ShipMove` | 243 | 264 | +21 |
| `TestHit` | 354 | 269 | -85 |
| `Draw7` | 137 | 92 | -45 |
| `EraseShip` | 124 | 102 | -22 |
| `ShipDraw` | 247 | 233 | -14 |
| `VStick` | 43 | 32 | -11 |

The first seven routines contribute 424 gross excess bytes. Four smaller
MIR6502 routines already recover 166 bytes, so broad regressions are not the
problem.

## Finding 1: Typed Loop Indices Lose Their Width at Materialization

WARPDEM carries many loop indices through MIR block parameters:

```text
b3(v53:byte):
  v6 =.b load computed global_addr g70[v53;1]+0
```

The direct byte-index selector already knows how to lower an ordinary
`GlobalAddr` or `StaticAddr` base to `AbsoluteIndexedY`. It accepts constants,
direct byte cells, registers, and explicit byte temp lanes.

The compiler's per-block materialization step, however, reconstructs its temp
width table from operations in that block and does not seed the table from
`MirBlockParam::width`. A byte block parameter consequently appears as an
untyped whole `VTemp`. The selector conservatively treats it as a possible word
index and emits:

```text
load loop-index home
materialize_indexed (zp$AC),y <- global_addr + a*1
load_indirect (zp$AC),y
```

instead of:

```text
ldy loop-index home
lda global,y
```

This is a MIR6502 fact-plumbing defect. NIR retains the typed indexed operation,
and no SemIR lookup or source-specific rule is needed.

The pattern dominates `MissileMove`, `MissileFire`, `HITBALL`, `BlownAway`,
and the byte-array portion of `Align`.

## Finding 2: Adjacent Same-Index Array Operations May Reload Y

WARPDEM frequently reads and writes several direct byte arrays with the same
loop index. Once Finding 1 selects absolute-indexed operands, the output may
still reload Y for adjacent operations even when:

- the index home is unchanged;
- Y is not clobbered;
- no call, machine block, indirect write, or unknown effect intervenes.

The existing shared machine-state and post-home rewrite infrastructure should
remove these reloads if its exact Y provenance survives the new forms. This is
an audit item first, not a new special-purpose peephole.

## Finding 3: HStick Widens a Constant Byte Shift

`HStick` computes a byte table index from:

```action
value((ports(port)&$C) RSH 2)
```

Classic keeps the value in A, applies two `LSR` instructions, and indexes the
table directly. MIR6502 currently materializes a wider shift path and then
stages the lookup. After direct byte indexing is fixed, inspect the remaining
shape and teach the general byte-expression selector to retain a byte
constant-right-shift in A/Y when its typed result is byte-sized.

The rule must not truncate a word shift merely because the final consumer is a
byte. The proof must come from the MIR operand/result width.

## Finding 4: Residual Word-Array and Destination Placement

`Align`, `ShipMove`, and portions of the missile routines contain indexed CARD
arrays, signed word updates, and values copied through temporary homes before
their final compare or store. Existing scaled `(zp),Y`, word-chain placement,
definition-sensitive dead-store, and compare-to-branch machinery already
covers parts of this space.

Re-audit these routines only after Slices 1-3. Add a new rewrite only for a
measured residual family with shared liveness, reaching-definition,
alias/effect, and machine-state proofs.

## Decisions Fixed by This Plan

1. Block-parameter widths are part of MIR and must remain available to every
   MIR6502 selection/materialization phase.
2. Native absolute-indexed addressing is valid only for a structured direct
   base and a proven byte-sized index.
3. Pointer values, array parameters, arbitrary dereferences, word indices, and
   volatile/absolute aliases retain conservative pointer materialization unless
   independently proven safe.
4. A 6502 indexed operand may cross a page; this is normal 16-bit effective
   address behavior and is not a rejection condition.
5. Calls, machine blocks, hardware memory, and unknown effects remain barriers
   to cross-operation register reuse.
6. No NIR change is planned. If a required type or storage identity is absent
   from verifier-clean NIR/MIR, stop and amend this note instead of consulting
   SemIR from MIR6502.
7. Each behavior-changing slice receives focused emitted-shape tests, an
   execution-level runtime check, a fresh WARPDEM audit, and its own commit.

## Implementation Slices

### Slice 0: Baseline and execution coverage

Status: complete.

- Add a real-source WARPDEM quality test. Missing source is a failure.
- Assert the important baseline materialized shapes and cap the current
  7,402-byte MIR6502 output.
- Add a compact VM fixture that carries a BYTE index around a loop and checks:
  - dynamic reads and writes of direct byte arrays;
  - copies between two direct arrays using the same index;
  - indices and bases that cross a page boundary;
  - first and last byte indices.
- Run the fixture under modern/classic and modern/MIR6502.

This slice must be code-size neutral.

Result:

- Added a mandatory real-source WARPDEM materialization and 7,402-byte output
  gate.
- Added a modern/classic and modern/MIR6502 VM oracle for a BYTE loop index,
  same-index array copying, unaligned direct bases, indices 0/1/127/128/255,
  and before/after sentinels.
- The baseline remains byte-identical.

### Slice 1: Preserve routine-wide temp widths in materialization

Status: complete.

- Build one routine-wide MIR temp-width catalog from operation definitions and
  typed block parameters.
- Diagnose contradictory width facts rather than silently choosing one.
- Pass the catalog into per-block selection/materialization.
- Canonicalize a proven byte `VTemp` index to its low byte lane before the
  existing direct byte-index selector runs.
- Keep word indices and unknown-width temps on the current full-address path.
- Add positive tests for entry and loop block parameters and rejection tests
  for word/unknown indices.
- Record direct-byte-index selections and width-related blockers.

Expected primary result: replace repeated `$AC/$AD` address construction in
the five array-heavy routines with `absolute,Y`.

Result:

- Added one shared routine temp-width catalog, including typed block
  parameters, and reused it in home-demand analysis and materialization.
- Block-local definition facts retain priority; the routine catalog supplies
  missing facts after CFG argument lowering. Conflicting routine observations
  conservatively widen to word.
- A proven BYTE temp index is narrowed only in the selector's copy of the
  operand. Existing delayed-index producer matching therefore remains intact.
- WARPDEM selects 77 typed byte indices. Total `materialize_indexed` operations
  fell from 94 to 26, and the `GlobalAddr` subset fell from 39 to 11.
- MIR6502 WARPDEM fell from 7,402 to 6,626 bytes: 776 bytes smaller than its
  own baseline and 476 bytes smaller than modern/classic.
- Recognized instruction bytes fell from 6,282 to 5,506, versus 5,980 for
  modern/classic. Recognized instructions fell from 2,855 to 2,436.
- The direct-array VM oracle, all 1,954 library tests, all integration tests,
  and all 20 modern/MIR6502 Toolkit programs pass.

### Slice 2: Validate Y provenance and same-index reuse

Status: complete.

- Re-audit WARPDEM after Slice 1 and count redundant adjacent `LDY` operations.
- First verify that the shared machine-state dataflow sees
  `AbsoluteIndexedY` as reading, not clobbering, Y.
- If needed, extend existing edge/straight-line register propagation to retain
  exact Y provenance across safe direct indexed loads and stores.
- Require an unchanged index definition/home and no physical alias write,
  register clobber, call, machine block, or unknown effect.
- Add positive, branch-edge, source-write, and clobber rejection tests.

Result:

- Added an optional final-layout memory map to the shared routine-wide
  machine-value analysis and post-home snapshot. Clients that do not have a
  final layout retain the previous conservative behavior.
- Exact direct writes and the bounded 256-byte address range of
  `absolute,X/Y` stores now invalidate a memory-backed register fact only when
  the physical ranges can overlap. Unresolved storage, indirect writes,
  machine blocks, absolute/ABI-observable memory, and unknown effects still
  invalidate the fact.
- The existing late reload fold now uses those shared facts and the existing
  flag-liveness proof; no listing-only or WARPDEM-specific matcher was added.
- Added positive straight-line and CFG-edge tests, plus physical-alias,
  indexed-range-overlap, live-flag, and unknown-indirect-write rejection tests.
- WARPDEM's proven/rejected Y-reload census increased from 9 to 23, removing 14
  additional `LDY absolute` instructions. `LDY` count fell from 124 to 110.
- MIR6502 WARPDEM fell from 6,626 to 6,584 bytes, 42 bytes smaller than Slice 1
  and 518 bytes smaller than modern/classic.
- Recognized instruction bytes fell from 5,506 to 5,464 and recognized
  instructions from 2,436 to 2,422. Data remains 1,108 bytes.
- The direct-array VM oracle, all 1,961 library tests, all integration tests,
  and all 20 modern/MIR6502 Toolkit programs pass.
- Final MIR6502 load SHA-256:
  `244c4793d1dad7dfbe88210bf893c57755b4b86b0215640ea2905522561ca352`.

### Slice 3: Keep byte-range word expressions in registers

Status: complete.

- Re-audit `HStick` after Slices 1-2.
- Prove the high byte remains zero when a typed BYTE value is widened into a
  word `AND`/`OR`/`XOR` chain or a logical `RSH` by a constant from 0 through 8.
- Delay only the proven expression definitions to an indexed word-array read,
  lower the surviving operations at byte width in A, and select the existing
  scaled `(zp),Y` word read.
- Reject word-range inputs, additions, subtractions, left shifts, variable
  shift counts, carry dependencies, extra uses, and unsupported consumers.
- Protect the behavior with both emitted-shape tests and a classic/MIR6502 VM
  fixture covering all four HStick-style table indices.

Result:

- The original plan's typed-result assumption was corrected: HStick's
  `ports(port)&$C` and `RSH 2` results are formally `CARD`, not BYTE. The new
  rule therefore proves a zero high byte through the expression instead of
  truncating a word result.
- The producer rewrite reuses the shared pre-home liveness, use-def, memory,
  carry, and transactional rewrite validation. The initial computed BYTE load
  remains at its original point; only the two word temporaries are elided.
- `HStick` now emits `AND #$0C; LSR; LSR; ASL; TAY` followed by the existing
  carry-preserving scaled `(zp),Y` word lookup. The word-shift runtime helper,
  four transient word-home stores/reloads, and full index materialization are
  gone.
- `HStick` fell from 70 to 34 recognized instruction bytes and WARPDEM fell
  from 6,584 to 6,546 load-file bytes. MIR6502 is now 556 bytes smaller than
  modern/classic for WARPDEM.
- A negative unit test retains the full word-helper path when the input range
  is not proven. The expanded VM oracle checks indices derived from masked
  values 0, 4, 8, and 12 under both modern/classic and modern/MIR6502.

### Slice 4: Residual listing audit and targeted closeout

Status: complete.

- Regenerate WARPDEM listing, materialized MIR, map, quality, XEX, telemetry,
  and per-routine metrics.
- Compare modern/classic and modern/MIR6502 by code bytes, data bytes,
  instruction counts, load/store traffic, logical homes, and routines.
- Re-run ALLOCATE, SORTDM1, CIRCLE, KALSCOPE, TN, and the full Toolkit sweep.
- If one residual family is both material and general, add a separately
  documented slice under Finding 4. Otherwise stop rather than accumulating
  marginal local peepholes.
- Update this note with final measurements and committed slice hashes.

Result:

- Final audit artifacts are under
  `target/warpdem-final-audit-20260726/warp/`.
- Modern/MIR6502 emits 6,546 bytes versus modern/classic's 7,102 bytes:
  MIR6502 is 556 bytes, or 7.8%, smaller.
- Relative to the 7,402-byte MIR6502 planning baseline, the implemented slices
  remove 856 bytes, or 11.6%.

| Metric | Planning MIR6502 | Final MIR6502 | Modern/classic | Final vs classic |
| --- | ---: | ---: | ---: | ---: |
| XEX bytes | 7,402 | 6,546 | 7,102 | -556 |
| Recognized instruction bytes | 6,282 | 5,428 | 5,980 | -552 |
| Data and inline machine bytes | 1,108 | 1,106 | 1,110 | -4 |
| Recognized instructions | 2,855 | 2,408 | 2,687 | -279 |
| `LDA` | 874 | 781 | 753 | +28 |
| `STA` | 738 | 635 | 658 | -23 |
| `LDY` | 150 | 109 | 139 | -30 |

- Final logical temp homes fell from 98 to 94 cells: 78 virtual-ZP cells and
  16 RAM cells. Final home traffic is 108 virtual-ZP reloads, 101 virtual-ZP
  stores, 101 RAM reloads, and 40 RAM stores. The listing contains no emitted
  spill-data labels.
- All 23 proven Y-reload candidates are elided. The delayed-index selector
  reports 24 consumers and 36 producer operations.

### Final routine comparison

The remaining gross routine deficits total only 84 recognized instruction
bytes. The five largest are:

| Routine | Modern/classic | Final MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `ShipMove` | 243 | 264 | +21 |
| `ScrollColors` | 117 | 130 | +13 |
| `FastDraw` | 191 | 201 | +10 |
| `ShipFly` | 183 | 191 | +8 |
| `DLI` | 65 | 72 | +7 |

These are outweighed by 636 bytes of routine wins, led by:

| Routine | Modern/classic | Final MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `BALLMOVE` | 615 | 405 | -210 |
| `MissileMove` | 423 | 311 | -112 |
| `TestHit` | 354 | 269 | -85 |
| `BlownAway` | 722 | 676 | -46 |
| `Draw7` | 137 | 92 | -45 |

`ShipMove` contains four word-negation results staged through private
two-byte homes before being copied back to their source globals. General
definition-sensitive word-unary destination placement could recover roughly
16 bytes there. It is the only coherent residual family, but it is small,
does not affect WARPDEM's now-favorable overall comparison, and deserves a
separate cross-program audit rather than another sample-led slice.

`ScrollColors` and `FastDraw` are mixtures of loop-bound orientation, fixed
local placement, pointer construction, and runtime ABI strategy. No single
safe general rewrite accounts for a material part of either difference.

### Cross-program closeout

Both modern Toolkit presets compile all 20 entries. Across their full outputs:

| Result | Count |
| --- | ---: |
| MIR6502 smaller than modern/classic | 17 |
| Equal | 1 |
| MIR6502 larger | 2 |

The combined modern/MIR6502 Toolkit output is 41,595 bytes versus 44,081 for
modern/classic, a reduction of 2,486 bytes. Relative to the Slice 2 sweep, the
new byte-range rule changes only `JOYSTIX.COM` and `WARPDEM.COM`; each shrinks
by 38 bytes.

The selected regression sizes remain:

| Program | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `TN` | 10,445 | 9,920 | -525 |
| `ALLOCATE` | 935 | 876 | -59 |
| `SORTDM1` | 4,129 | 3,268 | -861 |
| `SORTDM2` | 2,607 | 2,607 | 0 |
| `CIRCLE1` | 625 | 601 | -24 |
| `CIRCLE2` | 889 | 881 | -8 |
| `KALSCOPE` | 3,318 | 3,276 | -42 |

Final load SHA-256:

| Backend | SHA-256 |
| --- | --- |
| modern/classic | `0ccdd7f1dd93e9a7de657571b049c75c49b1961deb583488013fc08db28902c9` |
| modern/MIR6502 | `1c496a7684843ffb20d331e2e2556e4d6c2513526dc28379195d0e63bcd4adb9` |

Implementation commits:

| Slice | Commit |
| --- | --- |
| Plan | `b45d720` |
| Runtime/quality baseline | `bd0f0c3` |
| Routine-wide index widths | `db2bcfd` |
| Y provenance across disjoint stores | `a674eae` |
| Byte-range word index selection | `a420afa` |

Validation:

- the expanded direct BYTE and byte-derived CARD index VM gate passes under
  modern/classic and modern/MIR6502;
- all 759 MIR6502-filtered tests pass;
- the complete `cargo test` suite passes, including 1,963 library tests;
- both final Toolkit preset sweeps pass all 20 entries with no gate failures.

## Required Checks Per Behavior-Changing Slice

```sh
cargo test mir6502
cargo test --test compatibility -- --ignored <warp-runtime-test-name>
cargo test
tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/warpdem-audit-20260726 \
  --no-diffs \
  corpora/toolkit/original/extracted/WARP.DEM
```

Because the planned work stays inside MIR6502, the NIR fixture and sweep gates
are not required unless implementation changes NIR, semantic lowering, the NIR
verifier, or the NIR printer.
