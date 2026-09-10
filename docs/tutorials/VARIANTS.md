# Variants, generic types and pattern matching

Modern Action! supports nominal sum types with inline payloads:

```action
TYPE Event=VARIANT [
  NONE
  KEY [BYTE code]
  MOVE [INT x,y]
]
Event current

PROC Main()
  current=Event.MOVE(12,-3)
  LET saved=current
  current=Event.NONE
  CASE saved OF
  WHEN Event.NONE THEN
    PrintE("No event")
  WHEN Event.KEY(code) THEN
    PrintBE(code)
  WHEN Event.MOVE(x,y) THEN
    PrintIE(x+y) ; prints 9: saved is a value snapshot
  ESAC
RETURN
```

Constructors are positional and evaluated left to right. Nullary
constructors omit parentheses. Replace an entire value to change its alternative;
the tag and payload fields are not directly writable. Pattern bindings are
immutable and arm-local. Use `_` to discard a payload; use ELSE for unmatched
valid alternatives. Otherwise cover every alternative. A BEGIN/END block inside
an arm allows ordinary local declarations and LET.

Use `USE ALL FROM` inside a routine or `BEGIN` block to make a named variant's
constructors available without repeating its type:

```action
TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]

PROC Main()
  BEGIN
    USE ALL FROM MaybeByte
    LET item=SOME(42)
    CASE item OF
    WHEN NONE THEN
      PrintE("No value")
    WHEN SOME(n) THEN
      PrintBE(n)
    ESAC
  END
RETURN
```

The opening applies from that statement to the end of the enclosing routine or
`BEGIN` block, including nested scopes. Put local declarations before it. Inside
an IF, loop or CASE arm, introduce an explicit BEGIN/END block for the opening,
as for LET. Constructors are available in expressions and nested CASE patterns;
for example, an opened `NONE` in `Box.WRAP(NONE)` is a constructor pattern, not a
new payload binder. Use another name for a binder in that case.

Names remain case-insensitive. An opening cannot replace a visible variable,
constant, routine, type, module alias or a different opened constructor; such
collisions are errors. Reopening the same constructor identity is harmless,
including through different module aliases. Ordinary declarations in an inner
scope and subsequent LET bindings retain their usual shadowing behavior.
Qualified constructors such as `MaybeByte.SOME(n)` remain available.

CASE can also choose an integer or enum value:

```action
LET value=CASE item OF
WHEN NONE THEN
  0
WHEN SOME(n) THEN
  n
ESAC
```

This assumes the MaybeByte opening above. Exhaustive unguarded patterns need
no ELSE; all result arms must have the same type. See
[IF and CASE expressions](IF_CASE_EXPRESSIONS.md) for nesting, guards and
explicit conversions. Aggregate results remain outside this expression slice.

The target can be a local type or a public type reached through a module alias,
such as `USE ALL FROM API.MaybeByte`. This local form opens named, non-generic
VARIANT types only. It does not open enum members or modules, accept AS aliases,
or take explicit generic arguments. Module-header USE imports keep their existing
meaning. Both spellings resolve to the same constructor identity and generate
the same runtime operations.

The representation uses a BYTE tag (1..255) and an overlapping inline payload.
Zero means unconstructed, not the first alternative. Construct values with
runtime assignment before reading them. An invalid tag, including an active
nested variant's tag, invokes Error(105) on Atari before any arm or value copy
is exposed. Even ELSE does not catch invalid storage.

Construction evaluates payloads once, left to right, zeros only unused storage
and alignment gaps, then writes the tag last in its private temporary. It does
not clear active payload bytes before overwriting them. For example, a two-byte
`SOME [BYTE value]` alternative needs just payload and tag writes, with no
clearing loop. Aggregate payload copies preserve their complete byte image.

Scalars, records, unions, variants and data pointers can be payloads. Record
payloads may contain embedded fixed arrays. A [union](UNIONS.md) payload is a raw
value without its own tag or constructor patterns; the enclosing variant remains
checked. Inline values copy by value; pointers share their pointees. Recursive
definitions must cross a pointer boundary:

```action
TYPE Tree=VARIANT [
  EMPTY
  NODE [INT value Tree POINTER left,right]
]
```

See [the complete tree example](../../samples/variant-tree.act). It uses eight
arena slots, a constructed EMPTY sentinel, checked allocation and an explicit
traversal stack. There are no recursive routine calls or hidden allocations.
NIL is an explicitly declared zero-address constant, distinct from EMPTY.
Replacing a node preserves links; retaining another pointer to it shares that
node. Pointer safety, ownership and cycle handling remain the program's job.

Build from the repository root:

```sh
cargo run --bin actionc -- --mode mir6502 --runtime standalone --output variant-tree.xex samples/variant-tree.act
```

`--mode optimized` selects classic; `--runtime cart` selects cartridge runtime.
Running the example prints 1, 2, 3, 5, 6, 7, 8, 9 on separate lines. The ninth
distinct insertion is rejected without changing the full tree.

On Atari a Tree occupies seven bytes; native targets use their own pointer
width and alignment. Native layout/construction lowering is tested, but runtime
variant validation still needs native Error adapters. Atari routine-static
activation is unchanged: recursive data does not enable recursive/reentrant
procedures. Existing backend limits also apply, including classic's
LONGINT/LONGCARD restrictions.

Record, union and variant parameters and function results are supported on direct and
typed indirect compiler-defined calls in the modern profile. Parameters are independent mutable
copies; use an explicit POINTER parameter to modify the caller's object. Function
results compose with LET, assignment, constructor arguments, RETURN and CASE.
Arguments are captured once, left to right, before later argument effects.
Aggregate calls require all arguments, including trailing scalar arguments.
Foreign/cartridge entry signatures are not inferred from these source types.

Use signatures such as `Event FUNC POINTER transform(Event input BYTE n)` or
`PROC POINTER consume(Event input)`. The callback is captured before argument
effects; callbacks must have exactly matching nominal parameter/result types.
Raw numeric addresses cannot acquire an aggregate ABI. Static initialization
accepts a matching, already-declared routine address (`[@Make]`) or `[NIL]`.

## Generic records and variants

Use explicit concrete type arguments; LET can infer the resulting value type:

```action
TYPE Option<T>=VARIANT [NONE SOME [T value]]
TYPE Result<T,E>=VARIANT [OK [T value] ERROR [E error]]
TYPE Buffer<T>=[T ARRAY values(3)]
TYPE TreeOf<T>=VARIANT [EMPTY NODE [T value TreeOf<T> POINTER left,right]]

Option<BYTE> FUNC Make(BYTE n)
RETURN(Option<BYTE>.SOME(n))
```

Applications work in declarations, parameters/results, callable signatures,
LET annotations, payload types, layout queries, constructors and patterns.
`Option<BYTE>` and `Option<CHAR>` are different nominal types even when their
layouts match. Import aliases of the same definition share instances. Names
inside a generic definition resolve in that definition's scope.
There are no runtime type dictionaries, allocations, or generic routine inference.

See [generic-types.act](../../samples/generic-types.act) for executable
Option/Result composition and a three-node `TreeOf<INT>` arena. It builds with
either Atari backend/runtime and prints 7, 1000, and 12.

Type arguments are complete value types, including pointers and typed callable
pointers (callable payloads remain unsupported in variants). ARRAY/STRING
storage and routine names are not type arguments. A POINTER-qualified parameter
cannot itself be instantiated with a pointer; nested data-pointer types are not
currently represented by the source type system.

Regular self/mutual recursion through pointers is finite and supported. Inline
layout cycles and recursive specialization that increases argument structure
are diagnosed. Limits are 64 nested type applications, 64 active instantiations,
and 1,024 distinct concrete instances per compilation. Reusing an instance does
not consume another slot.

## Nested patterns

Constructor patterns can inspect inline nested variants and integer/enum literals:

```action
CASE wrapped OF
WHEN Result<Option<BYTE>,CARD>.OK(Option<BYTE>.SOME(0)) THEN
  PrintE("Zero")
WHEN Result<Option<BYTE>,CARD>.OK(Option<BYTE>.SOME(n)) THEN
  PrintBE(n)
WHEN Result<Option<BYTE>,CARD>.OK(Option<BYTE>.NONE) THEN
  PrintE("Absent")
WHEN Result<Option<BYTE>,CARD>.ERROR(code) THEN
  PrintCE(code)
ESAC
```

Arms run in source order. A later, more general pattern can cover values missed
by earlier specific ones. Fully shadowed patterns are errors; missing coverage
reports an example pattern. Literal subpatterns use the existing integer range
and exact enum-type checks. Use a binder or `_` to cover the remaining scalar
domain, including unnamed enum byte values; listing literals alone does not
prove complete scalar coverage. Payload ranges and computed expressions are
not patterns. Repeated binding names in one pattern are errors, not equality tests.

The selector is captured and validated before matching. Payload extraction is
dominated by the matching outer and inner constructor checks. Aggregate binders
are independent immutable snapshots; sibling/nested arms have distinct bindings.
Pointer patterns never implicitly dereference: bind the pointer and use a separate
`CASE pointer^ OF`. Pattern syntax is limited to 64 levels. Coverage analysis has
a 128-level recursion and 262,144-work-unit budget; exceeding either reports a
diagnostic instead of claiming completeness.

## Ordered guards

Add `IF condition` between a WHEN pattern and THEN:

```action
CASE current OF
WHEN Event.KEY(code) IF code>=32 THEN
  PrintBE(code)
WHEN Event.KEY(code) THEN
  PrintE("Control key")
WHEN _ IF CanRecover() THEN
  Recover()
ELSE
  PrintE("Other event")
ESAC
```

Match first, initialize the immutable bindings, then evaluate the guard once.
False continues to the next arm; it does not re-read the selector. Guards can
call routines, read hardware or fault, but skipped guards have no effects.
Changing the original value does not change the captured selector or bindings.
The snapshot remains shallow across pointers, so pointed-to data can change.

Guards do not contribute to exhaustiveness, even if written as constant true.
Keep an unguarded covering pattern or ELSE. Earlier unguarded patterns can make
a guarded arm unreachable, which is diagnosed. `WHEN _ IF condition THEN` is
a guarded catch-all. Bare `WHEN _ THEN` and guards on ELSE are not supported.

The same guard syntax works for integer and enum CASE. Repeating a label after
a guarded arm is allowed; overlapping unguarded labels and duplicates within
one header remain errors. A guarded interval may overlap earlier unguarded
labels if some of its values remain reachable. Enum selectors still include
unnamed byte values. CASE does not introduce a loop: EXIT still exits the
enclosing loop, and RETURN exits the routine.

Not yet supported: generic routines or omitted/inferred type arguments,
direct ARRAY payload declarations, volatile variants,
absolute/alias-backed variant declarations or raw/static variant initializers.
Use a record payload to contain an inline array. Typed pointers into explicitly
managed memory are available, but validation does not make arbitrary pointers
safe.

## Storage and copies

Independent variant objects must not partially overlap. Typed whole-value copies
(also of records with inline variants) require identical or disjoint ranges;
self-assignment, adjacent arena slots and nested variant subobjects remain valid.
Unknown pointer-copy ranges are checked before writing and partial overlap invokes
Error(106) on Atari. This does not provide general raw-pointer safety. Plain record
and UNION copies keep their overlap-safe behavior. See the
[variant storage contract](../VARIANT_STORAGE_CONTRACT.md).

## Complete examples and support matrix

[`algebraic-types.act`](../../samples/algebraic-types.act) combines Event,
ReadResult and OptionalByte with an ordinary record payload, value-returning
functions, nested patterns, guards and generic Option/Result. It prints
`65`, `0`, `9`, `10`, `7`, `5` on separate lines. The fixed-arena tree and generic
array/tree examples linked above cover explicit storage and pointer sharing.

```sh
cargo run --bin actionc -- --mode mir6502 --runtime standalone \
  --output algebraic-types.xex samples/algebraic-types.act
```

| Compilation mode | ADT support |
| --- | --- |
| Modern classic (`--mode optimized`), cart or standalone | Construct, copy, LET, direct/typed-indirect value calls, generics, nested patterns and guards, including LONGINT/LONGCARD payload operations |
| Modern MIR6502, cart or standalone | Same common subset, including LONGINT/LONGCARD payload operations |
| Native 68k / 65816 | Typed SemIR/NIR, layout and ABI/frame canaries; executable variants remain blocked on native Error adapters |
| Compatibility / original cartridge compiler | New ADT syntax is not supported; selecting the cartridge **runtime** is a separate choice |

Aggregate LET and pattern storage cannot be assigned, addressed, aliased,
converted to raw addresses or referenced from machine code. Passing their values
to ordinary by-value routines is legal. Explicit pointer values share mutable
pointees. Caller-owned results and callee-private value parameters do not add
recursive/reentrant Atari routine activation. Unknown foreign aggregate ABIs
remain errors rather than guessed signatures.

Tags are assigned in declaration order, inactive bytes are zeroed by construction,
and copies preserve the full extent. Reordering alternatives or changing target
layout can change raw bytes; there is no tag cast, stable serialization format or
ABI guarantee for foreign aggregate calls. Zero-filled storage is not a value.

See the [code-quality baseline](../ADT_CODEGEN_BASELINE.md) for measured image
size, CPU cycles, capture storage, checks and copies. There is currently meaningful
aggregate initialization/copy overhead; pattern matching is not advertised as
universally zero-cost.
