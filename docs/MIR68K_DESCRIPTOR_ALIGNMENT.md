# Descriptor alignment and guarded longword accesses

Implementation follows the four slices in the
[implementation plan](MIR68K_DESCRIPTOR_ALIGNMENT_IMPLEMENTATION_PLAN.md).
This report records measured behavior; the proof and access contracts live in
[NIR target shape](NIR_TARGET_SHAPE.md) and the
[MIR68K execution contract](MIR68K_EXECUTION_CONTRACT.md).

## Static descriptor facts

Runtime assignments can establish even pointer values in mutable descriptor
slots. Native automatic initialization uses explicit entry stores from the
actual backing address. Globals and incoming values remain unknown at entry.
The analysis distinguishes the pointer bytes from the size word and backing,
and invalidates facts on possible overlapping writes. Captured SSA values keep
the facts valid at their load. Calls follow the existing conservative storage
rule: an empty effects record is not a complete transitive user-call summary.
Volatile operations retain the earlier proof policy.

The baseline at `38471f2` and the static-facts slice have identical measurements
for all nine default workloads, in both raw and optimized NIR. In particular,
the optimized shaped matrix remains at 144051 instructions / 1332 code bytes /
88 frame bytes, and shaped DCT at 43279 / 6658 / 886. These workloads use global
descriptors whose entry contents cannot be assumed. A focused native regression
does improve with static alignment enabled and checks pre-entry odd-pointer
replacement, runtime rebinding through a call, and automatic initialization.

Baseline and per-slice CSVs, options, source hashes and diagnostic listings are
under `build/mir68k-descriptor-alignment/`. Measurements use r68k instruction
counts, not CPU cycle estimates.

Slice 1 validation: NIR snapshots unchanged; all 51 sweep fixtures passed;
compiler tests passed with the unrelated untracked sample scan excluded. The
exact sample-parser test also passed against tracked sources using the current
compiler library. Ten focused native tests passed across descriptor alignment,
pointer alignment, multidimensional arrays and index arithmetic.

## Optional guarded accesses

The longword-only guard is initially off. Developer runners accept
`--guarded-memory` / `--no-guarded-memory`, reject conflicting switches and record
the resolved setting. `--no-codegen-opt` disables guards regardless of argument
order; `--no-pointer-alignment` controls static proofs independently.

With guards and static proofs enabled, optimized shaped matrix1 executes 106651
instructions with 1494 executable bytes; shaped DCT executes 38991 instructions
with 7558 bytes. Frames and stack traffic are unchanged. The focused eight-value
loop executes 787 instructions with guards off, 651 on even pointers with guards
on, and 835 on odd pointers with guards on. Guarding trades additional code and
odd-path checks for cheaper aligned accesses.

Literal MC68000 qualification covers the actual address-test primitives and D0
preservation. Differential tests cover both NIR modes, register allocation and
forwarding settings, zero-trip loops, calls, full memory and byte traces, and
faults at every byte of even/odd longwords. Statically selected accesses and
excluded operations emit identical machine programs with guards off/on.

Slice 2 validation: all 16 MIR68K compiler unit tests and the affected native
qualification, guard, descriptor, pointer, fault, emission, forwarding,
allocation and measurement-parser tests passed. The real C runner rejected
conflicting guard switches before building. No NIR fixtures changed.
