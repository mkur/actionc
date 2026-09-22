# Post-X memory traffic and bounded INX forecast

Measured against qualified loop-X output at `7a73fca`, with identical compiler
selection at `e1baff3`. Inventory and forecasts were frozen at `81b5ac4` before
the INX implementation `1ce9624`. The [qualified result](MIR65816_LOOP_INX.md)
matches every forecast below. This historical inventory preserves its original
measurements and Exec816's independently maintained compiler pin.

The [typed facts](benchmarks/65816-post-x-inventory/facts.json),
[complete instruction inventory](benchmarks/65816-post-x-inventory/inventory.json),
[table](benchmarks/65816-post-x-inventory/tables.md), and
[movement counts](benchmarks/65816-post-x-inventory/movement.json) cover 28
raw/optimized builds, 30 counted routine instances, 28 uncounted Main wrappers
and all 132 Action records. Both host profiles and incoming I states agree.
All 2,148 counted instruction sites are classified; 176 are word memory LDA
sites (154 stack, 22 DP). Counts describe actual instructions, not redundancy.

## Remaining movement

Representative input is 13. Traffic is bytes; stores and loads below are
executed word instructions, including required operations.

| Optimized worker | Bytes | Cycles | Stack peak | Stack R/W | DP R/W | Word LDA stack/DP | Word STA total/private | Goto-backedge word LDA |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Rotation | 129 | 759 | 8 | 21/32 | 68/104 | 9/33 | 68/52 | 32 |
| Sum loop | 120 | 1,092 | 6 | 85/28 | 80/160 | 40/27 | 93/80 | 13 |

Rotation retains 16 staging-byte reads and 16 writes, plus 16 fixed-local byte
writes. Its 42 memory word loads cost 177 cycles (45 stack + 132 DP); that is
existing cost, not an elimination forecast. Its body performs 17 X reads and
nine X writes. The update result is still captured in a distinct DP home,
reloaded on the backedge, stored to the parameter and followed by TAX. The
cyclic two-value rotation still needs its checked one-word staging slot.

Sum-loop retains 80 mutable-parameter byte reads and 28 writes. Its 67 word
loads cost 308 cycles; its body still has no explicit X/Y accesses. Promoting
that counter requires a separate storage/alias proof. No such promotion is
included in the INX slice.

## Frozen implementation slice

The [frozen forecast](benchmarks/65816-loop-inx/frozen.json) authenticates the
current typed X candidate and its actual `TXA; CLC; ADC #1; STA q` encoding.
Select `INX; TXA; STA q` only for the already admitted unsigned `q = p + 1`
operation in its reserved loop body. Keep separate interfering homes for p/q,
every store, the backedge load and final TAX, frame maps, ABI and guards.

The implemented admission also requires no materialized comparison after the
update: its internal labels would cross the pending relation. Those bodies
retain the previous X-mirror update sequence. No corpus forecast changes.

INX changes X from p to q immediately. The tracker must invalidate the X/p
relation before that instruction and keep the reservation pending until the
existing final TAX refresh. CPX, another increment or a load of p must reject
the pending relation, even after the retained store happens to make memory and
X equal. No pending relation may cross an unchecked join. A checked immediate
TXA supplies q in A with word N/Z; it is not an X-forwarded load of p.

C/V are not outputs of the admitted MIR ADD. The closed scalar whitelist has
no flag-valued operands or machine blocks; other ADD/SUB selectors establish
carry, comparisons establish C/Z before branching, and public flags are
call-clobbered. INX may therefore retain prior C/V instead of ADC's results.
Its instruction model must preserve the actual flags, and qualification must
compare interruptions with the new instruction stream. Retain edge A/N/Z,
mode, memory and stack contracts; do not assert old/new whole-register equality.

| Rotation, every vector | Before | Forecast |
| --- | ---: | ---: |
| Bytes | 129 | 126 |
| Cycles | 759 | 735 |
| Instructions | 218 | 210 |
| Frame/peak | 8 | 8 |
| Stack R/W | 21/32 | 21/32 |
| DP R/W | 68/104 | 68/104 |
| X-forwarded input loads | 8 | 0 |
| Dedicated X increment updates | 0 | 8 |

The three-byte/eight-instruction reduction is projected from eight executions,
not measured new output. All other 27 complete Action builds and vbcc results
must remain unchanged. A new update counter keeps the prior X-load counter's
meaning; all other forwarding/copy counts remain equal.

## Delivery and qualification

1. Commit this measured inventory, typed admission and frozen complete-image /
   all-vector forecasts before enabling INX.
2. Add typed INX effects and a checked update permission, select only the planned
   destination/operand identities, and extend independent byte/identity observers.
   Qualify wraparound, index width, C/V preservation, stale relation rejection,
   direct/self/selective tails, raw/optimized probes, flat/o65 and IRQ/NMI across
   the entire pending interval. Commit the implementation separately.
3. Verify all 28 complete images and 264 records in both host profiles and I
   states against the frozen transform. Preserve the known vbcc unlink failure.
   Run affected native library/integration checks, full native qualification,
   comparison/disassembler tests and an isolated CRLF rebuild. Save a new
   baseline and update the quality plan in a final qualification commit.

Reproduce the inventory using the pre-INX compiler/tool revision `81b5ac4`:

```sh
A816_COMPARISON_MANIFEST="$PWD/target/loop-x-after/manifest.json" \
A816_MOVEMENT_FACTS=/tmp/post-x-facts.json \
  cargo test --test mir65816_register_inventory -- --ignored
python3 -B tools/compare65816/inventory_registers.py target/loop-x-after \
  --facts /tmp/post-x-facts.json --snapshot docs/benchmarks/65816-loop-x/after \
  --output docs/benchmarks/65816-post-x-inventory --check
python3 -B tools/compare65816/post_x_inventory.py \
  docs/benchmarks/65816-post-x-inventory/inventory.json \
  --output docs/benchmarks/65816-post-x-inventory/movement.json --check
python3 -B tools/compare65816/loop_inx.py target/loop-x-after \
  --facts /tmp/post-x-facts.json --output /tmp/inx-frozen.json
```
