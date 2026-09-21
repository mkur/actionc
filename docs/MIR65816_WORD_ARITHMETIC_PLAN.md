# Native 16-bit ADD/SUB implementation plan

Status: completed on 2026-09-21. Baseline/coverage is committed in `7907b77`,
emission in `7803040`, and qualification in the commit containing the
[results and validation report](MIR65816_WORD_ARITHMETIC.md). ADD and SUB each
measure **98 bytes / 135 cycles**, with unchanged stack use and guards. All
48 ordinary native tests pass in debug and release.

The approved plan below is retained as the implementation's acceptance boundary.
It was based on main `ef02c8f` and the
[native vbcc comparison](MIR65816_VBCC_COMPARISON.md).

## Objective and acceptance boundary

For a verified two-byte integer ADD or SUB, load the left operand into 16-bit A,
perform one native ADC/SBC, and write the result to its existing allocated stack
home. Both raw and optimized compilation use this target selection.

Preserve `action65816.native.v1`, image v3, the experimental o65 profile, stack
guards, fixed frames, temporary homes, outgoing argument layouts, and call
barriers. No SemIR/NIR change, allocator change, persistent CPU/DP allocation,
return-marshalling optimization, branch relaxation, or pointer-arithmetic
optimization belongs to this slice. Other scalar operations retain their current
selection, including bytewise fallback for ineligible ADD/SUB operands.

Expected gains are fewer instructions, fewer accumulator-width switches, and
less DP scratch traffic. Reading/writing the same two-byte stack values still
transfers the same number of stack bytes. Frame size and observed stack depth
should remain unchanged; general register allocation is separate work.

## Baseline code and constraints

| Component | Current behavior / implication |
| --- | --- |
| [`Builder::operation`](../src/mir65816/emit/select.rs) | Switches every operation except Load/Store/Call to A8 before dispatch. Select the word path before this switch. |
| `Builder::binary` in the same file | Implements ADD/SUB one byte at a time through DP `RIGHT=$10`; `value_byte` zero-extends narrow operands. |
| `value_width`, `value_memory`, `temp`, `parameter` | Expose physical widths and homes. Reuse these facts without recovering source semantics or following a temp back to its original load. |
| [`Code::a16` and `mark`](../src/mir65816/emit/code.rs) | Track accumulator width locally and discard knowledge at labels. Retain that policy. |
| [`abi::stack::access_displacement`](../src/mir65816/abi/stack.rs) | Can validate a complete width against `1..=255`, including current downward S movement. The selector's existing `displacement` helper validates only one byte. |
| [`AllocatedFrame`](../src/mir65816/emit/allocation.rs) | General arithmetic temps have invocation-owned stack homes. Simultaneously live inputs/outputs cannot overlap; no temporary address escapes. |
| [`Mir65816Op::Binary`](../src/mir65816/mod.rs) | Has an explicit result width and signedness. PointerOffset is a separate operation. Signed widening is already an explicit Cast. |
| [`disassemble65816.py`](../tools/disassemble65816.py) | Supports immediate ADC/SBC but needs stack-relative `$63`/`$E3` decoding. |

An existing load may already have read volatile, absolute, or pointed-to memory
into a private temp. Arithmetic on that captured temp is eligible. This does
not authorize folding, widening, duplicating, or reordering the original load.

## Exact selection rule

Admit only `Mir65816Op::Binary` with Add/Sub, result width two, and a two-byte
stack destination. Accept these operands in either position:

| MIR value | Word operand |
| --- | --- |
| `U16(bits)` | Immediate preserving all 16 bits, including signed representations. |
| `U8(bits)` | Zero-extended immediate; required for narrow loop-step constants. |
| `Temp(id, width=2)` with a matching two-byte stack slot | Checked stack-relative word. |
| `Param(id)` with physical width two | Checked stack-relative word from its authoritative parameter home. |

Leave narrow memory operands, wider values, DP homes, null/address/symbolic
constants, other operations, and other result widths on the existing path.
In particular, never turn a one-byte stack operand into a two-byte read followed
by a mask. Do not add implicit signed extension; consume existing explicit casts.
Signed and unsigned two-byte ADD/SUB use the same modulo-65536 bit operation.

Use a small private operand representation, for example
`Immediate(u16)` or `Stack(u8)`, after checking the full word extent. Classification
must be side-effect-free. A recognized but unsupported shape returns "not
selected"; undefined temps/parameters, inconsistent widths, invalid homes, and
out-of-range accesses remain errors rather than being hidden by fallback.

Validate the destination and both source operands before emitting any bytes or
changing mode knowledge. Use `access_displacement` with width two and the actual
`Builder::delta` for every stack word. A start at 254 is valid; a start at 255 is
invalid because its trailing byte exceeds the supported range. No truncating
casts, saturating offsets, or assumptions that delta is zero.

## Emission sequence

For stack/stack ADD, the intended local sequence is:

```asm
REP #$20          ; only when Code::a16 needs it
LDA left,S        ; $A3, checked two-byte source
CLC               ; $18, initialize carry for this operation
ADC right,S       ; $63, native two-byte arithmetic
STA destination,S ; $83, checked two-byte destination
```

For SUB use `SEC` (`$38`) and `SBC right,S` (`$E3`). Immediate left operands use
`LDA #word` (`$A9`); immediate right operands use `ADC #word` (`$69`) or
`SBC #word` (`$E9`). Support immediate-left subtraction directly without swapping
operands. Constant folding remains the upstream optimizer's responsibility.

Call this path from `operation` after existing Call handling and before the
generic A8 setup. If not selected, enter the unchanged byte path. Leave A in
16-bit mode on success; the next operation, label, edge, call, and return keep
their existing mode-management rules. Do not introduce global mode tracking.

The sequence changes A and ordinary arithmetic flags, and writes only the
destination word. It does not alter S, D, DBR, I, X/Y widths, or decimal mode;
the ABI supplies binary arithmetic. It uses no DP scratch, temporary pushes,
helpers, calls, or symbolic fixups. Do not route ADC/SBC through the existing
generic `memory` helper, whose DP handling selects LDA versus STA, not arithmetic.

Store the result before finishing the MIR operation. Existing closed-operation
liveness proofs remain sufficient, and no value gains a lifetime across a call
or control-flow boundary. Interrupts may occur between LDA, carry setup, ADC/SBC,
and STA; qualify A/P restoration through the existing context bridge.

## Commit 1: subtraction baseline and execution coverage

Extend [`tools/compare65816/corpus.py`](../tools/compare65816/corpus.py) with an
equivalent `subtract(x,y)` pair and independent modulo-65536 results. Include
`0-1`, `$0100-1`, `$8000-1`, `$7FFF-$FFFF`, equal inputs, and asymmetric inputs.
Update `report.py`'s representative-vector selection and regenerate/check the
paired fixtures. This makes 14 paired kernels; retain the existing 13-kernel
snapshot unchanged. Record the extended baseline before editing the emitter,
with compiler/tool/input hashes and the existing raw/optimized build settings,
under `docs/benchmarks/65816-word-arithmetic/before/`.

Add `tools/native65816-runtime-tests/tests/word_arithmetic.rs` with semantic
tests that pass on the current bytewise implementation. Compile once per case
and mode, then provide runtime inputs. Reuse the existing Bus, ABI caller,
canaries, and host wrapping arithmetic; do not create a second CPU model.

| Coverage | Required cases |
| --- | --- |
| Value boundaries | Cross product of `0, 1, $00FF, $0100, $7FFF, $8000, $FFFF`, using unsigned and signed 16-bit representations; both initial I states. |
| Operand shapes | Stack/stack, stack/immediate, immediate/stack, repeated operand, U8/U16 constants, and parameters. Private selector tests may construct MIR for shapes optimized source would erase. |
| Carry independence | Consecutive additions/subtractions where the first leaves carry set/clear; ensure every operation initializes its own carry. |
| Width transitions | Byte operations before and after word arithmetic; mixed byte/word loops and block joins. |
| Memory ordering | Volatile CARD inputs retain exactly the original ascending byte reads; pointer aliases and stores do not cause stale loads or neighboring-byte accesses. |
| Calls | Live 16-bit values around direct/indirect calls to an assembly callee that clobbers A/X/Y and all 64 DP scratch bytes. Reuse the pattern in `stack_allocation.rs`. |
| Unchanged fallback | BYTE, SIZE/pointer, LONGCARD/LONGINT, and one-byte stack operands inside a wider operation. |

Keep existing LF/CRLF normalization for fixture text. The comparison build's
`--verify-crlf` exercises both source spellings through real compilers; any new
newline-sensitive test instrumentation also needs both forms through that path.
Commit this coverage only after it passes against current emission.

## Commit 2: word selection, encodings, and focused regressions

Primary implementation: [`emit/select.rs`](../src/mir65816/emit/select.rs).
Add the private classification/preflight helper and the word sequence above;
leave allocator, ABI, NIR, and generic memory access behavior unchanged.

Extend [`tests/mir65816_emission.rs`](../tests/mir65816_emission.rs) and, where
private state is required, selector unit tests. Check selection in raw and
optimized output, both arithmetic opcodes and operand orders, U8 zero extension,
exact destination width, and acceptance/rejection at stack displacement limits.
Exercise nonzero delta directly in helper tests and retain existing outgoing-call
range regressions. Invalid plans must fail before the fast path appends bytes.
Check that legal fallback does not receive a partial word sequence or spurious
mode switch. Assert unchanged allocation/frame/call metadata.

Add disassembler support for ADC/SBC `d,S`, with focused decoding/truncation
tests in a new `tools/test_disassemble65816.py`. Verify the new encodings against
ca65 assembly, and execute the resulting machine bytes on the pinned native VM.
Use decoded instruction boundaries or isolated selector output for encoding
assertions; do not search entire binaries for opcode bytes that may be operands.

Update the scalar instruction-selection section of the
[emission contract](MIR65816_EMISSION_CONTRACT.md) with eligibility, complete-word
range checks, fallback, mode, scratch, and the requirement to store the result
before leaving the operation. Document implemented behavior without weakening
any verifier or ABI contract.

Before this commit, run the focused root tests and the native arithmetic,
word-arithmetic, memory, interop, indirect, stack-allocation, and stack-fault
targets. New source cases must execute serialized raw and optimized output.

## Commit 3: preemption, relocation, and measured qualification

Run the existing native suite in debug and release once the implementation and
focused regressions pass. It covers context switching, every reached enabled
instruction address, seeded IRQ/NMI, helpers, faults, and relocated o65 execution.
Confirm that both new ADC/SBC forms are reached by interrupt qualification,
including carry live before arithmetic and A live before the store. Add a small
targeted context case with wrap/borrow inputs only if those windows are not
already reached; do not substitute interrupt-mask variation for preemption.

Rebuild and execute the full 14-pair corpus, raw and optimized, with LF/CRLF
equivalence and both host VM build modes. Keep the historical snapshot immutable;
save a separate post-change snapshot and before/after table. Record code bytes,
cycles, frames, observed stack peaks, stack/DP traffic, guards, and artifact
hashes. Report ADD and SUB separately, plus constant-chain, sum-loop, rotation,
recursion, byte-sum, and forward-copy effects. Save the new snapshot under
`docs/benchmarks/65816-word-arithmetic/after/`.

The known optimized vbcc unlink corruption remains a reported comparison-test
failure. Require zero Action failures and no additional C failures; do not
disable that assertion or claim the opt-in comparison suite is green. All
ordinary native tests must pass.

## Validation commands during implementation

The implementation used these checks, including the new tests from commits
1 and 2; the results report records their scope:

```sh
cargo test --lib mir65816
cargo test --test mir65816_abi --test mir65816_contract --test mir65816_emission \
  --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
python3 -m unittest discover -s tools -p 'test_disassemble65816.py'

python3 tools/native65816-runtime-tests/qualify.py \
  --test arithmetic --test word_arithmetic --test memory --test interop \
  --test indirect --test stack_allocation --test stack_faults

# Final broad qualification, after focused failures have been resolved:
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

Use the [comparison runner](../tools/compare65816/README.md) for artifact build,
execution, and report commands. Do not use bare cargo in the isolated native
workspace: its qualification runner supplies the pinned CPU correction.
Run only relevant tests during iteration; do not repeat the broad passing suites
unless later changes or failures justify it.

This plan changes target emission, not NIR or semantic contracts. If actual
implementation changes those contracts, stop treating it as this bounded slice
and apply the contributor-required NIR snapshots, sweep, and full `cargo test`.

## Completion criteria

- Word ADD/SUB is selected for every admitted form; unsupported legal forms
  retain correct bytewise emission. Signed widening remains explicit.
- Raw/optimized machine execution passes boundary values, memory traces,
  clobbering calls, guard failures, preemption, and relocation coverage.
- The existing optimized add(13,41) improves from **116 bytes / 162 cycles**.
  Initial acceptance ceilings are **100 bytes / 140 cycles**, including its
  unchanged guard and return. Replacing only the current arithmetic sequence
  suggests about 98 bytes / 135 cycles; that is an estimate to verify, not a
  measured result. Explain any discrepancy before revising a budget.
- Subtraction and representative arithmetic loops improve in both code size
  and cycles where this path is exercised. No unexplained regression in the
  remaining corpus; account explicitly for unchanged fixed overhead.
- Existing frame sizes, stack peaks, argument/result layouts, storage-location
  and stack-cost metadata, and guard reservation amounts are unchanged. Code
  sizes, addresses, call-site offsets, and relocated branch targets may change.
- Commit the measured qualification and updated emission contract. No ABI
  version bump, new runtime dependency, or Exec816 pin change is required.
