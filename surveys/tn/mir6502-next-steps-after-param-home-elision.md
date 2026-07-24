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

Slice result:

Fresh artifacts were generated at revision `e70bf37`. The load files and
listings are reproducible as:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| MIR6502 XEX | 10,381 | `ff72b52f1b0c4c8373449513d76f840d34a9b8f218f7341012ed1e0be62fa5e0` |
| Modern/classic XEX | 10,445 | `3caefd677ab3d1489e39fcc0200126b442a15278b26a9cb5351434a1c8674f39` |
| Materialized MIR | 150,777 | `b8fa18e10e308369b5b69b9b218b306144adf0796b3dfcdce7b6cbf8a0c7b44b` |
| MIR6502 listing | 138,072 | `bdf45d8bf17e73df8d7f207783d5387fcb892aec8cee2b4bab5bb8326f842012` |
| Modern/classic listing | 136,233 | `668fb6ad3376a4449c6d79d2ae3050ca4deb415325002642cc244c9404750552` |

The main-segment reconstruction remains exact: MIR6502 has 9,498 instruction
bytes plus 871 data bytes, while modern/classic has 9,408 instruction bytes
plus 1,025 data bytes. MIR6502 therefore wins the whole load-file comparison
through 154 fewer data bytes while still emitting 90 more instruction bytes.
Across 103 matched routines the instruction-byte difference is only +41; the
MIR6502-only program-entry range accounts for the remaining 49 bytes.

The largest current positive routine deltas are:

| Routine | MIR6502 bytes | Classic bytes | Delta | Classification |
| --- | ---: | ---: | ---: | --- |
| `Handle` | 849 | 819 | +30 | call-argument scheduling, pointer-pair staging, CFG work |
| `PopUp` | 276 | 253 | +23 | call-result placement and dead cross-block loads |
| `Window` | 324 | 301 | +23 | word-result staging and a redundant X save/reload |
| `Draw` | 156 | 136 | +20 | indexed word values staged before A/X call arguments |
| `Strcpy` | 51 | 39 | +12 | pointer-pair and final ABI-destination planning |

The focused inspection found these concrete reusable families:

1. `Draw` has two indexed word loads whose low and high bytes are written to
   private zero-page homes and immediately reloaded into A and X. The classic
   backend loads the high byte into X and leaves the low byte in A.
2. `PopUp` loses known call results on the path to their consumers. This
   includes A/A0 copies, a `Range` result stored only to be loaded into Y for
   `FindItem`, and two dead A0/A1 loads before a comparison block.
3. `Window` contains an exact `STX spill12` / `LDX spill12` pair after a helper
   result. This is the clearest immediate X-value propagation candidate.
4. `Strcpy` and `Handle` use private pointer or scalar staging homes before
   copying the same values to their final pointer-pair or ABI locations.
5. The rest of `Handle` and `PopUp` is primarily call scheduling and
   control-flow shape, not general spill pressure.

The aggregate listing supports that classification. MIR6502 emits 1,194 LDA
and 851 STA instructions, or 48.2% of all instructions and 50.7% of code bytes.
Modern/classic emits 1,109 LDA and 801 STA instructions, or 45.1% and 48.5%.
MIR6502 therefore has 135 additional LDA/STA instructions despite its smaller
load file. Final MIR6502 storage telemetry reports 17 RAM spill labels and 43
private zero-page homes; RAM homes have 21 stores and 35 reloads, while the
zero-page homes have 46 stores and 48 reloads.

This makes X/Y machine-value telemetry worthwhile, but it should be treated as
one narrow input to later placement work rather than as a complete answer.
The highest-value follow-up families are indexed-word-to-A/X placement,
call-result-to-next-call placement, pointer-pair destination selection, and
dead register loads across CFG edges.

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
