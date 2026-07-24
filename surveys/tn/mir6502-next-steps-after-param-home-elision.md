# MIR6502 next steps after parameter-home elision

Date: 2026-07-24.

Starting revision: `afb44d0` (`mir6502: remove reassigned write-only parameter
homes`).

Scope: `samples/tn/modern/TN.ACT`, modern profile, comparing MIR6502 with the
modern/classic backend.

## Baseline

The two parameter-home slices reduced the MIR6502 TN load file from 10,455 to
10,381 bytes. Modern/classic remains 10,445 bytes, so MIR6502 is currently 64
bytes smaller.

This establishes size parity but does not close the code-quality gap. The last
valid audit still showed more MIR6502 load/store traffic, and several routines
remained larger despite the whole XEX being smaller.

## Execution plan

### Slice 1: repair and lock listing measurement

`actionc-listing-quality` currently assumes that the raw byte field is eight
characters wide. That works for instructions but truncates `.BYTE` rows with
four to eight values. The remaining hex bytes are then misclassified as
mnemonics.

Implement variable-width parsing up to the first two-space separator before
the listing text. Add a regression containing an eight-byte `.BYTE` row inside
a procedure and assert that:

- all eight bytes are data;
- no pseudo-instructions are invented;
- instruction and data byte totals reconstruct the represented address span.

Exit criterion: the tool's code-byte plus data-byte total exactly matches the
main load segment for fresh MIR6502 and classic TN listings.

Slice result:

| Backend | Instruction rows | Code bytes | Data bytes | Reconstructed main segment |
| --- | ---: | ---: | ---: | ---: |
| MIR6502 | 4,247 | 9,498 | 871 | 10,369 |
| Modern/classic | 4,236 | 9,408 | 1,025 | 10,433 |

Both reconstructed totals exactly match their emitted main segments. The new
eight-byte `.BYTE` regression is counted as eight data bytes and zero
instructions.

### Slice 2: regenerate and normalize the concentrated TN audit

Regenerate listing, map, materialized MIR, quality report, and load file for
both backends at the post-elision revision. Compare matched procedure ranges by
instruction-form bytes and inspect:

- `Window`;
- `Draw`;
- `Strcpy`;
- `Handle`;
- `PopUp`.

Classify each positive difference as:

- redundant register reload;
- temporary-home traffic;
- addressing selection;
- call argument/result scaffolding;
- control-flow/layout work;
- or genuinely necessary backend work.

`PopUp` must remain separate from spill-driven work because the preceding audit
found no final temporary home there.

Exit criterion: record exact artifacts, main-segment reconciliation, updated
routine deltas, and a ranked set of reusable optimization families.

### Slice 3: add telemetry-only routine-wide X/Y value propagation

The shared post-home machine-value analysis currently propagates accumulator
facts across CFG edges. Extend the same target-specific framework to carry X
and Y facts:

- constants and exact direct-memory values;
- agreement at CFG joins;
- exact register writes and clobbers;
- memory invalidation through the shared effect classifier;
- conservative call, helper, machine-block, and unknown-effect handling.

Expose typed register-value queries through the existing analysis snapshot and
rewrite context. Do not remove or rewrite instructions in this slice. Instead,
report candidate X/Y reloads whose loaded value already agrees with the
routine-wide incoming register fact.

Exit criterion: focused diamond, loop, clobber, and memory-invalidation tests
pass; TN telemetry names the routines and sites that could benefit from a later
rewrite.

## Follow-on work

If slice 3 finds useful sites, enable redundant X/Y reload removal as a narrow
follow-up. Then add consumer-driven multi-use placement across A, X, Y, the two
private pointer pairs, and ABI result/argument locations.

Do not begin with general register allocation or cross-routine home pooling.
Pooling has a small backing-only ceiling and does not remove the expensive
access instructions. Revisit high-pressure `SetWin` only when a concrete,
reusable producer/consumer family justifies it.
