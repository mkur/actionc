# External Runtime Bindings

Status: accepted internal compiler contract. The module syntax shown here is
implementation infrastructure and is not part of the standalone-runtime public
rollout.

## Purpose

The compiler-owned system-library interface must describe Action source
semantics without exposing the physical address or source routine selected by a
runtime.
The same `SYS` symbol identity is therefore used with both the cartridge and
standalone runtimes.

## Interface declarations

A named module declares a callable interface with `EXTERNAL`:

```action
MODULE SYS
  PUBLIC EXTERNAL PROC Zero(BYTE POINTER address, CARD size)
ENDMODULE
```

`EXTERNAL` is valid only on a public `PROC` or `FUNC` in a named module. An
external declaration has a parameter and result signature but no body, locals,
current-location marker, or fixed address. It contributes no code by itself.

The declaration owns the callable's source name, stable semantic identity,
types, ABI, and source-visible effects. Its implementation address is not a
constant in the importing program.

## Compiler-owned binding sources

Runtime bindings are compiler-owned Action source embedded in the compiler's
read-only virtual filesystem. They use `SET` to bind the interface identity:

```action
MODULE ACTION.RUNTIME.BINDINGS.CART
  SET SYS.Zero=$A78A
ENDMODULE
```

```action
MODULE ACTION.RUNTIME.BINDINGS.STANDALONE
  SET SYS.Zero=SYSBLK_Zero
ENDMODULE
```

The underscore in an implementation target separates the embedded runtime
unit from its routine while keeping the legacy `SET` expression unambiguous.

Binding units are metadata interpreted by the compiler; users do not import
them and they do not create a second public declaration. An absolute target is
valid only for the cartridge provider. A standalone target names a routine in
an embedded GPL runtime source unit.

The compiler requires each provider to bind exactly the public external names in
`SYS`. It diagnoses duplicate, missing, and unknown bindings; malformed targets;
a target of the wrong runtime kind; a missing implementation; and an
incompatible implementation ABI. Unreferenced declarations do not need to be
linked and add no bytes.

## Resolution

Calls, address-taking, static initializers, `SET` values, qualified imports,
open imports, and compatibility-prelude names first resolve to the external
interface's stable routine identity. Runtime binding happens after MIR6502
lowering but before target materialization:

- cartridge references become the selected absolute entry address;
- standalone references become the selected local runtime routine identity;
- the declaration-only routine is removed before emission.

Consequently `@SYS.Zero` observes the selected implementation. Standalone code
cannot accidentally retain a cartridge address, and importing `SYS` without
using a member cannot pull runtime code into the output.

The complete audited 71-routine interface is declared in
[`embedded/modules/sys.act`](../../embedded/modules/sys.act) and is available
from both providers. Runtime closure follows both explicit relocations and
legacy machine-code fallthrough. In particular, standalone `Zero` retains the
adjacent `SetBlock` body because the original six-byte entry prepares a zero
value and falls through into that implementation.
