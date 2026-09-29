# Component stores for 24-bit address expressions

Sole-consumer far-pointer stores now calculate and store the low word before
calculating and storing the bank byte. Selection preserves carry/borrow between
the two components without constructing a complete A/X result. The source is
an unexposed stack home or a captured value; observable reads retain their
original order, including when public source and destination overlap.

The baseline is actionc `9dac668a`, with Exec revision `134130b` and the same
133 generated source/layout inputs as the
[empty-frame measurement](../65816-empty-frames/README.md).

| Measurement | Before | After |
| --- | ---: | ---: |
| Exec routine code, 960 routines | 478,763 bytes | 478,620 bytes |
| NewList code | 92 bytes | 79 bytes |
| NewList frame / local peak | 0 / 0 bytes | 0 / 0 bytes |

Ten routines shrink, saving **143 bytes**; none grows. All 960 frame maps pass
Exec's validator, with unchanged frames, spills, stack peaks, argument homes,
object/temporary homes and call maps. Reserved bank-zero delta: **0 fixed bytes
and 0 bytes per task**, including guards, alignment and unused capacity. Counts
exclude data, alignment and container costs. This is a compile-only local
compiler override; the compiler pin and play image are unchanged.

The [NewList listing](newlist.asm) shows the immediate low-word store and bank
carry. It reads from the private incoming pointer home; destination setup uses
DP `$80..$82`. Unknown indirect writes still invalidate the prepared-base cache.
The [routine table](exec-routines.csv) and [provenance](provenance.json) record
the complete size comparison and input/compiler/image hashes. Reproduce with
the same module paths and compile command as the
[address-consumer measurement](../65816-address-consumers/README.md), using
`/tmp/exec816-component-stores.a816.json` as the output. The listing uses the
existing decoder with an in-memory version-3 envelope; version-4 arithmetic
fault metadata does not change instruction encoding.

Validation: 334 MIR65816 unit tests pass (one existing ignored), 39 emission,
o65 and state-boundary integration tests pass, and 21 native tests pass across
`address_consumers`, `home_demand` and `pointer_values`. These cover raw and
optimized NIR, exact three-byte stores, carry/borrow/wrap, overlapping public
sources, volatile read snapshots, relocation, stack guards and IRQ/NMI reentry.
The component-store selection regression checks that no TAX/TAY/TXA/TYA result
transfers remain and that the word store precedes bank arithmetic.
