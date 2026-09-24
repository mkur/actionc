# Exec816 list primitives: Action, vbcc and Calypsi

Compiler: `87feebf1`, after native 24/32-bit returns. The frozen
[`execlists.act`](../../../tools/native65816-runtime-tests/tests/fixtures/execlists/execlists.act)
is byte-identical to the live Exec816 module and the `c3500c8` measurement copy.
The [equivalent C](../../../tools/native65816-runtime-tests/tests/fixtures/execlists/execlists.c)
retains all twelve operations, sentinel behavior, signed-priority bias, FIFO
ties, null-name skipping and resumable name searches. Layout checks require
three-byte stored pointers, exact record sizes 6/9/11/11 and matching field offsets.
No compiler optimization or live Exec source was changed for this analysis.
The [implementation plan](../../MIR65816_POINTER_MICRO_OPTIMIZATIONS_PLAN.md)
orders the proposed improvements as separate slices, including captured-pointer
null reduction.

vbcc is the installed V0.9i pre / 65816 generator V0.2, using `-mhuge -ptr24
-near-threshold=0 -no-near-const`, with `-O=0` or `-O=1023`. vasm performs branch
relaxation. Both builds retain separate public functions; no hand-written
assembly replaces either implementation. The ABI difference is retained: Action
passes all arguments on its guarded stack; vbcc passes its first pointer in A/X
and uses DP pseudo-registers. Other arguments and call conventions also differ.

Calypsi 5.18 is installed at `/usr/local/lib/calypsi-65816-5.18`, with commands
linked into `/usr/local/bin`. It uses `-O0` or `-O2 --space`, large code and huge
data models. Record fields use `__far24` to store exactly three bytes; parameters
and working pointers use `__huge` to retain bank carry. The latter occupy four
bytes. Explicit casts narrow stores back to the packed field type. Using
`__far24` throughout would give cheaper pointer arithmetic without the required
bank-crossing behavior. No hand-written implementation or runtime library is used.

Calypsi's normal ABI passes its first two pointer arguments in `_Dp[0..3]` and
`_Dp[4..7]`, the third on the stack, and returns pointers in A/X. Huge pointer
arguments have a zero upper byte. `_Dp[8..15]` is preserved. The harness checks
these conventions independently of Action/vbcc. The installed compiler guide
sections 5.3 and 20.3 describe the address spaces and ABI; its hash is recorded.
Calypsi rejects the three-byte `sizeof` typedef assertions in its front end even
though target lowering emits three-byte `__far24` fields. Its equivalent checks
therefore inspect thirteen emitted layout constants in the linked binary.
Their 26 data bytes are excluded from executable sizes.

This compares equivalent operations with each compiler's own ABI. Calypsi's
size optimization extracts shared instruction sequences and shares some tails;
those helpers are counted separately below. Individual routine sizes alone are
not standalone implementation costs.

| Routine | Action optimized | Guards | Action excluding guards | vbcc optimized | Calypsi optimized |
|---|---:|---:|---:|---:|---:|
| NewList | 243 | 27 | 216 | 75 | 52 |
| NewMinList | 122 | 54 | 68 | 10 | 5 |
| IsListEmpty | 179 | 27 | 152 | 48 | 34 |
| IsMinListEmpty | 139 | 54 | 85 | 10 | 5 |
| AddHead | 285 | 27 | 258 | 95* | 63 |
| AddTail | 304 | 27 | 277 | 108 | 79 |
| Insert | 648 | 81 | 567 | 182* | 180 |
| Remove | 109 | 27 | 82 | 77* | 47 |
| RemHead | 226 | 54 | 172 | 43* | 51 |
| RemTail | 226 | 54 | 172 | 73* | 83 |
| Enqueue | 503 | 81 | 422 | 161* | 126 |
| FindName | 573 | 27 | 546 | 156* | 195 |
| Shared helpers | 0 | 0 | 0 | 0 | 169 |
| **Total** | **3,557** | **540** | **3,017** | **1,038** | **1,089** |

All sizes are emitted bytes, including every shared helper. No library code is
omitted. Entry and call checks are included in Action's guard column. Static
sizes count each function once; dynamic metrics include its executed callees.
The 29-byte Action driver is excluded. Against vbcc the full ratio is 3.43x;
excluding guards it is 2.91x. Removing guards alone would leave most of the gap.
Against Calypsi, the ratios are 3.27x with
guards and 2.77x excluding them. Raw totals are 3,635 Action / 1,439 vbcc /
1,360 Calypsi bytes.
Action optimization saves only 78 bytes here; Enqueue and FindName each grow by
eight bytes as their three-byte loop values acquire edge-copy traffic.

An asterisk marks an optimized vbcc routine with a failing execution vector,
including failures inherited from its callees. Smaller incorrect code is not a
valid optimization target. See [sizes](sizes.csv),
[Action optimized listing](actionc-optimized.lst),
[vbcc optimized listing](vbcc-optimized.lst), and
[Calypsi optimized listing](calypsi-optimized.lst); raw listings are
[Action](actionc-raw.lst), [vbcc](vbcc-raw.lst), and [Calypsi](calypsi-raw.lst).

## Correctness and measurement scope

The [independent oracle](../../../tools/compare65816/execlists_vectors.py)
constructs complete before/after memory states for 135 inputs. It covers empty,
one/multiple-node lists, all Insert predecessor forms, removals at each position,
signed priority boundaries and equal-priority FIFO insertion, matching/missing
and unnamed nodes, search resumption, odd far addresses, records/names crossing
banks, and bank-zero pointers with zero low bytes. Distinct pointer low bytes
expose accidental neighboring-field writes. Every input object and its metadata
is checked; guards detect writes outside objects. Result lanes, stack, DP,
widths and incoming interrupt-mask preservation are checked.

All **270 Action and 270 Calypsi records pass** (135 per compiler optimization
mode). Both host builds agree on all **810 paired-mask records**, or 1,620
executions per host. The previous 540 Action/vbcc measurements are unchanged,
and both compilers reproduce exactly their previous emitted binaries.
vbcc fails **14 raw and 54 optimized records**. Both comparison tests therefore
exit 101 (the Python wrapper exits 1), after saving all records. These are actual
compiler failures, not expected-success exemptions. No IRQ/NMI injection or
hosted Exec qualification is claimed by this comparison. All three compilers
were rebuilt with LF and CRLF inputs; resulting images/binaries match.

Two distinct vbcc failures matter when reading its assembly:

- In AddHead and Remove, some A16 stores at pointer offset two overwrite the
  next field. For example, optimized AddHead writes at `$0100C5` and changes
  `item.ln_Pred` at `$560556` from `$01` to `$00`. The instruction copies four
  bytes into a three-byte successor field. Other routines inherit this through
  calls. This extends the previously observed optimized `unlink` defect.
- Some pointer-zero tests load the low word, switch to A8, OR the bank byte,
  then branch on Z. The middle byte remains in hidden B and does not contribute
  to the branch. A pointer such as `$008300` is consequently treated as null.
  Bank-zero vectors expose this in raw and optimized Insert, RemHead, RemTail,
  Enqueue and FindName. The short zero-test sequence is not a correct model for
  Action's full 24-bit tests.

[Failures](failures.json) retain exact addresses and last writer PCs (or `none`
when an expected write never occurred). [Per-vector measurements](measurements.csv)
retain correctness alongside cycles, stack/DP traffic, frame peaks and guard
costs. Representative optimized timings, including all executed helper calls:

| Operation / vector | Action cycles | Guard cycles | vbcc cycles | Calypsi cycles |
|---|---:|---:|---:|---:|
| NewList / 0 | 439 | 25 | 141 | 244 |
| IsListEmpty / 0 | 296 | 25 | 85 | 81 |
| AddHead / 1 | 577 | 25 | 199* | 328 |
| Remove / 0 | 179 | 25 | 163* | 204 |
| Enqueue / 0 | 1,741 | 75 | 626* | 944 |
| FindName / 1, first five-character match | 2,795 | 25 | 492 | 971 |

The starred timings have incorrect outputs. Calypsi is 2.88x faster on this
FindName input; Action already beats it on Remove, even including the guard.
Code sharing saves space but introduces calls and returns. This is a size-mode
comparison, not a speed-mode comparison.

Calypsi also uses wider reads followed by `AND #$00FF` for some three-byte
pointer loads and byte operations. FindName vector 1 reads four padding bytes
outside logical objects (vbcc reads one; Action reads none). Readable padding
is mapped by this probe; writes to it are forbidden. Calypsi's guide explicitly
allows wider non-volatile reads. Those load sequences must not be copied into
Action's exact-width or volatile accesses without proving the extra byte is
owned and readable.

## Concrete micro opportunities

1. **Use `[dp]` for zero-offset indirect accesses.** There are 23 adjacent
   `LDY #0; LDA/STA [dp],Y` pairs. The unindexed opcode has the same address and
   access width, saving three bytes per eligible pair: **69 potential bytes**
   in this module, before relaxation. Selection must account for the eliminated
   Y assignment and N/Z effects. This is the smallest next slice.
2. **Native-width same-width casts.** Thirteen three-byte pointer/integer casts
   occupy **162 bytes**, copying each byte through A8 even though the bits do
   not change. Their private homes are disjoint in this module. Checked native
   private copies can use two overlapping words entirely inside each three-byte
   extent; budget mode changes explicitly. Keep semantic casts in MIR and improve
   target selection first. Eliminating the temporary itself is a separate step.
3. **Compact captured-pointer AddressOf.** Three `@pointer.field` operations with
   offset zero cost 26 bytes each; four with offset three cost 45 each: **258
   bytes total**. They stage the base in DP, do bytewise address arithmetic,
   copy back into a temp, and usually feed another same-width cast. Seven of
   these sites begin with an empty `SEP #$20; REP #$20` excursion. For checked,
   disjoint stack homes and A16 entry, direct copy/add candidates are 8 bytes
   for +0 and 16 for +3, including bank carry. That suggests 170 bytes at these
   sites before surrounding mode/layout effects; this is an encoding forecast,
   not an implemented saving. Symbolic, volatile and overlap cases need their
   own admission/fallback rules. Empty mode pairs overlap this forecast.
4. **Three-byte edge copies, then pointer increments.** FindName spends **172
   bytes** on loop-edge transfers and **112** on four casts plus two adds. That
   is nearly half the routine. Edge copies currently stage every byte even for
   simple disjoint assignments. Enqueue spends another 63 bytes on edges.
   Extend the existing checked copy strategy to three-byte homes in a bounded
   slice, preserving parallel-copy cycles. Native low-word increment plus bank
   carry is a separate candidate; no fourth byte may be read or written.

The [operation inventory](operation-inventory.json) records exact MIR spans.
All twelve standalone Action routines match frozen Exec machine bytes after
relocation, with identical frame/home contracts. The
[local candidate inventory](local-candidates.json) records instruction offsets.
Counts describe current cost; forecasts above are not additive across slices.

Calypsi confirms the value of the first three local changes: it uses `[dp]`
directly, moves low words at native width, and propagates carries for huge
pointer additions. Its zero tests examine both the low word and bank byte;
they avoid vbcc's hidden-B mistake. Its shared helpers are a later, separate
space optimization with call overhead and additional ABI proof obligations.

`Remove` is evidence that the current backend can already get close: it uses
pointer DP residency, no fixed frame, and is 82 bytes excluding its guard versus
vbcc's incorrect 77-byte version. Broad register allocation is not the first
step. The two wrappers are also conspicuous: vbcc tail-jumps in ten bytes,
and Calypsi uses five-byte JSL/RTL wrappers,
where Action captures arguments, copies casts, performs a guarded call and
tears down a frame. Tail-call selection has additional ABI/stack-proof work and
belongs after the simpler local changes.

## Handwritten reference sequences

The [handwritten Remove](hand-remove.s) uses A16/X16 and the current stack ABI.
It captures the pointer with two overlapping word reads, keeps the predecessor
in DP, and reuses the dead item home for the successor. A short-lived X value
preserves the successor's upper two bytes during that replacement. Its body is
50 bytes and 121 cycles, versus Action's 82 bytes and 154 cycles, excluding the
entry guard in both cases. Scratch use falls from nine to six bytes. The current
27-byte guard would bring its total to 77 bytes; this is an accounting forecast,
not a newly implemented compiler path.

The probe passes all nine Remove vectors plus six cyclic-list alias cases,
including one-node/self links, a shared predecessor/successor and node fields
crossing banks. Both incoming interrupt-mask states pass, with no padding reads.
Its test adapter takes 36 cycles: the measured 157-cycle entry-to-return total
therefore leaves 121 cycles in Remove itself. Timing uses page-aligned DP.

The [handwritten null probes](hand-null.s) test captured DP/stack pointers and
an indirectly addressed three-byte field. A captured pointer at `4,S` needs:

```asm
; A16
LDA 4,S
ORA 5,S
BEQ is_null
```

The six-byte core includes the short branch. It combines words at offsets zero
and one, so all three bytes contribute to Z without a fourth-byte read or mode
change. This saves size against the twelve-byte short-circuit sequence, but
reads both words even when the low word is already nonzero. The indirect form
uses `LDY #1; LDA [dp],Y; ORA [dp]; BEQ` and occupies nine bytes. Each variant
passes 29 values (zero, all 24 individual bits and four combined patterns) with
both I states; indirect storage straddles a bank boundary.

These are handwritten ca65 sources, not Calypsi output. Their test manifests use
the comparison harness's Calypsi argument convention solely to drive the probes;
the stack adapter constructs the Action argument layout. Sources and
[measurement hashes/counts](hand-probes.json) are archived here. The direct
private-home null reduction fits existing memory guarantees. Overlapping
external loads/stores repeat the middle-byte access and require the eligibility
proof specified in the plan; they are not a general volatile/MMIO replacement.
The probes do not qualify asynchronous execution or hosted Exec integration.

## Reproduction

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/execlists.py

A816_COMPARISON_MANIFEST="$PWD/target/execlists-comparison/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/execlists-comparison/debug.json" \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
python3 tools/native65816-runtime-tests/qualify.py \
  --test code_quality -- --ignored --nocapture

# Run this even after the debug command reports incorrect vbcc results.
A816_COMPARISON_MANIFEST="$PWD/target/execlists-comparison/manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/execlists-comparison/release.json" \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
python3 tools/native65816-runtime-tests/qualify.py \
  --release --test code_quality -- --ignored --nocapture

python3 tools/compare65816/report_execlists.py
```

The report requires matching host results, all Action results correct, and
unchanged tool/source/artifact hashes; it deliberately retains incorrect C
results. [Provenance](provenance.json) records hashes and failure counts.
The comparison harness reads BYTE/24-bit results, supports Calypsi huge-pointer
argument placement and preserved registers, and reports missing writes without
losing the run. This manifest disables only the ancillary
control-flow inventory, whose current one-dispatch-per-terminator assumption
does not handle multiword pointer predicates. Emitted-byte equality, CPU
execution, memory/ABI checks and existing movement witnesses remain enabled.
The default observer was checked with unchanged `wide_shift` and `unlink`
measurements, including the prior external failure. Validation was scoped to
these native comparison targets; no compiler or other backend changed.
