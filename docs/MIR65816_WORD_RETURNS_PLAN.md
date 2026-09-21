# Native 16-bit return implementation plan

Status: implemented on 2026-09-21. Baseline/coverage is committed in `5010b78`,
selection in `cac8aeb`, and qualification in the commit containing the
[results report](MIR65816_WORD_RETURNS.md). Identity measures 63 bytes / 71 VM
cycles; add/subtract measure 74 / 98, all with zero DP scratch traffic.

The approved plan below is retained as the acceptance boundary. It was based on
main `257e5f9`, following the completed
[native ADD/SUB slice](MIR65816_WORD_ARITHMETIC.md).

## Objective and scope

For a return whose authoritative ABI home is `NativeResult(A16)`, load an
eligible word directly into A16, then use the existing frame teardown and RTL.
This applies to raw and optimized compilation, including CARD and INT results.

Preserve physical ABI v1, image v3, the experimental o65 profile, stack guards,
allocation, argument/result layouts, caller cleanup, and call barriers. Preserve
the existing paths for BYTE, three-byte, four-byte, and procedure returns.
No SemIR/NIR change, persistent register allocation, arithmetic-to-return
forwarding, tail calls, load folding, branch relaxation, or Exec816 pin change
belongs to this slice. In particular, ADD/SUB still writes its allocated result;
the return reloads that home instead of assuming A still contains the value.

## Baseline code and measurements

[`Builder::return_value`](../src/mir65816/emit/select.rs) currently handles every
scalar through DP `$08..$0B`: it switches to A8, clears four bytes, copies the
result bytewise, restores A16, and loads A and X. For a stack word, this preparation
is 26 bytes and performs six DP byte writes and four DP byte reads.

`routine` already requests A16 at every terminator. `release(extent, true)`
preserves A through Y while restoring S, leaves X alone, and emits nothing when
the extent is zero. The [ABI definition](abi/action65816-native-v1.json) requires
only A16 for a word result; [ResultLocation](../src/mir65816/abi/mod.rs) leaves
unspecified registers/flags caller-clobbered. Clearing X is unnecessary here.
The BYTE high A byte and the three-byte result's high X byte must still be zero.

The existing [post-ADD/SUB snapshot](benchmarks/65816-word-arithmetic/after/tables.md)
is this plan's baseline. Its compiler implementation is `7803040`; `257e5f9`
adds qualification without changing emission. Representative optimized results:

| Kernel / input | Code bytes | VM cycles | Stack bytes below entry S | DP byte reads + writes |
| --- | ---: | ---: | ---: | ---: |
| identity(13) | 87 | 108 | 4 | 10 |
| add(13,41) | 98 | 135 | 8 | 10 |
| subtract(13,41) | 98 | 135 | 8 | 10 |
| constant chain(13) | 95 | 123 | 6 | 10 |
| sum loop(13) | 276 | 2,866 | 16 | 66 |
| recursive sum(13) | 333 | 4,687 | 190 | 196 |
| direct calls(13,41) | 377 | 611 | 20 | 30 |

Code/cycles include guards and RTL. Incoming arguments and return addresses are
reported separately in the snapshot CSV. The first four kernels spend all their
DP scratch traffic on result preparation. Their entry guard remains 45 bytes /
32 cycles. Recursive and repeated calls can benefit once per executed eligible
return; a loop's return is normally executed only once.

Replacing the stack-word preparation with `LDA d,S` saves 24 bytes and is
estimated to save 37 VM cycles. Thus add/subtract should approach **74 bytes /
98 cycles**, and identity **63 bytes / 71 cycles**. These are listing-derived
estimates, not measurements of an implemented change.

## Exact selection and emission

Gate selection on `Some(NativeResult(ResultLocation::A16))`, never just the
operand width. Preserve the existing errors for a value without a native result
home and for a function returning without a value.

Reuse `WordOperand`, `word_operand`, and `word_displacement` from ADD/SUB:

| Value | Selected form |
| --- | --- |
| U16 | Immediate preserving all 16 bits, including signed representations. |
| U8 | Zero-extended immediate, matching existing `value_byte` semantics. |
| Temp with matching two-byte stack home | Checked stack-relative word. |
| Param with physical width two | Checked word from its authoritative home, including a mutable parameter's frame object. |

Keep narrow/wide memory operands, DP homes, null/address/symbolic forms, and
other result homes on their existing paths. Do not implicitly sign-extend,
narrow, read a neighboring byte, or recover an operand's original memory source.
Consume explicit casts already present in MIR. A volatile or aliased load
captured in a private word temp may feed this path; its original accesses and
ordering must remain unchanged.

Preflight classification before emitting the return preparation or changing its
mode knowledge. Unsupported legal forms return "not selected" without bytes,
fixups, or mode changes; malformed identities, widths, homes and displacements
remain errors. Validate the full two-byte extent with the actual `Builder::delta`:
254 is the last valid starting displacement; 255 is invalid. Reuse checked
arithmetic rather than assuming delta is zero or truncating an offset.

The selected preparation is:

```asm
REP #$20       ; only if local mode knowledge requires it
LDA source,S   ; checked complete word; alternatively LDA #word
```

Both selected and fallback preparation must converge on the same existing
`release(self.frame.extent, true)` and single RTL. A nonzero frame still uses:

```asm
TAY
TSC
CLC
ADC #extent
TCS
TYA
RTL
```

Do not create a second teardown implementation or bypass stack restoration with
an early RTL. The zero-frame path omits teardown but still establishes A16 and
executes RTL. Keep current mode tracking at labels and terminators; do not
remove redundant REP/SEP elsewhere as part of this work.

The preparation writes no memory, uses no DP scratch or helper, makes no push,
and does not touch X. Teardown may clobber Y as already permitted. E, M/X widths,
binary arithmetic, D, DBR, I, restored S and RTL behavior retain their ABI
contracts. Ordinary arithmetic flags and X need not match historical values.

## Commit 1: return coverage and baseline verification

Verify the baseline snapshot's provenance and saved build hashes. Reuse
`target/word-arithmetic-after` when intact; do not regenerate the historical
snapshot with the new compiler. If artifacts are unavailable, reproduce in an
isolated checkout of `257e5f9`, including its compiler and build runner, while
preserving the working tree. Record the actual historical revision and binary
hash; `build.py --actionc` alone does not change its recorded checkout revision.
The existing 14 paired kernels / 66 vectors are sufficient for comparison; put
additional semantic cases in the native tests rather than expanding this corpus.

Add `tools/native65816-runtime-tests/tests/word_returns.rs` using the existing
serialized-image harness, independent ca65 callers, and host expectations.
Baseline semantic checks must pass before changing the emitter. Batch cases
by distinct behavior and provide runtime inputs after compilation:

| Coverage | Required behavior |
| --- | --- |
| Word bits and sources | CARD/INT values 0, 1, `$00FF`, `$0100`, `$7FFF`, `$8000`, `$FFFF`; constants, parameter/temp results, mutable parameters, explicit casts. |
| Control flow and frames | Multiple return blocks, a preceding byte operation, nested/direct/typed indirect calls, recursion, and real zero/nonzero frame cases. Use private selector tests for eligible shapes not naturally produced by source lowering. |
| Memory and calls | Return a captured volatile word with its exact byte trace; bank-crossing pointer read; an aliased word captured before a mutating call versus a reload after it. Assembly callees clobber all 64 scratch bytes and A/X/Y. |
| ABI observation | Capture A immediately after RTL, before caller cleanup clobbers it. Verify S, D, DBR, M/X, decimal mode and I in both mask states; do not require X=0 for word returns. |
| Other result homes | Retain independent BYTE high-A zero, three-byte high-X zero, full four-byte A/X, and procedure return checks. Exercise a word call alongside other widths to catch stale high-register assumptions. |

Audit existing ABI consumers and assembly fixtures for accidental X=0 assumptions
on word results. The current comparison reader already checks only A for a
two-byte return. Any necessary test adjustment must follow the published ABI;
do not weaken checks on defined bits, stack restoration, or domain state.

Compile and execute raw/optimized serialized bytes. If checked-out fixtures or
source instrumentation are added, normalize host CRLF before newline-sensitive
operations and verify LF/CRLF through the actual compilation path. Do not
normalize binaries or hide differences through broad whitespace trimming.

## Commit 2: checked word-return selection

Add a small private return-preparation helper in `select.rs`, reusing the checked
word classifier. Call it before the generic A8/DP preparation, only for the A16
result home; retain the common teardown and RTL. Avoid unrelated refactoring.

Extend [`word_tests.rs`](../src/mir65816/emit/word_tests.rs) or a focused sibling
test module with selection, rejection, and epilogue checks. Cover all admitted
forms; narrow/DP/wide fallback; missing temp/param, width/home errors; zero,
254/255, nonzero-delta and overflow boundaries. Assert that helper fallback/error
does not append a prefix or mutate mode knowledge. Preserve existing missing
return/result-home diagnostics.

Verify emitted preparation and zero/nonzero-frame tails using isolated selector
output or decoded instruction boundaries. Assert one RTL, no DP traffic in the
selected preparation, correct frame release, and unchanged storage/call metadata.
Never scan arbitrary binary bytes as opcodes. No new CPU instruction or
disassembler opcode support is needed.

Add/tighten focused root emission budgets for identity and add/subtract, and
update the existing comment that calls the old return sequence unchanged.
In the native probes, measure the executed return interval separately from
caller/helper traffic. A stack word still reads exactly its two source bytes;
the selected preparation has zero DP reads/writes. Validate independent expected
results and canaries, not just agreement with the old emitter.

Update the scalar-selection section of the
[emission contract](MIR65816_EMISSION_CONTRACT.md) with eligibility, fallback,
result-bit guarantees, and common teardown. Run focused compiler and native
tests before committing this slice.

## Commit 3: preemption, o65, and measured qualification

Extend existing preemption coverage accounting to identify reached word-return
tails within two-byte-result routines, using instruction boundaries and routine
extents. Confirm IRQ injection before and after the result load, while the
result is live in Y, around TCS, and before RTL. Require coverage of immediate
and stack sources and zero/nonzero frames; add a small targeted context case
only where existing fixtures do not reach a required shape. Keep the existing
ADD/SUB interruption checks and seeded IRQ/NMI schedules.

Run the full native suite in debug and release after focused tests pass,
including context restoration, stack faults, assembly interop, and relocated
o65 execution. Count actual passing tests and bind results to source/tool/VM
hashes and saved artifacts. Retain the pinned VM timing patch and existing
domain/NMI contracts. Mask variation alone is not preemption qualification.

Rebuild the 14-pair corpus with LF/CRLF equivalence, then execute both target
optimization modes in both host builds. Save the new snapshot under
`docs/benchmarks/65816-word-returns/after/`, with a delta against the unchanged
word-arithmetic `after` baseline. Include final identity and add/subtract
listings, representative loop/recursive/call effects, all vector measurements,
DP and stack byte traffic, frame/guard data, and provenance.

Reuse [`delta.py`](../tools/compare65816/delta.py)'s equality and regression checks;
add an optional report title while preserving its existing default if needed.
Extend `report.py` to retain identity listings for new snapshots without rewriting
historical reports. Zero-DP return assertions belong in focused tests; the full
corpus may still use DP for unrelated operations and helpers.

The known optimized vbcc unlink corruption must remain a reported failure in
both host builds. Run both comparison commands even when one fails. Require
zero Action failures, no new C failures, and identical vbcc measurements; do not
disable correctness assertions or describe the comparison test as passing.

Commit a results document and machine-readable qualification record, and update
the native-runner README and this plan's status. Keep scope to this return slice.

## Validation commands for implementation

The implementation used these checks; the results report records their scope:

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission \
  --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli

python3 tools/native65816-runtime-tests/qualify.py \
  --test word_returns --test word_arithmetic --test arithmetic --test execution \
  --test interop --test indirect --test memory --test stack_allocation --test stack_faults

# Final qualification, once focused failures are resolved:
python3 tools/native65816-runtime-tests/qualify.py -- --nocapture
python3 tools/native65816-runtime-tests/qualify.py --release -- --nocapture
```

Follow the [comparison runner](../tools/compare65816/README.md) for build,
execution, reporting and delta commands, using distinct baseline/post-change
directories. Use the qualification runner rather than bare cargo in the native
workspace. Run the corpus generator's `--check` if its fixtures change. Check
edited documentation links, Rust formatting and `git diff --check`. Broaden or
repeat passing suites only when later changes or unresolved failures justify it.

No NIR/semantic/verifier contract changes are planned, so a full root test run
and NIR sweep are not required. If implementation crosses that boundary, apply
the contributor-required checks and explicitly reassess the slice's scope.

## Acceptance criteria

- All admitted word returns use direct A16 preparation; other legal forms retain
  correct fallback. No undefined-register assumptions become public guarantees.
- Identity improves from 87 bytes / 108 cycles to at most **70 bytes / 80 cycles**;
  add and subtract improve from 98 / 135 to at most **80 bytes / 105 cycles**,
  in raw and optimized output. Guards and RTL remain included. Explain any
  discrepancy from the 63/71 and 74/98 estimates before revising these budgets.
- Identity and add/subtract have zero DP scratch traffic; the expected reduction
  is ten byte accesses per executed selected word return. No unexplained code
  size/cycle regression occurs anywhere in the comparison corpus.
- All routine storage maps, argument/result layouts, frame extents, spills,
  observed stack peaks, stack-byte traffic and guard costs remain unchanged.
  Code sizes, instruction counts, addresses and relocation offsets may change.
- Raw/optimized machine execution passes the focused boundary, memory, call and
  ABI probes, full native debug/release tests, preemption and o65 qualification.
  Measured artifacts and the known external-compiler failure remain reviewable.
- Each completed slice is committed on main, preserving existing local changes.
