# Implementation plan for 6502 write-through contracts

The acceptance case is the existing TN call `Range(..., limit, @c)`. Its third
argument points to one byte that `Range` may read and update. MIR6502 should
identify that byte as `PopUp.c` and let the allocator evaluate `PopUp.d`,
`mp` and `limit` against the remaining physical conflicts. Implement and
commit each slice with its tests. [Shared contract](6502_SHARED_CALLEE_FACTS_DESIGN.md#write-through-contracts) · [PopUp placement](MIR6502_PRIVATE_ZP_HOME_PLACEMENT.md#tn-examples)

## 1. Carry a pointer argument through a native control-flow join

Start from the existing reviewed `ArgumentBuffer` and MIR6502 `FormalWrite`.
The reviewed clause `writes-buffer 3 bytes=1` uses a one-based parameter
number. For `Range(s, max, c)`, the signature maps that pointer to zero-based
ABI bytes 3 and 4. Both native and reviewed writers should reach the same
call-site projection with offset zero and extent one. Run the existing
both-backend fixed-buffer review test alongside the native case.
[Shared facts](../src/callee6502.rs) · [Review parity test](../src/mir6502/analysis/known_callees/reviewed_inputs.rs) · [Native projection](../src/mir6502/analysis/known_callees/native_writes.rs)

Capture the TN home trace, maps and XEX sizes on both runtimes before changing
inference. Identify the selected MIR operations for both updates. The original
`origin` walk stops at a block with multiple predecessors. Use typed MIR
copies and the matched `SArgsCapture` mapping to retain a pointer when both bytes come from the same incoming formal on every
reachable predecessor. Test a branch that rejoins before the store, a branch
that skips the store, and a branch that replaces only the high byte. The first
two should publish a one-byte *possible* write. The third must cover the
alternative destination or report it unknown. Verify that a general memory
query still observes the read in a read-modify-write. [Native write tests](../src/mir6502/analysis/known_callees/native_writes/tests.rs)

## 2. Prove the pointer survives repeated updates

Use the existing CFG dataflow solver to propagate entry origins to a fixed
point. Seed the entry from the ABI, distinguish an unvisited edge from an
unknown value, and keep an origin at a loop header only while all reachable
inputs agree. Compute this once per routine and selected layout. Union the
write spans of both conditional stores. Repeating a store to the same byte
does not increase its extent. [Dataflow solver](../src/analysis/dataflow.rs) · [Callee summaries](../src/mir6502/analysis/known_callees.rs)

Account for anything that can change the two bytes holding the pointer before
the next use. An opaque call contributes its matched preservation facts. If a
store might overwrite a pointer byte that will be read again, carry that
disjointness requirement to the call site and use the bound only once it is
proved. This matters even for a one-byte store in a loop. Test a stable
pointer across an opaque call, a pointer advanced on the backedge, a callee
that clobbers one pointer byte, and a numeric destination that overwrites a
pointer byte on the first iteration. The latter cases must account for the
redirected write or leave it unknown. [Opaque call rules](6502_SHARED_CALLEE_FACTS_DESIGN.md#opaque-reviewed-calls-at-every-depth)

## 3. Apply the bound at each caller and verify the placement

Extend `writes_at_site` and its existing callers where needed. Resolve a
retained `@object` using its storage identity and bounds, and a staged pointer
using its physical ABI bytes. Forward an unresolved formal and its obligations
through a wrapper. Instantiate it afresh at each call. Combine overlapping
argument spans and check numeric ranges with 16-bit wrap. Keep an unresolved
destination conservative. [Call-site projection](../src/mir6502/analysis/known_callees/native_writes.rs) · [Residence queries](../src/mir6502/parameter_homes/calls.rs)

Exercise selection and expanded lifetime verification with the same writer.
Passing `@c` must keep `c` addressable and allow an independent live `d`
to use a disjoint home. Passing a numeric pool address must reject a supplied
home covering that byte, including the high byte of a word home. Also check
a forwarded destination, two aliased arguments, and a word write starting at
`$FFFF`. Keep the fixed writes from argument staging, scratch and descendants
in every case, and replay physical observations against the selected layout.
[Placement tests](../tests/mir6502_cross_call_homes.rs) · [Lifetime verifier](../src/mir6502/parameter_homes/lifetimes.rs)

## 4. Qualify the shipped TN program

Compare before and after builds of modern TN at `$3000` for cartridge and
standalone. At `PopUp → Range`, require the trace or a focused summary test
to show a one-byte write through `@c` alongside `Range`'s `$A0–$A3` scratch,
its parameter homes and the matched `Getchar` preservation guarantee. The former blanket rejection of
`d`, `mp` and `limit` should disappear at that call. Record each final home
or remaining conflict, total ZP and inline homes, and XEX byte deltas. A word
still needs two adjacent free bytes. Update the home report from the measured
maps. [TN home report](benchmarks/6502-tn-home-breakdown/README.md)

Reuse the TN keyboard VM test for increment, decrement, a rejected key followed
by exit, and menu selection. It already runs classic and MIR6502 on both
runtimes. Add a focused assertion that the caller retains the old value in
`d` while it observes the update through `@c` on the path that compares
them. Use the existing TN source and matched `Getchar` review for this check.
[TN keyboard VM test](../tools/vm-runtime-tests/tests/tn_dispatch.rs)

Run the affected targets as each slice lands.

```sh
cargo test --lib mir6502::analysis::known_callees::
cargo test --lib mir6502::parameter_homes::
cargo test --test mir6502_cross_call_homes --test mir6502_parameter_homes
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes --test tn_dispatch
```

## Qualification results

The selected MIR has two byte stores through `Range.c`. Both branches use
offset zero. The loop proof carries the condition that the argument cannot
redirect a later store by overwriting a live pointer byte. Passing `@PopUp.c`
discharges that condition. The VM observes menu positions 0, 0, 1 when the
first key advances the selection, and only 0 when Escape leaves it unchanged.
The check runs in classic and MIR6502 with both runtimes. [Keyboard VM test](../tools/vm-runtime-tests/tests/tn_dispatch.rs)

Discovery can use the callee homes already chosen in the same allocation
batch. Final lifetime verification rebuilds the write summaries with the
actual expanded frame. Determining a one-byte write permits `d` to take
`$EF`. The report records the remaining conflicts and the eight-byte XEX
increase when two spill homes lose that byte. [TN measurements](benchmarks/6502-tn-home-breakdown/README.md#measurement)
