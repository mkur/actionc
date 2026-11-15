# MIR6502 private-storage boundary qualification

The [implementation plan](../../MIR6502_PRIVATE_STORAGE_BOUNDARY_IMPLEMENTATION_PLAN.md)
is complete. Unknown assembly pointees no longer require complete callee read/write
footprints before private spill pruning. Explicit operands and declared effects,
source/ABI storage, historical exposure, preservation replay and final-cost guards
remain protected.

## Measured result

| ActionCart TN at $3000 | Correctness baseline | After | Change |
| --- | ---: | ---: | ---: |
| Normal optimized configuration | 9,862 | 9,870 | +8 bytes |
| Opt-in spill pruning | 9,862 | 9,870 | +8 bytes |

There is **no TN saving**. The selected program has no unused spill candidates.
The new explicit-observation inventory conservatively retains four additional
`STA $E3` instructions: one each in MoveMenuBar, Draw, Tag and TagAll.
The trace identifies unresolved machine observations as the blockers; it does
not prove that TN actually reads these temporary values. This is a remaining
reference-decoding limitation, not a reason to weaken explicit-alias protection.

| Routine | Before code bytes | After code bytes |
| --- | ---: | ---: |
| MoveMenuBar | 115 | 117 |
| Draw | 145 | 147 |
| Tag | 88 | 90 |
| TagAll | 87 | 89 |

All other routine code sizes are unchanged. Putchar remains 19 bytes,
FormatDirectoryRow 333 and SetWin 636. Source-parameter carrier checks were
not relaxed. Four more stores remain; no additional spill bytes were removed.

The independent execution controls permit **one-byte tail removal** and
**two-byte compaction** with an unknown assembly reader. They check all 256 byte
inputs, ActionCart/Standalone and origins $3000/$41C7, including result bytes and
surrounding memory sentinels.

## Evidence and scope

- [Qualification](qualification.json), [baseline measurements](baseline.jsonl)
  and [after measurements](after.jsonl).
- [All baseline routine ranges](baseline-routines.json) and
  [all after routine ranges](after-routines.json).
- MoveMenuBar [before](MoveMenuBar-baseline.asm) / [after](MoveMenuBar-after.asm);
  Draw [before](Draw-baseline.asm) / [after](Draw-after.asm);
  Tag [before](Tag-baseline.asm) / [after](Tag-after.asm);
  TagAll [before](TagAll-baseline.asm) / [after](TagAll-after.asm).
- [Baseline artifact hashes](baseline-receipt.json),
  [after artifact hashes](after-receipt.json) and [source hashes](source-hashes.json).

Both builds used a clean linked checkout, excluding the paused directional-write
edits and unrelated work. Baseline compiler: `336ff2a6`; implementation:
`499ce201`. Large images, full output dumps and traces remain in
`build/private-storage-boundary/{baseline,after}/`.

Validation: 16 ownership tests and 28 pruning tests passed. The pruning group
passed 27 cases initially; its historical-alias case then passed a focused
recheck after updating the expected earlier rejection reason. Two MADS-only
tests remained ignored. No full suite or other backend suite ran. No
newline-sensitive fixture handling changed.

The TN probe checks pruning off/on and compares the pruning-enabled traced
output with the untraced complete output. It measures code size and output
consistency; TN application execution was not part of this qualification.

## Reproduction

[probe.rs](probe.rs) is the existing late-spill-pruning probe narrowed to
ActionCart/$3000, with a larger diagnostic budget and extra artifact recording.
Run from the repository root with a library built from the intended clean revision:

```sh
mkdir -p build/private-storage-boundary
rustc --edition=2024 docs/benchmarks/6502-private-storage-boundary/probe.rs \
  --extern actionc=build/checked-arena/shared-target/debug/libactionc.rlib \
  -L dependency=build/checked-arena/shared-target/debug/deps \
  -o build/private-storage-boundary/probe
build/private-storage-boundary/probe build/private-storage-boundary/recheck --tn-only
```

The recorded configuration files contain the common base configuration plus the
explicit pruning off/on override. The checks used the shared Cargo target with
debug information and incremental compilation disabled, dev/test optimization
level 1, and two build jobs.
