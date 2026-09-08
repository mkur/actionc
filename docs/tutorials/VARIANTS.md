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

Record and variant parameters and function results are supported on direct
compiler-defined calls in the modern profile. Parameters are independent mutable
copies; use an explicit POINTER parameter to modify the caller's object. Function
results compose with LET, assignment, constructor arguments, RETURN and CASE.
Arguments are captured once, left to right, before later argument effects.
Aggregate calls require all arguments, including trailing scalar arguments.
Foreign/cartridge entry signatures are not inferred from these source types.

Not yet supported: typed indirect aggregate calls, generic types,
nested patterns, guards, direct ARRAY payload declarations, volatile variants,
absolute/alias-backed variant declarations or raw/static variant initializers.
Use a record payload to contain an inline array. Typed pointers into explicitly
managed memory are available, but validation does not make arbitrary pointers
safe.
