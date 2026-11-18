# MIR6502 implementation plan for opaque reviewed calls

Make zero-page home analysis consume the same checked assembly contract for a
direct call and for a call reached through a native wrapper. TN's
`Print → Putchar → Internal` is the acceptance case. Use the existing review
and original assembly. The change belongs in MIR6502's consumption of the
shared facts. [Design](6502_SHARED_CALLEE_FACTS_DESIGN.md#opaque-reviewed-calls-at-every-depth)

Land each slice with its regression coverage in one commit. Before slice 1,
capture the MIR6502 home diagnostics, selected homes and XEX sizes for
unmodified TN on cartridge and standalone. Reuse those build settings in slice 3.

## 1. Use the reviewed entry obligations at every call depth

Use the existing review matcher in both direct home discovery and acyclic
composition. At a reviewed assembly leaf, `abi-inputs-only` establishes that
the ordinary ABI arguments suffice. Check canonical argument lanes, result
width and invocation-specific preconditions at each call site. Continue
analyzing typed MIR for the wrapper's setup, branches and memory effects. Reserve
`entry-reads abi` for queries about physical reads before definition.
[Direct home checks](../src/mir6502/parameter_homes/calls.rs) ·
[Acyclic composition](../src/mir6502/analysis/known_callees/entry_inputs/acyclic.rs)

Test a direct call and calls through one and two native wrappers. In each
case `Internal(ch)` requires A and accepts either incoming carry. Its table
read, scratch write and preserved pointer pairs remain visible. Exercise an
incompatible call ABI and a stale body or table dependency. Any enclosing summary that relied on the
invalid claim must lose it. Source assembly stays opaque on a failed match.
Keep the existing conservative handling of recursive and indirect calls.

## 2. Model the returned value in caller liveness

Account for argument and memory reads first. On normal return, expose the
result lanes justified by the typed ABI and matched facts. For `Internal`,
layout proves its table read disjoint from the old `$A0`. A later load of
`$A0` in `Putchar` therefore reads the returned byte. The review also puts
that byte in A at return. Keep the caller-visible result separate from
`defined_results`, whose consumers require a physical write on every
returning path. Today both computations use the same home transfers.
[Entry and result dataflow](../src/mir6502/analysis/known_callees/entry_inputs.rs)

Test a wrapper that consumes the returned byte and one that forwards it.
Include a reviewed leaf that reads the old `$A0` and returns it in A, an
unknown read footprint, and a void leaf with only a possible write. Retain
possible incoming reads in the first two cases. A possible write supplies
no result. Check that forwarding through a wrapper grants no physical
must-write fact and that a conditional call supplies a result only on the
path that executed it.

## 3. Qualify home selection and execution in TN

Exercise the full home planner on the original `Print → Putchar → Internal`
chain. Verify that `Internal` contributes neither incoming carry nor the old
`$A0`. Check the wrapper's own accesses, including its indirect screen
store, when judging a candidate home. Recheck the contract and storage
overlap on the candidate layout. Report remaining blockers by routine and
byte, selected homes and whole-image size deltas for both runtimes. Keep the
planner's existing cost and rollback checks. [TN routines](../samples/tn/modern/LIB.ACT)

Run the existing cartridge VM check for all 256 inputs and both carry values
in classic and MIR6502. Exercise a nested caller on cartridge and standalone
with a live value placed in a disjoint ZP home. Check the stored result and
the surviving value. Use an overlapping home as the rejection control. Run the
focused analysis, placement, physical-flag and VM targets.

```sh
cargo test --lib mir6502::analysis::known_callees::
cargo test --test mir6502_cross_call_homes --test mir6502_callee_entry_flags
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_dispatch --test cross_call_homes
```
