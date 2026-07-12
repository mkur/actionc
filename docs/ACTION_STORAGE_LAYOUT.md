# Action! Storage Layout Notes

This note is the quick compatibility anchor for Action! variable, pointer, and
array storage. It summarizes behavior observed from original-compiler probes and
toolkit captures. For deeper evidence, see
`surveys/probes/original-compiler/ABI.md` and the per-probe notes in
`surveys/probes/original-compiler/outputs/original/`.

## Scalar Storage

Plain scalar variables use little-endian storage:

| Type | Size | Storage |
| --- | ---: | --- |
| `BYTE` / `CHAR` | 1 | single byte |
| `CARD` / `INT` | 2 | low byte, then high byte |
| pointer types | 2 | low byte, then high byte |

Assignments and address constants are often emitted high-byte first by the
original compiler when it is storing a two-byte pointer or routine address, but
the memory representation remains low/high.

Absolute aliases such as `BYTE color=$2FD` or `CARD RTCLOK=$12` do not allocate
storage. The symbol names the existing address.

Symbolic scalar aliases preserve the backing address of their target, including
when the target itself is absolute-backed. For example, after placing an
unsized array at `$CB`, `BYTE low=line, high=line+1` names `$CB` and `$CC`; it
does not allocate two new initialized bytes.

One compatibility quirk from the toolkit: after an absolute `CARD` alias in a
comma-separated declaration, following `CARD` entries may use vector-like
placement. An initialized `CARD` first gets three bytes of padding and then its
normal two-byte scalar cell; following uninitialized `CARD` entries reserve
4-byte vector cells instead of normal 2-byte scalar cells. This showed up in
resident-library style vectors and layout-integration captures and is modeled
only for the compat path.

## Routine Storage

Routine parameters and locals live in the routine's storage area before the
routine body. This is why compatible routines usually need an entry trampoline:
calls enter after the storage bytes and jump over them to the executable body.

Examples:

```action
PROC P(BYTE b CARD c)
```

The low byte of the first argument arrives in `A`, the next byte in `X`, and
later bytes use the Action! argument area. The prologue stores those bytes into
the routine storage block.

Calls to current-location routines declared with `=*` are public Action ABI
boundaries. In addition to the register argument homes, their leading argument
bytes are observable at `$A0`, `$A1`, and `$A2`. This matters for hand-written
machine routines and high-level routines that intentionally name those ABI
locations as absolute aliases.

Local pointer variables use the same two-byte low/high layout as globals.

## Byte Arrays

Sized byte arrays up to 256 bytes are stored inline:

| Declaration | Storage |
| --- | --- |
| `BYTE ARRAY a(n)`, `n <= 256` | `n` inline bytes |
| `BYTE ARRAY a(n)`, `n >= 257` | 4-byte descriptor plus backing storage |
| `CHAR ARRAY a(n)` / `STRING a(n)` | `n` inline bytes |

Original Action! preserves declared length metadata inside inline byte-array
storage when bytes 2-3 exist:

```action
BYTE ARRAY b(4)
```

stores four inline bytes, with bytes 2-3 initialized to `$04,$00`. For
`BYTE ARRAY b(3)`, byte 2 is initialized to `$03`. This is compatibility
metadata, not the descriptor format used by non-byte arrays.

Initialized byte arrays and strings are also inline. They are not descriptor
backed unless they are declared as unsized pointer variables.

Dynamic indexing into inline byte arrays can use absolute indexed addressing
such as `LDA array,X` or `STA array,X`.

The exact inline threshold comes from `ARRTHB.COM`: global and local
`BYTE ARRAY(255)` and `BYTE ARRAY(256)` have inline-array vtype `$9A`, while
`BYTE ARRAY(257)` switches to descriptor/backing vtype `$92`. The broader
`ARRTHG.COM` and `ARRTHL.COM` captures confirm the same descriptor-backed shape
for 320, 384, 512, and larger byte arrays.

Current `actionc` compatibility status:

- `ARRTHL.COM` is byte-exact after matching the 256/257 threshold.
- `ARRTHB.COM` has the same load size and threshold layout. The remaining
  known difference is the first two bytes of the local inline `BYTE ARRAY(255)`
  and `BYTE ARRAY(256)` storage: original Action! leaves `$01,$32` residue
  there, while `actionc` zero-fills them. Bytes 2-3 still carry the declared
  length metadata.
- `ARRTHG.COM` has the same load size and marker/routine addresses. The
  remaining known difference is descriptor pointer words for large global byte
  arrays: original Action! chooses different unsaved backing addresses from its
  runtime code-pointer state, while `actionc` uses deterministic skipped backing
  ranges after the saved segment.

## Unsized Arrays

Unsized arrays are pointer variables:

| Declaration | Storage |
| --- | --- |
| `BYTE ARRAY a` | two-byte base pointer |
| `CARD ARRAY a` | two-byte base pointer |
| `INT ARRAY a` | two-byte base pointer |

If an unsized array has an address initializer, the pointer bytes are initialized
to that address:

```action
BYTE ARRAY screen=$580
```

stores `$80,$05` in the array variable.

Inside expressions, unsized arrays behave like pointer-backed arrays.

## Non-Byte Sized Arrays

Sized `CARD ARRAY` and `INT ARRAY` variables are descriptor-backed, not inline
element storage:

| Declaration | Storage |
| --- | --- |
| `CARD ARRAY a(n)` | 4-byte descriptor |
| `INT ARRAY a(n)` | 4-byte descriptor |

The descriptor format is:

| Descriptor bytes | Meaning |
| --- | --- |
| 0..1 | backing data pointer |
| 2..3 | backing byte size, when the original emits one |

For uninitialized sized non-byte arrays, the descriptor lives in normal storage
and the backing data is placed outside the ordinary saved code bytes, after the
load segment. The descriptor's pointer targets that backing area, and bytes 2-3
hold the backing byte size.

For initialized sized non-byte arrays observed in toolkit code, the original
emits the backing data inline before the descriptor, then emits a 4-byte
descriptor whose first word points back to that inline data. The second
descriptor word has been observed as `$0000` in this form. `TURTLE.ACT` depends
on this shape:

```action
CARD ARRAY TG_SinTab(91)=[ ... ]
```

is emitted as inline word data followed by:

```text
descriptor[0..1] = address of first table word
descriptor[2..3] = $0000
```

This was a recent source of address drift: emitting only a two-byte pointer made
every following routine start two bytes too early.

Initialized unsized non-byte arrays use the same inline backing data, but the
compiler's code pointer only reserves the first descriptor word. The following
storage may overlap the descriptor's unused `$0000` size word. `PMG.ACT` shows
this with consecutive arrays:

```action
CARD ARRAY PM_BSize=[0 $100 $80],
           PM_Waste=[0 768 384]
```

`PM_BSize` points at its inline backing data, and `PM_Waste` begins where
`PM_BSize`'s unused size word would otherwise be.

For routine-local initialized unsized non-byte arrays, a declaration with
multiple initialized entries keeps the original two-byte pad before the first
backing block. A single initialized local non-byte array does not get that pad;
`layout_integration.act` confirms `CARD ARRAY localWords=[...]` places backing
data immediately after the preceding local storage.

Dynamic indexing into descriptor-backed arrays loads/builds an element address in
zero page and then uses `($xx),Y`.

## Array Parameters

Array parameters are passed as two-byte base pointers, not as full descriptors.

When passing array arguments:

| Argument form | Passed value |
| --- | --- |
| sized byte array | inline storage base address |
| sized non-byte array | backing data pointer from descriptor bytes 0..1 |
| unsized array | stored two-byte pointer |

Inside the callee, array parameters behave as pointer-backed arrays.

Observed argument byte order for:

```action
PROC Touch(BYTE ARRAY bp, CARD ARRAY cp, BYTE i)
```

is:

| Parameter byte | Location |
| --- | --- |
| `bp` low | `A` |
| `bp` high | `X` |
| `cp` low | `Y` |
| `cp` high | `$A3` |
| `i` | `$A4` |

## Address Temporaries

The original compiler mostly uses these zero-page pairs for array and pointer
address work:

| Zero page | Role |
| --- | --- |
| `$AE/$AF` | array/pointer address temporary |
| `$AC/$AD` | alternate element address temporary |
| `$84/$85` | right operand for runtime arithmetic helpers |

Common shapes:

- byte-array address: `base + index`
- word-array address: `base + index * 2`
- word-array expression index: compute index in `$AE/$AF`, then build element
  pointer in `$AC/$AD`
- runtime multiply: right operand goes in `$84/$85`, left operand in `A/X`

`TURTLE.ACT` exposed two important compat shapes:

- `TG_SinTab(90-theta)` keeps the word expression index in `$AE/$AF` and builds
  the final element pointer in `$AC/$AD`.
- `length * TG_ICos(...)` stores the function result directly from `$A0/$A1`
  into `$84/$85` before calling the multiply helper.

## Local Non-Byte Arrays

Local `CARD ARRAY(n)` and `INT ARRAY(n)` use the same 4-byte descriptor form as
globals. Their descriptors live in the routine storage block. Backing storage is
assigned after the saved segment; probes show local backing areas assigned in
reverse declaration order while descriptors stay in declaration order.

Local initialized unsized `CARD ARRAY`/`INT ARRAY` values use inline backing data
and two-byte pointer cells like globals. `PMG.ACT` shows an extra two-byte zero
pad before the first such local backing block; subsequent initialized unsized
arrays can overlap the previous pointer cell's unused high word in the same way
as globals.

Local dynamic byte-array indexing can use absolute indexed addressing. Large
uninitialized local byte arrays, observed at TN scale, are represented with a
4-byte descriptor plus skipped backing storage outside the saved load bytes;
this keeps the object file from carrying kilobytes of zero-filled local buffers.
Local dynamic non-byte-array indexing follows the descriptor-backed path through
zero-page pointers and `($xx),Y`.

## Compatibility Guardrails

When changing storage or codegen, check these before trusting size deltas:

- A two-byte drift before the first routine often means a descriptor was emitted
  as a pointer, or vice versa.
- Byte arrays and non-byte arrays intentionally use different storage models.
- Initialized non-byte arrays are a special case: inline backing data plus a
  4-byte descriptor.
- Array parameters receive base pointers, never the full 4-byte descriptor.
- Existing toolkit exact matches are sensitive to zero-page pair choice,
  especially `$AE/$AF` versus `$AC/$AD`.
