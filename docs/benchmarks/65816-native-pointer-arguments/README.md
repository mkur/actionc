# Native 24-bit argument packing

Eligible captured three-byte arguments and numeric constants can now use two
A16 transfers at offsets zero and one. Both words remain within the complete
three-byte source/destination extents; only the private middle byte repeats.
The public stack ABI, exact source-memory captures, guards, padding and result
capture are unchanged. See the [emission contract](../../MIR65816_EMISSION_CONTRACT.md).

The selector compares bytewise, word-plus-byte-tail and overlapping-word forms,
including mode changes across the complete argument list. It retains the older
forms on ties. In particular, a word/byte tail can still be smaller before an
A8 indirect-target preparation. Calls with no padding omit the unused zero-load
and A8 setup. The guard join still requires the first payload width to be stated
explicitly; the size model accounts for that permission boundary.

This is a MIR65816-only change. Symbolic byte fixups and mixed-width extension
keep their existing paths. All operand, home and ABI-result checks finish before
call emission. Outgoing slots lie below the caller's private homes, so repeated
middle-byte writes cannot alter a source, another argument or padding. No
source-language read or write is widened, repeated or removed by packing.

## Frozen Exec size

The baseline is actionc `c04f5536`, after pointer-copy coalescing and native
bitwise operations. The unchanged frozen Exec `622b139-dirty` workload contains
631 routines, eight task slots, shell, console/windows and MyDOS.

| Measurement | Before | After | Saved |
|---|---:|---:|---:|
| Compiler code, guards included | 397,368 B | 389,743 B | **7,625 B (1.92%)** |
| Compiler code, guard ranges subtracted | 325,116 B | 317,491 B | **7,625 B** |
| Compiler initialized data | 951 B | 951 B | 0 B |

Of 2,045 call sites, **1,168 shrink**. Call spans save 7,608 bytes; shorter local
branches in surrounding comparison spans save another 17 bytes. **406 routines
shrink and none grow.** All 120 frozen input hashes match.

All routine contracts, temporary homes, frame extents, stack peaks and zero-fill
match the baseline. All 2,676 guards retain their sizes and checked amounts,
totalling 72,252 bytes. Added stack, DP and reserved bank-zero capacity: **0 B**.
This is a compiler-only size build; packaging, assembly and hosted qualification
were not rerun. Guard subtraction is not a separately compiled release image.

[Summary and hashes](exec-summary.json), [routine sizes](exec-routines.csv), and
[changed operation spans](exec-spans.csv) retain the measurements. The latter
contains only spans whose lengths changed; the audit also checks every unchanged
MIR operation and verifies no call span grows.

## ExecList

The same twelve list routines shrink **3,017 → 2,944 bytes**, including unchanged
540-byte guards, or **2,477 → 2,404 bytes excluding guards**. Raw output saves
72 bytes; optimized output saves 73, including one additional relaxed branch.

| Changed routine | Previous non-guard bytes | New non-guard bytes |
|---|---:|---:|
| NewMinList | 52 | 44 |
| IsMinListEmpty | 69 | 61 |
| Insert | 477 | 461 |
| RemHead | 158 | 150 |
| RemTail | 164 | 156 |
| Enqueue | 387 | 362 |

For example, an unpadded captured pointer now packs as:

```asm
REP #$20
LDA $09,S
STA $01,S
LDA $0A,S
STA $02,S
```

This ten-byte fragment replaces eighteen bytes of A8 setup, an unused zero load,
three byte transfers and restoration to A16. The second word covers the middle
and bank bytes, and does not touch a fourth byte.

The independent list oracle passes all **270 paired-mask records** in each host
profile: 135 vectors in raw and optimized modes, each with both incoming I
states. Debug/release results are identical, as are actual LF/CRLF builds.
Optimized per-vector cycle changes range from −19 to zero; raw from −17 to zero.
There are no measured cycle regressions. For example, optimized Enqueue vector 0
falls from 1,578 to 1,560 cycles, with guards included in both measurements.

Private stack reads/writes increase by at most three bytes per vector from the
repeated middle bytes. DP traffic, stack peaks and guard costs are unchanged.
See [before/after sizes](list-sizes.csv), [measurements](lists/measurements.csv),
[deltas](lists/delta.csv), [new assembly](lists/actionc-optimized.lst), and
[list provenance](lists/provenance.json).

## Focused validation

- Eight call-selector/padding unit tests pass. The encoding oracle exhaustively
  enumerates four-argument width sequences, constants/captured homes, both next
  widths and all three initial mode-permission states. Separate checks cover
  byte 255, the end of DP scratch, invalid homes, authoritative mutable parameter
  storage, symbolic/extension fallback and preflight without partial emission.
- Forty-three ABI, emission, o65 and emission-snapshot integration tests pass.
  The reviewed snapshot is unchanged; no NIR contract or printer changes occur.
- Nineteen focused debug runtime tests cover independent ca65 callees, direct
  and indirect calls, all result widths, mutable parameters, aliases/nested calls,
  volatile captures, bank crossings, guard-fault ordering, padding, canaries,
  replay/state proofs and two relocation placements. Six call-copy/padding tests
  also pass in release, alongside the list oracle.
- The focused native-call preemption test passes in both compiler modes. It
  checks every reached enabled task/PC/status boundary for ADDRESS and LONGCARD
  direct/indirect calls under IRQ and NMI, plus seeded schedules.

Runtime traces require one write per padding/non-middle payload byte. Only an
admitted three-byte argument's middle byte may repeat, at most twice, and every
write must already contain its final expected byte. The direct pointer probes
specifically require the new `[1, 2, 1]` write pattern. External captures retain
exact three-byte reads, including the bank-crossing and volatile cases.

Validation exposed a pre-existing stale ca65 reference in `state_tracking`:
the prior bitwise slice added stack AND/EOR and immediate ORA to the generated
probe without adding them to its independent assembly. The three missing
reference instructions are corrected in separate commit `f5905600`; the full
nine-test state-tracking target passes. No compiler behavior changes in that fix.

[Validation provenance](validation.json) records the input hashes, logs and
focused run manifests. Final backend and hosted Exec qualification remain off
at the user's request.

The main commands are:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --lib mir65816::emit::select::call
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --test mir65816_abi --test mir65816_emission \
  --test mir65816_o65 --test mir65816_state_boundary
python3 tools/native65816-runtime-tests/qualify.py \
  --test call_copies --test call_padding --test interop --test indirect --test state_tracking
python3 tools/native65816-runtime-tests/qualify.py \
  --test preemption native_call_arguments_and_ax_results_restore_at_every_reached_task_boundary
python3 tools/native65816-runtime-tests/qualify.py --release \
  --test call_copies --test call_padding
```

List reproduction uses `tools/compare65816/execlists.py` with the current CLI,
filters the manifest to Action artifacts, and runs the ignored `code_quality`
target with `A816_COMPARISON_MANIFEST` and `A816_COMPARISON_RESULTS` set, in both
host profiles. `report_pointer_micro.py` verifies and archives the paired results.
