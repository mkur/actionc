# Codegen Profiles

`actionc` has two profiles:

- `legacy`, also accepted as `compat`, is the default profile.
- `modern` is an opt-in profile for source and code generation choices that do
  not need to preserve the original compiler's byte shape.

Profiles are separate from backends. The default backend is `classic`, the
original AST code generator. `mir6502` is the newer MIR6502 backend; it is the
future direction for `actionc`, but it is experimental today. The old backend
spelling `legacy` is still accepted as an alias for `classic`.

Supported combinations:

- `--profile legacy --backend classic`: default compatibility path.
- `--profile modern --backend classic`: optimized mature-output path.
- `--profile modern --backend mir6502`: experimental MIR6502 path.

`--profile legacy --backend mir6502` is intentionally rejected. The legacy
profile is about compatibility-oriented classic code generation, while MIR6502
is an experimental modern backend path.

## Legacy Profile

The legacy profile should stay close to the original Action! compiler.
Compatibility fixes may change it, but new optimizations should not.

Compatibility is a goal, not a guarantee. The new compiler accepts many
original Action! idioms in this profile, including some undocumented pointer and
routine-address patterns. That is the main source-surface difference from
`modern`: legacy tolerates more old code, while modern asks source to make
ambiguous pointer and callable intent explicit. Old source code can still depend
on parser quirks, cartridge library behavior, memory layout, or machine-code
side effects that are not yet fully reproduced. When preserving old source
matters, keep the original source and libraries available as fixtures and add a
focused compatibility test or probe before changing semantics or codegen.

The legacy profile intentionally rejects some expression shapes that are legal
to parse but awkward to lower compatibly:

- function calls as arguments in standalone routine-call statements;
- most function calls inside arithmetic expressions;
- compound assignments where the target contains a function call;
- indexed assignments where both sides contain function calls.

Those restrictions keep the compatibility path conservative and make accidental
divergence easier to spot.

## Modern Profile

The modern profile keeps Action! source semantics, but it may move away from
the original compiler's physical layout, temporary placement, and instruction
sequence when the observable behavior is preserved. For generated programs,
`--profile modern --backend classic` is the recommended optimized path and will
generally produce the smallest binaries.

Modern is also the profile used by maintained modernized sources under
`samples/`, such as Toolkit and TN ports.

## Source Surface Changes

Most syntax extensions documented in
[SYNTAX_EXTENSIONS.md](SYNTAX_EXTENSIONS.md) are accepted by `actionc` in both
profiles. They include typed casts, explicit address values, function pointer
declarations, and machine-block label-byte syntax. They are not the switch that
makes a source file "modern"; they are available so source can express intent
explicitly.

Explicit lexical `BEGIN`/`END` blocks and comparison-as-value expressions
require the modern profile. In compatibility source, `BEGIN` and `END` remain
ordinary identifier spellings and do not acquire cartridge token IDs.

Modern classic and MIR6502 materialize comparisons as BYTE 0/1 values, including
arguments, returns, indexes, composed expressions and scalar CONST definitions.
Widening zero-extends the result; operand promotion is unchanged. Value-context
AND/OR/XOR remain eager bitwise operators, not new short-circuit operators.
Compatibility accepts comparisons in conditional contexts, but rejects numeric
comparison values during semantic analysis with a modern-profile diagnostic.
See [comparison values](SYNTAX_EXTENSIONS.md#comparison-values).

Legacy accepts many old Action! idioms that depend on implicit address-taking or
loose routine-address handling. Modern prefers the explicit extension forms for
those cases.

Some old idioms are rejected in both profiles. For example, a plain `CARD`
value is not a typed pointer for arbitrary dereference or indexing (`p^` or
`p(0)`). Modernize those sites by declaring the intended pointer type, such as
`BYTE POINTER p`, or by casting an explicit address at a call boundary, such as
`BYTE POINTER(@menuData)`. The maintained Toolkit and TN samples use this style
for old menu/data-block patterns.

Modern also rejects direct retargeting of routine names:

```action
DrawMenu = OtherProc      ; rejected in modern profile
handler = @OtherProc     ; use an explicit function pointer instead
```

For new or modernized source, prefer the explicit extension forms:

- typed cast expressions such as `BYTE POINTER(expr)`;
- explicit address values such as `@buffer` and `@DrawMenu`;
- `PROC POINTER` and `FUNC POINTER` declarations;
- `<label` and `>label` inside machine blocks for low/high address bytes.

These forms make intent visible to the compiler instead of relying on loose
original-compiler typing or ambiguous machine-block operands.

## Layout Changes

The modern profile does not preserve every physical layout artifact of the
single-pass cartridge compiler. Current layout differences include:

- string literals used inside a routine can be pooled in routine hidden storage
  instead of being emitted inline behind a local jump;
- dynamic `FOR` end-bound caches can be allocated in routine hidden storage
  instead of being embedded at the loop site;
- routine entry can be emitted directly after its parameter/local storage when
  the routine does not need a patchable entry trampoline; this also applies to
  public routine addresses and descriptor-backed local arrays;
- an internal routine's one- or two-byte parameter storage can be omitted when
  proof-guided classic lowering consumes every incoming A/X byte without a
  physical storage reference;
- an extra final `RTS` can be removed when the preceding routine already ended
  with one.

Externally visible calls and public Action! ABI boundaries remain part of the
observable contract. The modern profile may use internal facts about registers
or temporaries, but generated code must still accept the public A/X argument
placement and materialize public return behavior where user code can observe
it. Direct entry changes only the physical `JMP`: calls, `@routine`,
machine-block routine addresses, and `RUNAD` still resolve to the stable
executable entry.

The classic backend's parameter-storage elision is deliberately narrower than
the public ABI. It applies only to modern, direct A/X parameter frames with at
most two bytes. The emitted body must contain no reference to the parameter
cells, no parameter address may escape, and the routine may not contain machine
blocks, effect annotations, current-location expressions, locals, or hidden
storage. Otherwise the normal parameter cells and entry stores remain.

Nested calls accepted inside function-value expressions use the same protected
argument staging in both classic profiles. Arguments are evaluated once, from
left to right, at the public ABI base and immediately saved on the stack. Only
after evaluation are they restored to their final argument slots and A/X/Y.
Using the ABI base avoids an overlapping word-return copy when a word argument
follows a byte argument. Call detection traverses casts as well as other
expression wrappers. This is a correctness contract, not a modern optimization;
the single-result forwarding shortcut remains modern-only. It does not relax
the legacy call-statement restrictions above or alter no-call staging paths.

Inferred accumulator return-byte facts require a proven equality at every
value return. Memory-content aliases use the processor tracker's equality
proof; matching `Unknown` descriptions never establish equal runtime bytes.
Known low/high-byte return forwarding remains available, while unproven
results are read from their public ABI slots.

## Codegen Optimizations

Modern-only optimizations are guarded by
`CodegenProfile::enables_modern_optimizations()` and are reported in map output
when they fire.

Current optimization categories include:

- removing redundant register reloads and constant stores;
- reusing already prepared pointer and effective-address values;
- lowering proven indexed byte-array access to direct 6502 addressing modes;
- lowering proved byte-indexed two-byte elements by keeping `2 * index` in
  `Y`, carrying the scale overflow into the pointer high byte, and using
  `(zp),Y`;
- removing unnecessary call-result materialization;
- removing call-argument stores or forwarding arguments through the stack;
- inverting short branches when that avoids extra jumps;
- turning suitable call/return sequences into tail calls;
- removing jumps to an immediately following `RTS`;
- removing proved-unused internal parameter cells and their ABI capture stores;
- preserving known call facts for later local lowering.

The processor-state tracker records known immediate values in `A`, `X`, and
`Y`. It is deliberately conservative: values are invalidated across calls,
jumps, label joins, stack pulls, and memory loads unless a local proof says the
value is still safe to use.

Indexed word loads must preserve their address pointer until both bytes have
been read. This includes arithmetic operands materialized into the same
zero-page pair used for addressing. The scaled and general effective-address
paths share an overlap-safe consumer: overlapping destinations stage the low
byte on the stack until the high-byte read completes, without borrowing `X` or
another scratch pair. Partial overlaps and fixed absolute aliases of zero-page
storage obey the same rule; non-overlapping loads require no staging.

Full word-valued pointer indexes use the same two-carry schedule as array
indexes in both classic profiles. The carry from doubling the index low byte
is saved across the base low-byte addition; that addition's carry feeds the
high-byte rotation, and the saved scale carry feeds the final high-byte add.
Neither carry may be discarded or substituted for the other. After index
evaluation, this address arithmetic preserves X/Y and balances its status-stack
save/restore with either scratch pointer pair. Index expressions retain their
existing evaluation and clobber contracts.

The general pointer-index fallback also retains its captured base across a
computed index. A different final index home is not a scratch-disjointness
proof: nested arithmetic, indirect loads, and calls can overwrite the base
pair while materializing the index. The existing materialization predicate
selects stack preservation around that evaluation. This keeps the original
base-before-index read order, including when a call changes the pointer
variable, and leaves constant and simple-scalar fast paths unchanged.

An indexed assignment must retain its selected destination across RHS
evaluation, including casts and runtime arithmetic without source-level calls.
Multiply, divide, remainder, and shifts may use `X` for operands or results;
the existing index-preserving assignment path stages the result before
restoring the destination index in both classic profiles.

Unknown operand values are not reusable identities, including when embedded in
logic or subtraction results. The tracker must not equate computations over
different indexed bytes just because both inputs are unknown. Destructive
accumulator operations also cannot retain an old-`A` identity, and subtraction
facts require an identified carry input (no borrow, or a tracked low-byte
comparison). Without these proofs, normal reloads remain.

Signed subtract-based comparisons must test N xor V, not N alone. Both classic
profiles correct overflow before the sign branch, including staged, indirect
and call-return operands. This is an intentional correctness divergence from
the original subtract/sign byte shape at signed-word limits.

## Proof-Guided Lowering

Some modern optimizations are backed by explicit proof records. The current
proof consumers include:

- `index-address`, used for byte-array/index addressing choices;
- `parameter-storage`, used to prove that direct A/X parameter bytes have no
  physical storage consumers or observable addresses;
- `value-availability`, used for call-result and scalar byte materialization.

Use `--emit-proofs` to see accepted proof-guided lowering events:

```sh
cargo run --bin actionc-emit -- \
  --profile modern \
  --backend classic \
  --emit-proofs path/to/source.act
```

Use `--emit-proof-attempts` when investigating why a proof-guided lowering did
not fire.

## Observing Differences

Compare source listings:

```sh
cargo run --bin actionc-emit -- \
  --profile legacy \
  --backend classic \
  --emit-source-listing samples/hello-world.act

cargo run --bin actionc-emit -- \
  --profile modern \
  --backend classic \
  --emit-source-listing samples/hello-world.act
```

Use `--emit-map` to include routine addresses, optimization records, and effect
summaries:

```sh
cargo run --bin actionc-emit -- \
  --profile modern \
  --backend classic \
  --emit-map samples/hello-world.act
```

For larger profile comparisons, use `actionc-compare`; archived snapshots such
as [TN_MODERN_GAP_SNAPSHOT.md](archive/snapshots/TN_MODERN_GAP_SNAPSHOT.md)
show the kind of size and optimization deltas it can capture.
