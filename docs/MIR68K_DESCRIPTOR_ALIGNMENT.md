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

## Default selection and complete workload measurements

The selected default enables static proofs and longword guards. Unknown word
accesses retain the bytewise path. Compared with `38471f2`, optimized shaped
matrix1 uses 26.0% fewer instructions and shaped DCT uses 9.9% fewer. Code grows
by 162 and 900 bytes respectively. Every default workload has unchanged frame
size and stack traffic; no measured instruction count regresses.

| Workload | Instructions, baseline → selected | Code bytes, baseline → selected | Largest frame | Stack read / write bytes |
| --- | ---: | ---: | ---: | ---: |
| insertsort | 5437 → 5349 | 1604 → 1622 | 136 | 2125 / 1843 |
| matrix1 | 71806 → 69106 | 990 → 1044 | 68 | 24373 / 13969 |
| matrix1-multidimensional | 144051 → 106651 | 1332 → 1494 | 88 | 53741 / 31777 |
| binarysearch | 8302 → 8302 | 754 → 754 | 68 | 885 / 737 |
| sha | 14924 → 14924 | 3196 → 3196 | 452 | 8453 / 7090 |
| jfdctint | 38767 → 35567 | 5768 → 6632 | 742 | 13560 / 10432 |
| jfdctint-multidimensional | 43279 → 38991 | 6658 → 7558 | 886 | 15398 / 12982 |
| adpcm_dec | 15153 → 13509 | 7842 → 8382 | 692 | 8300 / 6340 |
| adpcm_enc | 1892709 → 1886577 | 10034 → 10682 | 724 | 421070 / 264561 |

[The complete CSV](mir68k-descriptor-alignment.csv) keeps raw and optimized NIR
separate for all five configurations: baseline (`38471f2`), static facts only,
guards only, both enabled (`guarded`), and conservative target materialization.
The latter retains the default NativeLoops NIR policy; it changes target options
only. Reproduce the current configurations with the `code_quality` example and
respectively `--no-guarded-memory`, `--guarded-memory --no-pointer-alignment`,
`--guarded-memory`, or `--no-codegen-opt`. Per-run hashes/options remain in the
matching build directories. All reference fixtures are unchanged.

Runtime coverage includes negative INT coordinates with 80 KB row strides,
odd rebasing, BYTE wrap, LONGCARD stores with rebinding during the RHS call,
recursive LONGCARD local arrays, wide volatile traces and terminal partial
faults. Native source records pad wide fields; a verified MIR case exercises
five-byte record strides with a one-byte field offset, including bare images at
two origins and HUNK execution at two independent sets of segment bases.

Slice 3 validation: the complete native VM suite and all 16 MIR68K compiler
unit tests passed. Both VM workspaces passed the full 252-case matrix1 and
181-case DCT corpora, including flat/shaped implementations, native raw/optimized
NIR and the existing 6502 compiler/runtime combinations. Complete states and DCT
row-pass snapshots were checked; the 6502 targets also checked LF/CRLF handling.
