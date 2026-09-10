# Untagged unions

UNION gives several typed views of the same storage. It is available in the
modern profile; compatibility rejects it. Use VARIANT instead when you want
named alternatives, checked tags and pattern matching.

```action
TYPE BytePair=[BYTE low,high]
TYPE WordView=UNION [
 CARD word
 INT signedWord
 BYTE ARRAY bytes(3)
 BytePair parts
]
WordView original

PROC Main()
 original.bytes(2)=$A5
 original.word=$1234
 original.bytes(0)=$78
 LET saved=original
 original.word=0
 PrintCE(saved.word)        ; 4728 ($1278) on Atari
 PrintBE(saved.parts.high)  ; 18
 PrintBE(saved.bytes(2))    ; 165: the word store left this byte alone
 PrintCE(original.word)     ; 0: saved is a snapshot, not an alias
RETURN
```

The complete example is [union-views.act](../../samples/union-views.act).
From the repository root:

```sh
cargo run --bin actionc -- --profile modern --backend classic --runtime standalone samples/union-views.act
cargo run --bin actionc -- --profile modern --backend mir6502 --runtime cart samples/union-views.act
```

## Representation and access

Every direct member begins at offset zero. In `UNION [BYTE first,second]`, the
two bytes overlap; use a record member for sequential fields. UNION is contextual
after `TYPE name=` and remains available as an ordinary identifier elsewhere.

SIZEOF is the largest member extent rounded up to the maximum member alignment.
OFFSETOF is zero for direct members; nested records retain their normal offsets.
Array elements use the complete padded union size as their stride. Pointer
widths, alignment and byte order follow the target, not the member last written.
On big-endian 68k, the lowest-address byte of `$1234` is `$12`, not `$34`.
These layouts are not a portable file format or a foreign aggregate ABI.

A read interprets the stored bits. It does not convert from the last-written
type, check an active member, or set a tag. A member store changes only that
member's bytes; it does not clear other bytes. Initialize all bytes you intend
to inspect: unwritten bytes are not promised to be zero. Enum views permit all
256 byte representations, including unnamed values.

Whole-value assignment copies the complete extent, including padding, and is
overlap-safe. LET captures an immutable copy. Its members cannot be written or
used to expose the snapshot's address. A copied pointer still shares its pointee;
immutability does not freeze pointed-to memory. Direct and exactly typed indirect
value parameters/results use the existing aggregate calling convention. Atari
routine storage is still non-reentrant; unions do not add automatic allocation.

Whole unions are not numbers, comparisons, truth values or CASE selectors.
Select a scalar member first, for example `CASE view.word OF`, or use `@view`
for an explicit typed address. Scalar arithmetic widths are unchanged: merely
declaring a LONGCARD member does not widen calculations on another CARD member.

## Members and composition

Members may be integers, enums, data pointers, records, fixed-length arrays or
other unions. Types remain nominal even when layouts match. Generic unions reuse
the existing finite type specialization machinery:

```action
TYPE Overlay<T,U>=UNION [T first U second]
Overlay<CARD,BytePair> value
```

Explicit type arguments are required. Imports preserve the defining identity.
Pointer recursion is allowed; inline recursive layouts and unbounded generic
expansion are rejected. The existing depth-64/1024-instance budgets apply.

Inline VARIANT, REAL and callable-pointer members are rejected, including when
hidden inside records, arrays or generic instances. Data pointers to those types
are allowed, but do not establish pointee validity. A union may be a variant
payload: the outer variant still validates its tag, and a bound union payload
is an immutable snapshot. There are no union constructors or union patterns.

## Initialization and low-level storage

Positional lists and strings cannot initialize a union, or an aggregate containing
an inline union. There is no implicit "first member" initializer. Assign members
at runtime and use whole-value copies instead.

Existing `=` storage bindings remain available:

```action
WordView memory=$0600      ; RAM-backed absolute storage, not a word initializer
WordView alias=memory     ; another name for the same storage
```

Object-level VOLATILE applies to selected member accesses and is inherited by
ordinary storage aliases. Whole volatile copies access the full extent; they
are not atomic and are not automatically safe for hardware registers. Byte order
of multi-byte accesses may differ between backends. Existing restrictions on
volatile pointer declarations and member qualifiers remain in force.

## Support and costs

| Target/backend | Union support |
| --- | --- |
| Atari modern classic, either runtime | BYTE/CARD/INT/LONGINT/LONGCARD operations, complete aggregate copies and calls |
| Atari modern MIR6502, either runtime | Same member operations, aggregate copies and calls |
| 65816 small/native and 68k | Verified layout, pointer representation, access and aggregate ABI/frame lowering canaries; no native execution claim |
| Compatibility | Rejected during semantic analysis |

Classic and MIR6502 preserve all four bytes of LONGINT/LONGCARD members,
including field writes and compound assignments. Opaque copies retain the
complete aggregate extent. Native variant validation still needs a native Error
adapter; enclosing a union does not remove that requirement.

The [cost audit](../Action_2027/UNIONS_CODEGEN_AUDIT.md) measures identical code
size and cycles for direct union views versus explicit RAM aliases. Whole-value
copies have ordinary aggregate costs, reported separately. There is no union
tag helper, implicit clear or union-specific optimizer.
