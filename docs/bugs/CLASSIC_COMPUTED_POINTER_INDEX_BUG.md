# Classic computed pointer index overwrites its own base

Status: open, exposed by the second Oscar64 behavioral-port batch on
2026-09-05, against compiler commit `37ee296`.

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

Compatibility and Optimized fail under both ActionCart and Standalone.
MIR6502 passes the same independent memory expectations under both runtimes.
All 120 classic VM cases fail; all 60 MIR cases pass. The classic failures
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

The first command is intentionally an active failing correctness test; the
second passes. Neither uses expected-panic assertions or ignored cases.

To inspect the code, from the repository root:

```sh
cargo run --bin actionc -- --mode compatibility --runtime standalone \
  --listing /tmp/oscar-arraytest-classic.asm -o /tmp/oscar-arraytest-classic.xex \
  fixtures/runtime/oscar64/arraytest.act
```

## Follow-up repair scope

Repair general pointer/index materialization so a live base and an already
prepared destination survive all nested index temporaries. Assess an existing
preservation/staging path or an alias-safe evaluation schedule rather than a
special case for subtraction or reverse-copy loops. Simply swapping base and
index evaluation also needs proof: obtaining the base can itself use scratch
or observe storage modified by the index expression.

Add focused checks for both pointer scratch pairs, BYTE/INT/CARD indexes,
nested addition/subtraction, and load/store/copy consumers. Keep these ports'
source expressions and independent oracles unchanged when enabling the fix.
No compiler implementation was changed as part of the test-port batch.
