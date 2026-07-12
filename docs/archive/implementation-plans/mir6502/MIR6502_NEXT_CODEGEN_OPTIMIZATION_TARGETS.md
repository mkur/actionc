# MIR6502 Next Codegen Optimization Targets

Snapshot date: 2026-06-08.

The current TN listing is much smaller than the initial MIR6502 output, but it
is still dominated by load/store traffic. The next optimization slices should
therefore focus on keeping short-lived values in 6502 registers and pointer
scratch state before they become routine storage traffic.

## Part 1: Expression-Tree Lowering Before Materialization

Recognize short producer/consumer chains before temps are assigned homes:

- `load -> binary op -> binary op -> store`
- `load -> address calc -> load/store`
- `load/compare -> branch`

Lower these as accumulator/Y-register plans directly when all intermediates are
single-use inside one basic block. Do not cross calls, barriers, machine blocks,
or control-flow joins.

## Part 2: Y-Register Index Residency

Track when `Y` already contains a byte index inside a block. Reuse it across
adjacent array/string accesses instead of reloading the same index temp.

## Part 3: `$AC/$AD` Pointer Residency

Track when the default indirect pointer scratch still contains a known pointer.
Avoid restaging `$AC/$AD` for repeated nearby `(zp),Y` array/string/pointer
accesses. Invalidate on calls, opaque blocks, and writes to the scratch pair.

## Part 4: Store-Forwarding Across Memory Homes

Generalize the accumulator-forwarding work beyond adjacent spill loads. When a
stored value is still available in A/X/Y and no clobber intervenes, skip the
reload. Only remove the original store with a separate dead-store proof.

## Part 5: Branch Lowering Cleanup

Reduce branch-over-`JMP` sequences by inverting conditions when the real target
is branchable. Keep long-branch fallback behavior for out-of-range targets.

## Part 6: Call-Adjacent Tail Cleanup

Rewrite safe `JSR f; RTS` tails into `JMP f` when ABI/effects allow.

## Part 7: Inc/Dec Idioms

Fold safe byte `load +/- 1/store` shapes to `INC`/`DEC`, preserving carry chains
for word updates.
