# MIR6502 scaled `(zp),Y` word-element addressing

Date: 2026-07-22.

Scope: port the classic backend's scaled `(zp),Y` CARD ARRAY addressing
strategy to the modern-profile MIR6502 backend, using TN as the primary
measurement workload.

## Result

MIR6502 now keeps the low byte of `2*index` in Y and folds the ASL carry into
the pointer high byte. Word loads and stores then use offsets zero and one from
the same pointer without first adding the whole scaled index to the pointer.

On `samples/tn/modern/TN.ACT`:

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Load file | 12,098 bytes | 11,955 bytes | -143 bytes |
| Full scale-two address materializations | 34 | 4 | -30 |
| Listing `PHP`/`PLP` pairs | 34 | 4 | -30 |
| Recognized instructions | 4,952 | 4,831 | -121 |
| Recognized instruction bytes | 11,094 | 10,956 | -138 |

The final load-file SHA-256 is
`18580ebc851d5a1f4b145bc6943408e68b58da4d9ee8aa4e38f5c13fce25eb33`.
The XEX size is authoritative; the listing-quality parser can interpret bytes
inside machine/data procedures as instructions.

## Implementation slices

1. `e553996` adds an explicit MIR6502 scaled-Y address consumer, printer and
   emitter support, target effects, cost modeling, and basic verification.
2. `ba3d428` selects scaled-Y word reads with post-home register, flag, pointer
   pair, and access-window proofs.
3. `c25fe8b` selects word stores under the corresponding source and liveness
   proofs.
4. `2aacb31` lets same-index word copies prepare two pointer pairs while sharing
   one Y offset, and teaches emission to advance that shared offset only once.
5. The final hardening slice verifies the block-local scaled-Y protocol and
   updates the pre-existing LEA word-array expectations to the selected form.

No NIR form or source-language rule changed. The representation and selection
remain in MIR6502 because Y, flags, pointer pairs, and `(zp),Y` are target
strategy.

## Safety contract

A scaled-Y materialization is legal only for scale two and prepares both a
pointer pair and Y offset zero. Selection requires:

- a byte-sized index and a base that the emitter can split without transient
  target state;
- dead A and processor flags after replacing the classic full-address setup;
- dead Y and pointer-pair homes at the rewritten window exit;
- no intervening Y clobber, pointer-pair touch, or incompatible indirect
  access;
- monotone byte offsets, so emission can use offset zero followed by at most
  one `INY` for offset one.

Pre-emission verification rejects scaled accesses without a matching active
materialization and rejects offset reuse that would require moving Y backward.
Calls, machine blocks, and unproved cross-block state remain barriers.

## Residual TN sites

Four full scale-two calculations remain deliberately unselected:

| Routine | Sites | Reason |
| --- | ---: | --- |
| `Sort` | 2 | The word copy has different source and destination indexes, so one Y value cannot address both pointer pairs. |
| `MakeJmp` | 1 | The result feeds an opaque machine block; Y/flag observability is not narrow enough to prove the replacement. |
| `Handle` | 1 | The prepared pointer state remains live/reused outside the local read window. |

These cases need broader or different transformations, not a relaxation of the
current scaled-Y proof.

## Reproduction

```sh
cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/TN-mir6502.materialized.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-listing \
  samples/tn/modern/TN.ACT \
  > target/TN-mir6502.lst

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/tn/modern/TN.ACT \
  > target/TN-mir6502.xex

cargo run --quiet --bin actionc-listing-quality -- \
  target/TN-mir6502.lst
```

Useful checks:

```sh
wc -c target/TN-mir6502.xex
rg -c 'materialize_indexed .*scaled_y' target/TN-mir6502.materialized.mir
rg 'materialize_indexed .*\*2' target/TN-mir6502.materialized.mir | rg -vc scaled_y
rg -c '\bPHP\b' target/TN-mir6502.lst
rg -c '\bPLP\b' target/TN-mir6502.lst
```
