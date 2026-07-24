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

Slice result:

The post-home machine-value state now carries A, X, and Y facts through the
same routine CFG. X/Y facts support constants, exact direct-memory values,
register moves, must-agree joins, loop convergence, exact writes, conservative
clobbers, and memory invalidation from the shared MIR effect classifier. A
direct store from a register also establishes the useful equality between that
register and the destination memory. Fixed zero-page value facts can now
capture stores from X and Y as well as A.

The post-home rewrite context exposes a typed `register_value_at` proof query.
No rewrite consumes the new X/Y facts in this slice. Final-program telemetry
only reports a candidate when an X/Y load would reproduce the exact incoming
machine value.

Fresh TN telemetry reports three candidates:

| Routine | Register | Site | Incoming and loaded value |
| --- | --- | --- | --- |
| `Window` | X | `b0`, op 35 | `spill12+0` |
| `Convert` | Y | `b3`, op 1 | global 5, offset 0 |
| `Convert` | Y | `b7`, op 1 | global 5, offset 0 |

The `Window` result confirms the exact `STX spill12` / `LDX spill12` case found
by the manual audit. The two `Convert` results demonstrate that the same
analysis also finds facts entering blocks through CFG edges.

This slice is code-generation neutral. The fresh TN XEX is still 10,381 bytes
with SHA-256
`ff72b52f1b0c4c8373449513d76f840d34a9b8f218f7341012ed1e0be62fa5e0`,
identical to the slice-2 artifact. Focused analysis and context tests pass, as
does the complete test suite (1,755 library tests plus integration and fixture
tests).

The candidates are not removal proofs by themselves: LDX and LDY also update
Z/N. A later rewrite must combine the value proof with shared flag liveness
before deleting either instruction.

### Slice 4: remove flag-safe redundant X/Y reloads

Consume the routine-wide X/Y value facts through the shared post-home rewrite
context. Remove an X/Y load only when:

- its incoming register value equals the value it would load;
- the read is not an absolute-memory access;
- and all flags written by the load are dead after the operation.

Keep candidate, accepted, and flag-live-blocked telemetry separate.

Slice result:

All three TN candidates pass the flag-liveness proof and are removed:

| Routine | Result |
| --- | --- |
| `Window` | one X reload removed |
| `Convert` | two Y reloads removed |

TN falls from 10,381 to 10,374 bytes. The fresh XEX SHA-256 is
`50d38a020bc6260878d779d583f64bc1f03c0ff2b0699043151d894e17a8aeea`.
The listing contains 4,247 instruction rows, 9,494 code bytes, and 868 data
bytes. Focused tests cover both a removable store/reload and a reload whose
Z/N result feeds a branch and must remain.

### Slice 5: place indexed word values directly into A/X

Target the two `Draw` sequences where an indexed word read is staged through
private homes immediately before a call consumes the value in A/X. Generalize
the existing indexed-word consumer selection so the low and high lanes can be
placed directly in their final register pair when:

- the word value has one coupled consumer;
- the call argument home is exactly A/X;
- the indexed address and Y value remain valid for both byte reads;
- and shared liveness proves that the alternate load order is unobservable.

Do not add a `Draw` special case. Cover the reusable indexed-word-to-register
pair shape and retain the staged fallback.

Slice result:

The call-expression materializer now recognizes the canonical Action ABI shape
where an indexed word argument occupies A/X, a byte argument occupies Y, and
later byte arguments occupy their fixed memory homes. It prepares memory
arguments first, reads the indexed word directly into A/X, and loads Y last.
The emitted call summarizes already-prepared memory arguments as memory cells,
preventing a later materialization round from storing them again and
clobbering A.

One of the two audited `Draw` sites now uses the direct path. Its private
two-byte home is gone and the final sequence uses the existing fixed A0 cell
only to preserve the low byte while the high byte is transferred to X. The
second site retains the staged fallback because its index is a wrapping byte
subtraction (`i - 1`). Treating that subtraction as a negative address
displacement would differ when `i` wraps through zero, so this slice does not
make an unstated range assumption.

TN falls from 10,374 to 10,368 bytes. The fresh XEX SHA-256 is
`3c6820d322bc715b450a0a96b43c57e4863228603d41a109cbc5b07471d0b6a4`.
The listing contains 4,244 instruction rows, 9,494 code bytes, and 862 data
bytes. Focused tests cover the direct A/X path, final Y scheduling, fixed-memory
argument preparation, and the already-materialized call summary.

### Slice 6: improve call and pointer destination placement

Use consumer-selected destinations for the remaining audited families:

- call results forwarded to a following call's A/X/Y argument homes;
- known A/A0 result aliases consumed without a shadow reload;
- pointer bytes placed directly in the selected fixed pointer pair;
- and dead register loads on paths entering comparison blocks.

Implement only families with explicit value, effect, dominance, and liveness
proofs. Rebuild the final TN listing after each accepted family and record the
new routine deltas and load/store share.

## Follow-on work

After these slices, use the fresh audit to decide whether broader
consumer-driven multi-use placement across A, X, Y, the private pointer pairs,
and ABI locations is justified.

Do not begin with general register allocation or cross-routine home pooling.
Pooling has a small backing-only ceiling and does not remove the expensive
access instructions. Revisit high-pressure `SetWin` only when a concrete,
reusable producer/consumer family justifies it.
