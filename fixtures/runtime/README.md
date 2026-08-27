# Runtime Fixtures

These fixtures execute generated load files with `actionc-vm` and
check observable memory results. All gates use the pinned VM library through
the isolated `tools/vm-runtime-tests` crate. Compatibility scripts that also
check a compiler selection retain that preflight, then select the matching
library test.

Run the initialized-array gate directly:

```sh
fixtures/runtime/run-initialized-arrays-vm.sh
```

Run the focused KALSCOPE backend-contract gate directly:

```sh
fixtures/runtime/run-kalscope-contracts-vm.sh
```

Run the KALSCOPE code-generation pattern gate directly:

```sh
fixtures/runtime/run-kalscope-codegen-patterns-vm.sh
```

Run the modern/classic scaled CARD-index boundary gate directly:

```sh
fixtures/runtime/run-scaled-card-indexes-vm.sh
```

Run the modern/classic dual indexed CARD-compare gate directly:

```sh
fixtures/runtime/run-dual-indexed-word-compares-vm.sh
```

Run the modern/classic dual-pointer word-transfer gate directly:

```sh
fixtures/runtime/run-dual-pointer-word-transfers-vm.sh
```

Run the modern/classic ALLOCATE behavior gate directly:

```sh
fixtures/runtime/run-allocate-vm.sh
```

Run the modern/classic Toolkit SORT behavior gate directly:

```sh
fixtures/runtime/run-sort-vm.sh
```

Run the MIR6502 ordered absolute-subtraction overlap gate directly:

```sh
fixtures/runtime/run-ordered-absolute-sub-vm.sh
```

Run the MIR6502 alias-safe indirect call-field gate directly:

```sh
fixtures/runtime/run-indirect-call-fields-vm.sh
```

Run the modern/classic direct unsigned word-compare boundary gate directly:

```sh
fixtures/runtime/run-direct-word-compares-vm.sh
```

Run the MIR6502 signed return-word compare-to-zero gate directly:

```sh
fixtures/runtime/run-signed-return-word-zero-compares-vm.sh
fixtures/runtime/run-signed-word-relation-matrix-vm.sh
```

Run the direct four-byte Action call-argument arithmetic gate directly:

```sh
fixtures/runtime/run-direct-action-word-arithmetic-args-vm.sh
```

Run the indexed BYTE-to-fixed-Action-home gate directly:

```sh
fixtures/runtime/run-indexed-byte-fixed-action-args-vm.sh
```

Run the paired wrapping word-arithmetic comparison gate directly:

```sh
fixtures/runtime/run-paired-word-arithmetic-compare-vm.sh
```

Run the CIRCLE signed-INT arithmetic and comparison gate directly:

```sh
fixtures/runtime/run-circle-int-math-vm.sh
```

The gate compiles `initialized_arrays.act` with the modern classic and MIR6502
backends. It covers global and local initialized BYTE and CARD arrays,
including the descriptor-backed CARD representations, then checks the six
result bytes at `$0600-$0605`. The fixture remains inside a generated-code
loop. The isolated `tools/vm-runtime-tests` crate compiles both programs through
the `actionc` library, runs their object bytes through the pinned VM library,
and inspects final RAM directly. It does not load the Action! cartridge or
Atari OS ROM, create memory dumps, or add the VM to normal compiler builds.

The KALSCOPE contract fixture checks two observable behaviors used by that
program: `BYTE low=line, high=line+1` must alias an absolute-backed array
pointer, and a current-location (`=*`) machine routine receives its first two
argument bytes in A/X. The raw callee explicitly saves those registers to
`$A0/$A1` before a nested call and reloads them afterward; `$A0/$A1` are
callee-owned scratch here, not caller-provided argument homes. Both classic
and MIR6502 must produce `12 34 82 84` at `$0600-$0603`. Its compatibility
script now selects the corresponding direct library test.

The KALSCOPE code-generation fixture covers the program's concentrated general
shapes independently of graphics and OS state: a local byte pointer committed
and incremented between indirect stores, word add feeding XOR, two pure byte
arguments in A/X, indirect low/high word projections, and a word-indexed shift
stored into a byte array. Classic and MIR6502 must produce
`11 22 33 44 AF 45 AF 45 82 84 1F` at `$0600-$060A` and `$A5` at `$0610`.

The scaled CARD-index fixture writes and reads unaligned fixed-base,
descriptor-backed, and typed-pointer word storage at indexes 0, 1, 127, 128,
and 255. It covers constant and scalar stores, word call arguments, computed
indexes, signed words, array-pointer values, an overlapping two-address copy,
and a call on the right-hand side of a store. The 34 result bytes at
`$0600-$0621` also exercise a destination that overwrites its own descriptor,
a page crossing, the high-byte access at `Y=$FF`, the ASL carry for indexes 128
through 255, and wrapping the corrected base high byte from `$FF` to `$00`.
Its classic-backend oracle runs through the direct library harness.

The dual indexed CARD-compare fixture is the focused cross-backend oracle for
MIR6502's two-pointer compare selector. It uses odd pointer-backed table bases,
indexes 127, 128, and 255, equal and reversed operands, and a comparison across
two different arrays. Its ten-byte result range checks `<`, `<=`, `>`, and
`>=` under both modern/classic and modern/MIR6502.

The dual-pointer word-transfer fixture exercises scaled indexed-to-indirect,
indirect-to-indexed, direct private-pointer word copies, and destination-aware
word addition, and direct-word-to-indirect copies in both backends. Each
indexed direction has a disjoint case and a case where the destination
overlaps one source byte. The pointer-backed direct cases cover local and
parameter operands. Compound addition covers disjoint and identical pointers,
source pointers one byte above and below the target, page crossing, carry
propagation, and `$FFFF` wrap. Direct-source copies cover exact aliasing and
destination addresses one byte above and below the source. The MIR6502
preflight requires six selected `copy_indirect_word` operations, including two
scaled-source copies, plus one `indirect_word_compound` and one
`copy_direct_word_to_indirect` operation. The 30 result bytes at
`$0600-$061D` prove overlap-safe ordering, private-pointer rematerialization,
and low-to-high carry behavior for MIR6502. Classic is still compiled and
executes the first 24-byte shared oracle, but its pre-existing direct-source
overlap order is not used as a correctness reference for the final six bytes.

The ALLOCATE fixture includes the maintained modern Toolkit implementation and
uses separate fixed heap regions for empty-list, exact removal, split,
`$00FF/$0100` selection, insertion, left and right coalescing, repeated
allocate/free, and `AllocInit` scenarios. The 46 result bytes at
`$0600-$062D` contain hard-coded return values and free-list snapshots; backend
equality alone is not used as the correctness oracle. The Toolkit source's
original two-sided-coalescing behavior is not asserted here because it updates
the just-freed header instead of the preceding free block after a left merge;
that source-level issue is separate from backend equivalence.

The SORT fixture includes the maintained modern Toolkit implementation and
checks hard-coded BYTE, CARD, INT, and string results under both backends. It
covers both directions, duplicates, already sorted data, unsigned and signed
boundaries, string prefixes, repeated partition-list use, and sentinels around
every fixed input array. The result bytes at `$0600-$066F` are a correctness
oracle rather than a backend-equality check.

The ordered absolute-subtraction fixture places the indirect destination one
byte above `MemHi`, so the destination low byte aliases the fixed source's high
byte. Its hard-coded `$0F4E` result proves that MIR6502 captures both source
lanes and both pointer/RHS lanes before the first indirect write. This focused
gate is MIR6502-only because the classic backend's legacy schedule has weaker
pointer-alias behavior. Its runtime oracle runs through the direct library
harness.

The indirect call-field fixture selects two bounded four-byte transfers into
the fixed `$A4-$A7` call homes. One source crosses a page boundary. The other
starts at `$A3`, one byte before the destination range, so a load/store
sequence that writes an ABI home before capturing every source byte corrupts
the next argument byte. The 18 hard-coded result bytes at `$0600-$0611`
validate the callee's tag, pointer, and two word arguments. The preflight also
requires both `copy_indirect_bytes_to_fixed_zp` selections.

The direct word-compare fixture executes `Lt`, `Ge`, `Gt`, and `Le` branches
around `$0000`, `$00FF/$0100`, `$7FFF/$8000`, and `$FFFF`. Its indirect-left
cases exercise the low-byte `CMP` to high-byte `SBC` carry chain; its
indirect-right `Gt`/`Le` cases exercise safe operand reversal. The twelve
hard-coded result bytes are checked under both modern backends through the
direct library harness.

The signed return-word fixture executes all four signed relational predicates
with zero on either side of an `INT` call result. It covers `$8000`, `$FFFF`,
`$0000`, `$0001`, and `$7FFF`, requires all eight direct return-slot
selections, and checks 40 hard-coded predicate results plus a signature byte.

The CIRCLE INT fixture exercises ordinary-memory signed word add/sub into the
first A/X call argument and a narrowed Y argument, wrapping `Phiy`/`Phixy`
chains, direct signed zero and word relations, and two leaf-call results
feeding a signed comparison. Both modern backends check 30 hard-coded result
bytes at `$0600-$061D`. Three signed-overflow comparison slots have separate
classic and MIR6502 expectations: the classic backend's legacy N-only branch
misclassifies `$8000` versus `$7FFF`, while MIR6502 must satisfy the signed
language result.
It is MIR6502-only because it verifies the MIR type contract for a signed
operand on either side; classic Action comparison selection is left-operand
driven.

The direct Action word-argument fixture covers word addition and subtraction
in both the A:X first-argument lane and the Y:`$A3` second-argument lane. Its
29-byte oracle covers carry, borrow, wrap, a live companion argument,
commuted addition, and an arithmetic source spanning fixed `$A2/$A3`. The
MIR6502 preflight requires all six static arithmetic call sites to use the
direct schedule; both modern backends execute the same hard-coded oracle.

The indexed BYTE call-argument fixture zero-extends local-array elements into
canonical Action CARD argument homes. It covers the four-byte prefix and a
twelve-byte call, requires six direct placements, and starts at index 255 so
the base-plus-index calculation or a following constant offset must cross a
page for every backing alignment. Both backends must produce the same 17
hard-coded marker, argument, high-lane-zero, and completion bytes.

The direct BYTE-array and paired word-arithmetic gates also run their memory
oracles through the direct library harness. The paired gate retains its
compiler-selection preflight in the compatibility script.

The native REAL-to-INT fixture checks dynamic positive and negative rounding
under both modern backends and both runtime link modes. It also copies both
source values after conversion so the VM oracle proves that their complete
six-byte packed representations, including the exponent/sign byte, remain
unchanged.

The native REAL compaction fixture checks compile-time integer promotion and
sign-bit negation under both modern backends and runtime modes. Its memory
oracle covers positive and negative packed values, both negation directions,
and canonical positive zero after negating zero.

The native REAL decimal-mode fixture runs against a controlled FPI test shim
that returns with the processor decimal flag set, captures status immediately
after the conversion, and performs dynamic byte addition. Both modern backends
and runtime modes must clear D after the FPP call and produce the binary sum
rather than its BCD counterpart.

The scripted gates are also part of the opt-in compatibility integration tests:

```sh
cargo test --test compatibility -- --ignored
```
