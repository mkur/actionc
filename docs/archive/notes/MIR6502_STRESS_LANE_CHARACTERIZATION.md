# MIR6502 Stress Lane Characterization

Snapshot date: 2026-06-03.

This note records the small virtual byte-lane shapes from
`docs/MIR6502_CAUTIOUS_STRESS_MATERIALIZATION_PLAN.md` Phase 3. It is evidence
only; it does not change the MIR contract.

After `ccf5871 mir6502: legalize zero-page stress byte rhs temps`, the current
stress classifier reports no materialized-MIR `virtual temp byte` bucket. The
old lane failures below were inspected from the refreshed stress dump that
motivated the cautious plan.

## Small Lane Shapes

### `arrays`: `Neg:bb2`, `v0.b0` / `v0.b1`

Producer:

```text
v0 =.w load param p0+0
```

Consumer:

```text
v1 =.w #$00 sub v0 carry_in=- carry_out=ignore
store.w fixed_zp $A0, v1
```

Shape:

- both lanes are consumed immediately by the split byte subtraction;
- no call or branch sits between producer and consumer;
- signedness is not used by the subtraction itself;
- the consumer home is the fixed zero-page word return area.

### `records`: `Walk:bb7`, `v4.b0` / `v4.b1`

Producer:

```text
v4 =.w load *global g3+0+2
```

Consumer:

```text
v5 =.w v3 add v4 carry_in=- carry_out=ignore
store.w local l1+0, v5
```

Shape:

- both lanes are consumed by the split byte add;
- no call or branch sits between producer and consumer;
- the word load is from an indirect record-field address;
- the result is immediately stored to the local word accumulator.

### `arithmetic_control`: `SignedMix:bb8`, `v9.b0` / `v9.b1`

Producer:

```text
v9 =.w load local l0+0
```

Consumer:

```text
v10 =.w #$00 sub v9 carry_in=- carry_out=ignore
store.w local l0+0, v10
```

Shape:

- both lanes are consumed by the split byte subtraction;
- the producer and consumer are in the same branch target block;
- no call sits between producer and consumer;
- signedness was used by the earlier branch condition, not by this lane
  consumer.

## Current Interpretation

These three failures were not independent lane-lifetime bugs. They were exposed
when word operations split into byte-lane operations whose RHS was a virtual
temp or virtual temp byte. Materializing RHS temp operands to their spill homes
legalizes these shapes without weakening pre-emission verification.

The next remaining stress materialization work should be selected from the
current classifier output, not from these stale lane diagnostics.
