# MIR6502 WARPDEM Optimization Implementation Plan

Status: active

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

### Slice 1: Preserve routine-wide temp widths in materialization

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

### Slice 2: Validate Y provenance and same-index reuse

- Re-audit WARPDEM after Slice 1 and count redundant adjacent `LDY` operations.
- First verify that the shared machine-state dataflow sees
  `AbsoluteIndexedY` as reading, not clobbering, Y.
- If needed, extend existing edge/straight-line register propagation to retain
  exact Y provenance across safe direct indexed loads and stores.
- Require an unchanged index definition/home and no physical alias write,
  register clobber, call, machine block, or unknown effect.
- Add positive, branch-edge, source-write, and clobber rejection tests.

### Slice 3: Keep typed byte constant shifts in registers

- Re-audit `HStick` after Slices 1-2.
- Select one or more `LSR A` operations for a byte-width unsigned `RSH` by a
  small constant.
- Feed the result directly to a byte-indexed lookup or byte consumer when
  liveness and register/flag demand allow.
- Reject signed shifts, word operands/results, variable counts, observable
  intermediate flags, and clobbering consumers.
- Protect the behavior with a runtime fixture covering values around the shift
  and table-index boundaries.

### Slice 4: Residual listing audit and targeted closeout

- Regenerate WARPDEM listing, materialized MIR, map, quality, XEX, telemetry,
  and per-routine metrics.
- Compare modern/classic and modern/MIR6502 by code bytes, data bytes,
  instruction counts, load/store traffic, logical homes, and routines.
- Re-run ALLOCATE, SORTDM1, CIRCLE, KALSCOPE, TN, and the full Toolkit sweep.
- If one residual family is both material and general, add a separately
  documented slice under Finding 4. Otherwise stop rather than accumulating
  marginal local peepholes.
- Update this note with final measurements and committed slice hashes.

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

