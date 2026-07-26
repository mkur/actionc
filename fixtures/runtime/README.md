# Runtime Fixtures

These fixtures execute generated load files with `action-compiler-vm` and
check observable memory results. The VM project defaults to the sibling path
`../action-compiler-vm`; override it with `ACTION_COMPILER_VM_DIR`.

Run the initialized-array gate directly:

```sh
fixtures/runtime/run-initialized-arrays-vm.sh
```

Run the focused KALSCOPE backend-contract gate directly:

```sh
fixtures/runtime/run-kalscope-contracts-vm.sh
```

Run the modern/classic scaled CARD-index boundary gate directly:

```sh
fixtures/runtime/run-scaled-card-indexes-vm.sh
```

Run the modern/classic dual-pointer word-transfer gate directly:

```sh
fixtures/runtime/run-dual-pointer-word-transfers-vm.sh
```

Run the modern/classic ALLOCATE behavior gate directly:

```sh
fixtures/runtime/run-allocate-vm.sh
```

Run the modern/classic direct unsigned word-compare boundary gate directly:

```sh
fixtures/runtime/run-direct-word-compares-vm.sh
```

The gate compiles `initialized_arrays.act` with the modern classic and MIR6502
backends. It covers global and local initialized BYTE and CARD arrays,
including the descriptor-backed CARD representations, then checks the six
result bytes at `$0600-$0605`. The fixture remains inside a generated-code
loop, so the VM does not enter cartridge or Atari OS code. The VM CLI still
requires the repository's tracked `roms/action.rom` and `roms/rev02.rom` images
when it starts execution.

The KALSCOPE contract fixture checks two observable behaviors used by that
program: `BYTE low=line, high=line+1` must alias an absolute-backed array
pointer, and a current-location (`=*`) machine routine receives its first two
argument bytes in A/X. The raw callee explicitly saves those registers to
`$A0/$A1` before a nested call and reloads them afterward; `$A0/$A1` are
callee-owned scratch here, not caller-provided argument homes. Both classic
and MIR6502 must produce `12 34 82 84` at `$0600-$0603`.

The scaled CARD-index fixture writes and reads unaligned fixed-base,
descriptor-backed, and typed-pointer word storage at indexes 0, 1, 127, 128,
and 255. It covers constant and scalar stores, word call arguments, computed
indexes, signed words, array-pointer values, an overlapping two-address copy,
and a call on the right-hand side of a store. The 34 result bytes at
`$0600-$0621` also exercise a destination that overwrites its own descriptor,
a page crossing, the high-byte access at `Y=$FF`, the ASL carry for indexes 128
through 255, and wrapping the corrected base high byte from `$FF` to `$00`.

The dual-pointer word-transfer fixture exercises scaled indexed-to-indirect,
indirect-to-indexed, and direct private-pointer word copies in both backends.
Each indexed direction has a disjoint case and a case where the destination
overlaps one source byte. The direct cases cover local-backed and
parameter-backed pointer operands. The MIR6502 preflight requires six selected
`copy_indirect_word` operations, including two scaled-source copies. The twelve
result bytes at `$0600-$060B` prove both overlap-safe ordering and private
pointer rematerialization.

The ALLOCATE fixture includes the maintained modern Toolkit implementation and
uses separate fixed heap regions for empty-list, exact removal, split,
`$00FF/$0100` selection, insertion, left and right coalescing, repeated
allocate/free, and `AllocInit` scenarios. The 46 result bytes at
`$0600-$062D` contain hard-coded return values and free-list snapshots; backend
equality alone is not used as the correctness oracle. The Toolkit source's
original two-sided-coalescing behavior is not asserted here because it updates
the just-freed header instead of the preceding free block after a left merge;
that source-level issue is separate from backend equivalence.

The direct word-compare fixture executes `Lt`, `Ge`, `Gt`, and `Le` branches
around `$0000`, `$00FF/$0100`, `$7FFF/$8000`, and `$FFFF`. Its indirect-left
cases exercise the low-byte `CMP` to high-byte `SBC` carry chain; its
indirect-right `Gt`/`Le` cases exercise safe operand reversal. The twelve
hard-coded result bytes at `$0600-$060B` validate both branch polarities.

It is also part of the opt-in compatibility integration tests:

```sh
cargo test --test compatibility -- --ignored
```
