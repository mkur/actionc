# Native 32-bit Add/Sub

Implemented on 2026-09-24 against compiler baseline `32af3e2b`. LONGCARD and
LONGINT addition/subtraction now use two A16 operations for eligible stack
values and numeric constants. This slice changes MIR65816 instruction selection;
NIR, allocation, public ABI and guard policy are unchanged.

## Selection

The low-word ADC/SBC establishes carry/borrow with CLC/SEC. Its result is stored
directly, then the high-word operation consumes the carry/borrow. No DP scratch,
X/Y temporary or helper is needed. Both signed and unsigned results wrap modulo
2^32. Narrow numeric constants zero-extend; signed widening of captured values
continues to use explicit casts.

Selection checks complete source/result extents and authoritative parameter
homes before emission. Identity and disjoint source/destination homes are
eligible. Partial overlaps and unsupported operands retain the existing
fallback; malformed homes remain errors. External and volatile captures retain
their original access count/order. See the
[emission contract](../../MIR65816_EMISSION_CONTRACT.md#scalar-instruction-selection).

For two captured stack operands, the arithmetic sequence is **43 -> 13 bytes**
and **77 -> 32 VM cycles**. Tests compare the actual selected bytes with ca65
output, execute the previous bytewise sequence independently, and check exact
private reads/writes. This example begins in A16; its native result remains in
A16, while the old sequence ended in A8. Whole-routine measurements include
subsequent width changes.

## Exec measurement

The optimized workload is Exec `622b139-dirty`, eight task slots, console/windows,
shell and MyDOS, from `build/play-622b139-actionc-32af3e2b/native`. The original
packaged image SHA-256 is
`200572a14109579065f3bca897c5e7be489a1f893d755408c05577464c6f41d0`.

The Exec working tree changed during this work. The measurement was repeated
against a frozen compiler input set: the original generated sources and layout,
plus the consumed source files. Two changed includes (`task-dos.inc` and
`task-sio.inc`) were restored in the snapshot from the recorded Exec revision,
with hashes matching the original build. Only absolute INCLUDE paths were
rewritten to the snapshot root. That build produces a compiler image identical
to the earlier candidate measurement, confirming the same 2,965-byte saving.
The user's Exec working tree was not modified.

| Measurement | Before | After | Saved |
| --- | ---: | ---: | ---: |
| Compiler-generated routine bytes | 423,023 | 420,058 | **2,965** |
| Same code with measured compiler guards subtracted | 350,771 | 347,806 | **2,965** |
| 104 long Add/Sub operation spans | 4,422 | 1,498 | 2,924 |

Another 41 bytes disappear through neighboring mode/layout changes. Of 631
routines, 59 shrink and none grows. Every routine contract except its address
and size is unchanged, as are all typed MIR operations. All 2,676 compiler
guards remain, occupying 72,252 bytes. Compiler-initialized data remains 951
bytes. The [changed-routine table](changed-routines.csv) and
[measurement record](measurement.json) retain the accounting and artifact hashes.

This is a compiler-only measurement build with guards enabled. The platform
assembly, packager-added data and XEX were not rebuilt, and full hosted Exec
qualification was not run. The initial public release cap remains 256 KiB of
loaded code plus initialized data **with guards disabled**; its actual artifact
must be measured separately. Subtracting guard bytes here does not claim a
qualified build with guards disabled.

## Validation

- 240 native 65816 library tests passed; one existing test remains ignored.
- 48 integration tests passed across `mir65816_abi`, `mir65816_arithmetic`,
  `mir65816_emission` and `mir65816_o65`.
- Five new long-arithmetic VM tests passed in both debug and release hosts,
  covering raw/optimized compilation and both initial I states. Cases include
  signed extremes, low-word carry/borrow, full wraparound, constant operand
  order, mixed-width casts, mutable parameters, loops, external aliases,
  volatile/bank-crossing accesses, complete call clobbers, independent ca65
  encodings, exact private traffic and two o65 placements.
- The focused preemption test passed in the release host: IRQ and NMI restore
  full state at 300 distinct reached task/PC/flag combinations in each compiler
  mode, plus two seeded schedules. Both carry and borrow paths are exercised.
- Another 21 release-host VM tests passed across arithmetic, word arithmetic,
  long equality, wide returns, call copies, stack allocation and constant shifts.
- LF/CRLF source parsing and preemption-fixture instrumentation are exercised
  through their actual paths. No shared frontend/IR contract or unrelated
  backend suite was changed.

Use the qualified wrapper for focused reproduction:

```sh
python3 tools/native65816-runtime-tests/qualify.py --test long_arithmetic
python3 tools/native65816-runtime-tests/qualify.py --release --test long_arithmetic
python3 tools/native65816-runtime-tests/qualify.py --release --test preemption native_long_add_sub
```

The compact measurement record identifies the pinned VM, timing patch, input
inventory digests and local qualification manifests. Full Exec qualification
remains reserved for the final optimization commit, as requested.
