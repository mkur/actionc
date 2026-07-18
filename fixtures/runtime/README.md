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

The gate compiles `initialized_arrays.act` with the modern classic and MIR6502
backends. It covers global and local initialized BYTE and CARD arrays,
including the descriptor-backed CARD representations, then checks the six
result bytes at `$0600-$0605`. The fixture remains inside a generated-code
loop, so the VM does not enter cartridge or Atari OS code. The VM CLI still
requires the repository's tracked `roms/action.rom` and `roms/rev02.rom` images
when it starts execution.

The KALSCOPE contract fixture checks two observable behaviors used by that
program: `BYTE low=line, high=line+1` must alias an absolute-backed array
pointer, and calls to a current-location (`=*`) routine must expose their first
arguments in the public Action ABI homes `$A0/$A1`. Both classic and MIR6502
must produce `12 34 82 84` at `$0600-$0603`.

The scaled CARD-index fixture writes and reads unaligned fixed-base,
descriptor-backed, and typed-pointer word storage at indexes 0, 1, 127, 128,
and 255. It covers constant and scalar stores, word call arguments, computed
indexes, signed words, array-pointer values, an overlapping two-address copy,
and a call on the right-hand side of a store. The 34 result bytes at
`$0600-$0621` also exercise a destination that overwrites its own descriptor,
a page crossing, the high-byte access at `Y=$FF`, the ASL carry for indexes 128
through 255, and wrapping the corrected base high byte from `$FF` to `$00`.

It is also part of the opt-in compatibility integration tests:

```sh
cargo test --test compatibility -- --ignored
```
