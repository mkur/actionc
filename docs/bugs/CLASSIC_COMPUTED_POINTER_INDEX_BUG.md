# Classic computed pointer index overwrites its own base

Status: fixed, 2026-09-05. Exposed by the second Oscar64 behavioral-port batch
against compiler commit `37ee296`; the repair is separate from the ports.

## Trigger and scope

[`arraytest.act`](../../fixtures/runtime/oscar64/arraytest.act) retains these
small routines from Oscar64's 16-bit reverse-copy tests:

```action
PROC Reverse(INT POINTER d,s INT n)
  INT i
  i=0
  WHILE i<n DO d(i)=s(n-i-1) i==+1 OD
RETURN

PROC ReverseByte(INT POINTER d,s BYTE n)
  BYTE i
  i=0
  WHILE i<n DO d(i)=s(n-i-1) i==+1 OD
RETURN
```

Before the repair, Compatibility and Optimized failed under both ActionCart and Standalone.
MIR6502 passes the same independent memory expectations under both runtimes.
All 120 classic VM cases failed; all 60 MIR cases passed. The classic failures
include the original 100-element checks, not just added boundary cases.

For example, with source `$5000`, word destination `$5800`, and count 2, the
host supplies source words `[0, $00FF]`. The first reversed word must be
`$00FF`; classic instead writes a low byte of `$00` at `$5800`.

The full fixture also checks all original sums, source buffers, and guards.
Even an external count of zero fails the original byte-index reversal: the
first original reversed element at `$7805` is expected to be 9, but retains
the poison low byte `$A5`. No watchdog or object-buffer overlap is involved.

## Confirmed emitted-code defect

The Compatibility/Standalone listing of `Reverse` shows this order:

1. Prepare destination `d(i)` in `$AE/$AF`.
2. Load source pointer `s` into `$AC/$AD`.
3. Evaluate the inner subtraction `n-i` **into `$AC/$AD`**, destroying `s`.
4. Subtract 1 into `$C0/$C1`.
5. Scale `$C0/$C1` and add it to the now-corrupted `$AC/$AD`.
6. Read the source word through that wrong pointer and store through `$AE/$AF`.

For `n=2, i=0`, the word-index routine consequently reads from `$0004`, not
`s+2`. The BYTE-index routine likewise overwrites the source pointer's low
byte with the `n-i` intermediate before adding the scaled final index.

The shared fallback in
`src/codegen/expr.rs::pointer_index_slot_with_addr` loads the base before
calling `emit_index_expr_to_temp`. Selecting a different final index temp does
not guarantee that recursive expression materialization preserves the base.
This is distinct from the previously fixed scale-carry loss at index 128:
the new failing listing already contains that fix's PHP/PLP carry schedule.

## Reproduce

From `tools/vm-runtime-tests`:

```sh
cargo test --locked --test oscar64_conformance oscar64_classic_reverse
cargo test --locked --test oscar64_conformance oscar64_mir_reverse
```

Both commands now pass. Neither uses expected-panic assertions or ignored
cases; the fixture expressions and memory oracles are unchanged.

To inspect the code, from the repository root:

```sh
cargo run --bin actionc -- --mode compatibility --runtime standalone \
  --listing /tmp/oscar-arraytest-classic.asm -o /tmp/oscar-arraytest-classic.xex \
  fixtures/runtime/oscar64/arraytest.act
```

## Repair and regression coverage

The general pointer-index fallback uses the existing
`arithmetic_operand_needs_materialization` predicate to select stack
preservation of the captured base around index evaluation. It restores that
base before scaling/adding the index; no spare scratch pair or source-specific
recognition is introduced. Constant and simple-scalar fast paths remain
unchanged, as does the earlier scale/base carry schedule.

This preserves the existing base-before-index read order. Reloading the
pointer variable after evaluating the index would be observably different if
an index call changed it. A focused exactly-once call test checks that case.

Two additional focused tests cover all four accepted pointer scratch pairs,
byte/word element sizes, stack canaries and unchanged X/Y for pure index
arithmetic, wrapping address calculations, BYTE/INT/CARD indexes, nested
addition/subtraction, and load/store/copy consumers. Both tests failed before
the fix and pass afterward. All 180 original/extended reverse-copy VM cases
pass with unchanged source expressions and independent oracles.
