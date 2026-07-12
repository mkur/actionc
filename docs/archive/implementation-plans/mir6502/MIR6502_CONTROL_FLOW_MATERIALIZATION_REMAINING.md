# MIR6502 Control-Flow Materialization Remaining

Snapshot date: 2026-06-03.

This note records the remaining `control_flow` stress materialization cluster
after the cautious stress slices through `f146284`.

Current classifier summary:

```text
Materialized MIR buckets:
     7 pre-emission MIR cannot contain virtual temp
     6 pre-emission MIR cannot contain word-width pseudo ops

Materialized MIR by fixture:
control_flow                  6  pre-emission MIR cannot contain word-width pseudo ops
control_flow                  7  pre-emission MIR cannot contain virtual temp
```

All remaining materialized-MIR failures are in `control_flow.act`. The common
shape is a word compare or word bit-test that produces a boolean temp used by a
larger byte boolean expression. These are not address-staging failures.

## `ComplexGate:bb13`

Raw MIR:

```text
v13 =.b load param p0+0
v14 =.b load param p1+0
v15 =.b v13 xor v14 carry_in=- carry_out=ignore
v16 =.w v15 and #$0002 carry_in=- carry_out=ignore
v17 = cmp.w v16 ne #$00
...
v25 =.b v17 and v24 carry_in=- carry_out=ignore
```

Failure:

```text
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain virtual temp `v16`
```

Classification:

- `v16` is a word-width bit-test intermediate derived from a byte temp.
- `v17` is a boolean compare result, not a direct branch condition.
- `v17` crosses a call to `NotFlag` before it is consumed by `v25`.
- This likely needs a durable boolean home for the compare result, not flag-only
  branch lowering.

## `SignedGate:bb21`

Raw MIR:

```text
v10 =.w load param p0+0
v11 =.w load param p1+0
v12 = cmp.w.signed v10 ge v11
v13 =.w load param p1+0
v14 = cmp.w.signed v13 ne #$00
v15 =.b v12 and v14 carry_in=- carry_out=ignore
```

Failures:

```text
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain virtual temp `v10`
pre-emission MIR cannot contain virtual temp `v11`
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain virtual temp `v13`
```

Classification:

- `v10` and `v11` are word loads consumed immediately by a signed word compare.
- `v13` is a word load consumed immediately by a signed nonzero compare.
- The compare results are byte boolean temps combined in the same block.
- No call sits between these producers and consumers.

## `Main:bb60`

Raw MIR:

```text
v8 = cmp.b v7 ne #$00
v9 =.w load global g12+0
v10 = cmp.w.signed v9 ge #$00
v11 =.b v8 and v10 carry_in=- carry_out=ignore
```

Failures:

```text
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain virtual temp `v9`
```

Classification:

- `v9` is a global word load consumed immediately by a signed compare against
  zero.
- `v10` is a boolean temp used as a byte operand in `v11`.
- No call sits between `v9` and the compare.

## `Main:bb63`

Raw MIR:

```text
v21 =.w load global g12+0
v22 = cmp.w.signed v21 lt #$00
...
call r1 args=[v23.b -> a] result=v24.b <- return+0
v25 = cmp.b v24 eq #$01
v26 =.b v22 or v25 carry_in=- carry_out=ignore
```

Failures:

```text
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain virtual temp `v21`
```

Classification:

- `v21` is a global word load consumed immediately by a signed compare against
  zero.
- The boolean result `v22` crosses a call before its byte `or` consumer.
- This needs the compare result materialized to a durable byte home.

## `Main:bb61`

Raw MIR:

```text
v31 =.w load global g8+0
v32 = cmp.w v31 gt #$0100
...
v37 =.b v32 or v36 carry_in=- carry_out=ignore
```

Failures:

```text
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain virtual temp `v31`
```

Classification:

- `v31` is a global word load consumed immediately by an unsigned word compare.
- `v32` is a boolean temp consumed by a byte boolean expression in the same
  block.
- No call sits between `v31` and the compare, though the block begins after
  calls to the sweep routines.

## Implementation Guidance

The next code slice should not weaken the pre-emission verifier. A safe first
target is a same-block word compare whose result is consumed as a byte operand
without crossing a call, for example `Main:bb60` or `Main:bb61`.

The call-crossing boolean cases (`ComplexGate:bb13`, `Main:bb63`) should be
handled only after durable boolean homes for materialized word compares are
explicit and tested.
