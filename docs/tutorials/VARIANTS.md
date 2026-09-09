# Variant values and fixed-arena trees

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

Constructors are qualified, positional and evaluated left to right. Nullary
constructors omit parentheses. Replace an entire value to change its alternative;
the tag and payload fields are not directly writable. Pattern bindings are
immutable and arm-local. Use `_` to discard a payload; use ELSE for unmatched
valid alternatives. Otherwise cover every alternative. A BEGIN/END block inside
an arm allows ordinary local declarations and LET.

The representation uses a BYTE tag (1..255) and an overlapping inline payload.
Zero means unconstructed, not the first alternative. Construct values with
runtime assignment before reading them. An invalid tag, including an active
nested variant's tag, invokes Error(100) on Atari before any arm or value copy
is exposed. Even ELSE does not catch invalid storage.

Scalars, records, variants and data pointers can be payloads. Record payloads
may contain embedded fixed arrays. Inline values copy by value; pointers share
their pointees. Recursive definitions must cross a pointer boundary:

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

Record and variant parameters and function results are supported on direct and
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
LET annotations, payload types, layout queries, constructors and flat patterns.
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

Not yet supported: generic routines or omitted/inferred type arguments,
nested patterns, guards, direct ARRAY payload declarations, volatile variants,
absolute/alias-backed variant declarations or raw/static variant initializers.
Use a record payload to contain an inline array. Typed pointers into explicitly
managed memory are available, but validation does not make arbitrary pointers
safe.
