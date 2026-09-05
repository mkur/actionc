# Classic indirect scalar-copy pointer overlap

Status: fixed, 2026-09-06.

The structurally faithful Oscar64 `structarraycopy.c` port exposed a classic
scalar-materialization bug, not a record-copy failure. Both Compatibility and
modern classic returned `$00FC` instead of zero from the original checksum,
with either runtime. The complete Point source and destination arrays were
correct. MIR6502 passed. This affected all 132 classic executions in the new
198-case port; the earlier 4,380 Oscar64 cases remained green.

## Cause

For `sum==-tcorners(i).y`, classic prepared the source record address in
`$AE/$AF`, then materialized the word operand into those same scratch bytes:

```asm
LDY #3
LDA ($AE),Y
STA $AF       ; destroys the pointer's high byte
DEY
LDA ($AE),Y   ; reads the low byte through the corrupted pointer
STA $AE
```

The original values have zero high bytes, so the second read came from zero
page. Ordinary subtraction and pointer-indexed record fields reproduced the
same defect. The source-level compound-operation typing was not the cause.

## Repair and ownership

The shared classic scalar slot-copy path uses the existing
`slot_overlaps_zero_page` identity check and the stack-staging approach already
used by prepared indexed word loads. When a fixed destination overlaps the
indirect source pointer, both source bytes are read before either destructive
store. Byte-to-word copies also capture their byte before writing the zero
high lane. The word-expression materializer routes this overlap case through
the shared copy path; non-overlapping paths are unchanged.

This retains the source access order, X/Y behavior and byte-widening semantics
of the selected copy path. The stack is balanced; no extra shared scratch is
introduced. Fixed absolute zero-page aliases are included, while relocatable
addresses are not mistaken for fixed aliases. No SemIR/NIR contract or MIR6502
selection changed, and there is no record- or benchmark-specific workaround.

## Regression coverage

- `src/codegen/tests/indirect_copy.rs` exercises both classic profiles,
  both scalar copy orders, four scratch pointer pairs, fixed absolute and
  zero-page destinations at displacements -2 through +2, byte/word sources,
  volatile/nonvolatile loads and offsets 0/2/127/254. It checks the result,
  X/Y, a stack sentinel, untouched zero page and source guards (2,560 runs).
- `tests/fixtures/classic_record_field_arithmetic.act` retains compound and
  ordinary subtraction through direct record arrays and record pointers.
  The public compiler matrix checks nine indexes through 257 and seven word
  patterns under all three modes and both runtimes (378 runs).
- The Oscar64 port keeps its original expression and independent complete
  memory/counter oracle; its fix is committed separately from the port.

Reproduce the focused regression checks from the repository root:

```sh
cargo test --lib indirect_copy
```
