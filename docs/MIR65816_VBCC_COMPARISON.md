# Native 65816: actionc versus vbcc

On this simple-program corpus, vbcc produces substantially smaller and faster
code for all 12 kernels whose optimized outputs are correct in both compilers.
Actionc passes all 13 kernels in both modes. Optimized vbcc fails the remaining
kernel, a linked-list unlink using three-byte pointers, by overwriting the next
field. Its 77-byte result is **not a valid performance win**.

The main opportunity for actionc is native-width arithmetic and keeping loop
state out of repeated stack loads/stores. Mandatory stack checks explain part
of the leaf-function cost, but little of the measured loop cost. Current main
already reuses temporary stack slots and uses DP homes in eligible pointer
leaves; this comparison does not describe the older Exec816 pin `c2268b7`.

## Scope and reproducibility

Measured on 2026-09-21 at actionc `12597a9` with no local compiler-source changes.
The external compiler is vbcc V0.9i pre, 65816 backend V0.2, assembled by vasm
2.0f / 6502 backend 1.0c and linked by vlink 0.18a. Tool hashes, source hashes,
VM provenance, and emitted-artifact hashes are in the
[snapshot](benchmarks/65816-vbcc/provenance.json).

The [runner instructions](../tools/compare65816/README.md) reproduce the
[paired corpus](../tools/native65816-runtime-tests/tests/fixtures/code_quality)
and [all measurements](benchmarks/65816-vbcc/results.csv). The
[complete tables](benchmarks/65816-vbcc/tables.md) cover raw and optimized code.
This first corpus targets **native 65816**, not 6502 or 68000.

The 13 pairs cover unsigned 16-bit identity/add/maximum, a constant arithmetic
chain, 32-bit shifts/XOR, loop-carried values, summation, recursion, two direct
calls, byte traversal, record access, list unlink, and explicit forward copying.
Sixty vectors include unsigned wraparound, zero-trip loops, bank crossings,
and both overlap directions. Inputs are supplied after compilation. Neither
language receives constant benchmark arguments during compilation.

Action raw means `--no-opt`; optimized is the default NIR pipeline. Vbcc raw
means `-O=0`; optimized means the `-O=1023` backend bitmask selected by the vc
driver's level 2. These settings do not disable all frontend simplification:
vbcc folds the constant chain even at `-O=0`.

C uses `-mhuge -ptr24 -near-threshold=0 -no-near-const`. Three-byte stored pointers
and six-byte list nodes match Action's layout; huge pointer arithmetic must
cross banks. No forced inlining or `restrict` is used. Vasm uses `-816
-opt-branch -Fvobj`; vlink resolves the object into a flat binary at `$010000`.
The VM consumes that final binary, or Action's serialized image, independently
of compiler IR.

## Representative optimized results

Cells are **actionc / vbcc**. All code includes worker/helper bodies, prologues,
stack checks where emitted, and RTL. Cycles cover worker entry through RTL.
Stack is the deepest observed S below worker-entry S; it includes nested calls
but excludes the already-present incoming arguments and three-byte return frame.

| Kernel / input | Code bytes | VM cycles | Additional stack bytes |
| --- | ---: | ---: | ---: |
| identity(13) | 87 / 2 | 108 / 8 | 4 / 0 |
| add(13, 41) | 116 / 8 | 162 / 21 | 8 / 0 |
| constant chain(13) | 112 / 6 | 148 / 13 | 6 / 0 |
| maximum(13, 41) | 234 / 12 | 210 / 21 | 6 / 0 |
| wide shifts($12345678) | 379 / 57 | 653 / 147 | 14 / 2 |
| eight loop rotations(13) | 401 / 37 | 2,624 / 308 | 26 / 0 |
| sum loop(13) | 311 / 22 | 3,542 / 344 | 16 / 0 |
| recursive sum(13) | 368 / 28 | 5,363 / 699 | 190 / 67 |
| two calls(13, 41) | 412 / 24 | 688 / 67 | 20 / 3 |
| byte sum(16 bytes, bank crossing) | 405 / 35 | 6,747 / 630 | 22 / 0 |
| record field(bank crossing) | 106 / 10 | 151 / 24 | 8 / 0 |
| unlink | 129 / **invalid** | 189 / **invalid** | 0 / **invalid** |
| forward copy(8 bytes) | 410 / 42 | 4,023 / 406 | 18 / 0 |

These are individual kernel measurements, not an application benchmark score.
For the representative valid pairs, cycle ratios range from 4.44× to 13.5×.
The VM counts CPU cycles without memory wait states or platform startup work.

## What the code shows

**16-bit arithmetic is still expanded bytewise in actionc.** Its
[optimized add](benchmarks/65816-vbcc/add.optimized.actionc.lst) copies both
arguments to temporary stack homes, changes M to 8-bit, performs two ADCs
through DP `$10`, stores the result, and marshals the return through DP
`$08..$0B`. The
[vbcc add](benchmarks/65816-vbcc/add.optimized.vbcc.lst) is eight bytes:

```asm
STA $00       ; first argument arrived in A
LDA $04,S     ; second argument
CLC
ADC $00       ; native 16-bit addition
RTL
```

Action's ABI explains its incoming stack arguments, but does not require
bytewise CARD arithmetic or clearing X for a CARD return. X is unspecified
for 16-bit results. After attributing the 45-byte / 32-cycle checked-entry
sequence, add still contains 71 bytes and consumes 130 cycles. This is an
accounting breakdown of unchanged emitted code, not an unchecked build.

**Branches also carry avoidable expansion.** Action's maximum kernel compares
individual bytes, materializes a boolean in a stack slot, reloads it, and uses
multiple four-byte JML transfers within the same small routine. Vbcc uses one
16-bit stack-relative CMP and short conditional branches. Adjacent REP/SEP
pairs and jumps to the next instruction also survive in Action's output.
These are further target-level opportunities, separate from slot reuse.

**Loop state accounts for substantial traffic.** At `n=13`, optimized actionc
[sum-loop](benchmarks/65816-vbcc/sum_loop.optimized.actionc.lst) performs 287
stack-byte reads and 230 writes. The
[vbcc loop](benchmarks/65816-vbcc/sum_loop.optimized.vbcc.lst) keeps the counter
in X and the sum in DP; its three stack reads are the final RTL and it writes
no stack bytes. Action's entry check takes 32 of its 3,542 cycles. Using more
of the available DP scratch would help, but native-width instruction selection
is also necessary.

**Current optimizations help selectively.** The constant chain shrinks from
427 to 112 bytes and 658 to 148 cycles in actionc. Unlink improves from 191
bytes / 324 cycles / 6 stack bytes to 129 / 189 / 0, using nine DP bytes.
Conversely, optimized loop rotation grows from 385 to 401 code bytes and from
18 to 26 stack bytes, while saving only ten cycles. Sum-loop's frame grows
from 14 to 16 bytes and byte-sum's from 16 to 22. Existing lifetime reuse
controls stack growth; it does not provide general register allocation.

**Calls show both ABI and clobber-analysis differences.** Vbcc's recursive
routine saves/restores the callee-preserved DP register r16, using 67 additional
stack bytes at depth 13 versus actionc's 190. In the two-call kernel it keeps
the first result in r0 across a second call to the visible Helper, whose emitted
code only changes A/X/flags. This is narrower knowledge than the general
call-clobbered ABI. Action's two-call path spends 160 cycles in checks; recursion
spends 864. The remaining costs still include argument marshalling, frames,
scalar lowering, and conservative storage across calls.

## Correctness findings

Both host debug and release runs produce identical 240 measurement records,
each covering I=0 and I=1: **480 executions per host build**. All 240 actionc
executions pass. Vbcc passes 238 and fails the same optimized unlink vector
under both I states. All raw vbcc kernels pass.

The [optimized vbcc unlink listing](benchmarks/65816-vbcc/unlink.optimized.vbcc.lst)
contains this final machine instruction at `$010030`:

```asm
97 00   STA [$00],Y    ; Y=2, M=0: writes two bytes
```

The target is the third byte of `previous->next`, a three-byte pointer.
There is no preceding SEP to make this store one byte wide. At `$12FFFF`,
`previous->previous` changes from `$00` to `$FC`; the test records the offending
store PC. The raw compiler emits the required byte-width store and passes.
The invalid optimized result measures 77 bytes / 163 cycles, but cannot support
a code-quality claim. Pointer size assertions, matching record layouts, and
full-object memory checks are essential here.

Vbcc also uses a 16-bit load followed by `AND #255` for nonvolatile byte-sum,
reading one byte beyond the logical buffer on the final iteration. The harness
maps accessible canary padding, records these reads, and forbids padding writes.
These kernels operate on ordinary RAM; they do not establish equivalent MMIO
or volatile-access behavior.

The first maximum spelling, `IF x>y THEN RETURN(x) ELSE RETURN(y) FI`, was rejected
by actionc's raw native emitter with `unresolved terminal fallthrough`. The
checked-in pair uses the equivalent early-return form in both languages. No
compiler workaround or correction was included in this comparison slice.

## ABI and measurement boundaries

The benchmark preserves the
[Action native ABI](MIR65816_PHYSICAL_ABI_V1.md) and all stack guards. Action
passes every argument on the stack, including padding; vbcc passes the first
argument in A or A/X. For example, add has five incoming stack argument bytes
in Action and two in C. The CSV records those separately. Caller instruction
cost is excluded for both; internal callers remain measured. Action's unused
dummy Main is excluded; both emitted Helper bodies are included for direct_calls.

Action owns 64 call-clobbered DP scratch bytes per execution domain. This vbcc
mapping reserves 80 bytes: 32 two-byte pseudo-registers and four four-byte
scratch temporaries. Of these, r16..r27 are callee-preserved and checked by the
harness. Therefore vbcc's results do not imply ABI compatibility with Exec816.

The harness verifies results, complete memory objects, padding writes, stack
canaries, S/D/DBR, native mode, M/X/decimal/I state, Action domain metadata,
and vbcc's callee-preserved DP. Interrupt masks are varied without injecting
IRQ/NMI. This comparison does not qualify vbcc preemption, arbitrary external
helper clobbers, or Action's previously qualified context bridge. Cycles spent
in full recognized stack-check sequences are counted separately without
rewriting any machine code.

## Focused next implementation slice

The [word-arithmetic implementation plan](MIR65816_WORD_ARITHMETIC_PLAN.md)
specifies operand eligibility, commit boundaries, tests, and acceptance budgets.

Add native **16-bit scalar ADD/SUB emission for stack or immediate operands**,
using M=0 and native stack-relative operands where legal. Keep the current
allocated homes, public arguments/results, guards, and call barriers. Store
each result back to its existing home; a general register allocator is a later
change. This isolates an instruction-selection improvement demonstrated by
add, constant-chain, and the loop kernels.

Limit the fast path to verified two-byte integer operations with known ordinary
storage. Preserve byte-exact volatile accesses and existing lowering for 24-bit
pointers and wider values. Do not retain CPU/DP values across calls, helpers,
machine blocks, or control-flow joins in this slice. Validate unsigned wrap,
subtraction borrow, signed bit patterns, aliasing-sensitive stores, and both raw
and optimized machine execution; repeat existing native preemption/guard tests
when changing emitter behavior. The corpus provides a baseline for measuring
the gain without changing the ABI or weakening checks.
