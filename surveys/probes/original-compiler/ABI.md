# Action! ABI Notes

This note collects ABI behavior observed from the original compiler probes and
manual discussion. Treat it as the compatibility baseline for `actionc` codegen.

## Routine Layout

Original Action! emits each user routine in this broad shape:

1. routine parameter and local storage bytes,
2. a public entry trampoline, usually `JMP body`,
3. the routine body.

Calls target the trampoline address, not the body address. `RUNAD` likewise
points at `Main`'s trampoline when a `Main` routine is present.

Storage bytes in original saved load files come from live memory and may contain
ambient values. `actionc` emits deterministic zero-filled storage instead.

Confirmed probes:

- `FUNC.COM`
- `NESTED.COM`
- `ARRPAR.COM`

## Argument Byte Order

Arguments are flattened into bytes from left to right. Multi-byte scalar values
are passed little-endian:

- byte 0: low byte,
- byte 1: high byte.

The first argument bytes are placed in fixed ABI locations:

| Argument byte offset | Location |
| --- | --- |
| 0 | `A` |
| 1 | `X` |
| 2 | `Y` |
| 3 | `$A3` |
| 4 | `$A4` |
| n | `$A0+n` |

There are no caller-side `$A0`, `$A1`, or `$A2` copies of the first three
argument bytes. `A`, `X`, and `Y` replace those memory positions; the fixed
argument area begins with byte offset 3 at `$A3`. Declaring a routine at the
current code location with `=*` changes its entry/layout contract, not this
argument placement.

An instruction such as `STA $A0`, `STX $A1`, or `STY $A2` at the beginning of
a machine-code routine is authored callee code that saves a register for its
own use. It is not paired with an implicit caller mirror. `STRNAM.COM` makes
this explicit: callers pass `Open` and `Xio` bytes 0-2 in A/X/Y, while those
routines themselves execute `STX $A1` before a nested call.

So a routine with three `BYTE` arguments receives:

| Parameter | Incoming byte | Location |
| --- | --- | --- |
| arg 1 | byte 0 | `A` |
| arg 2 | byte 1 | `X` |
| arg 3 | byte 2 | `Y` |

For `PROC P(BYTE a, CARD b, BYTE c)`, the incoming bytes are:

| Parameter byte | Location |
| --- | --- |
| `a` | `A` |
| `b` low | `X` |
| `b` high | `Y` |
| `c` | `$A3` |

Calls may omit trailing arguments. `actionc` emits only the bytes belonging to
the supplied argument prefix. Omitted argument bytes are not zero-filled; the
callee prologue still copies the declared parameter frame from the ABI
locations, so omitted bytes retain whatever is already in the corresponding
registers or `$A0+n` locations. This matches the low-level calling convention
and avoids inventing default values not represented in the original ABI.

## Fixed-Address Calls

External fixed-address declarations use the same caller-side byte ABI as user
routines, but the call target is the declared absolute address:

```action
PROC Sys=$1234(BYTE a CARD w BYTE b BYTE POINTER p)
```

For `Sys($11,$2233,$44,@gb)`, the caller places:

| Argument byte | Location |
| --- | --- |
| `$11` | `A` |
| `$33` (`w` low) | `X` |
| `$22` (`w` high) | `Y` |
| `$44` | `$A3` |
| `@gb` low | `$A4` |
| `@gb` high | `$A5` |

and emits `JSR $1234`. A declaration with an empty machine-body marker, such as
`PROC Sys=$1234(BYTE a) []`, is treated as an external declaration and does not
emit a local routine body.

## Callee Parameter Frame

The callee copies incoming ABI bytes into its own parameter storage before using
the parameters.

For parameter frames of 3 or more bytes, original Action! uses an `SArgs`-style
runtime helper plus inline frame metadata. The `ARGTHR.COM` probe confirms the
threshold for `FUNC`s: 2-byte frames use direct stores, while 3-byte frames use
`SArgs`. The `SARGS.COM` probe confirms the same behavior for `PROC
B3(BYTE a,b,c)`, and `RETURNS.COM` observed it for `CARD FUNC AddC(CARD a,b)`.

The `RETURNS.COM` probe also shows direct callee stores for smaller function
frames:

| Routine | Arg bytes | Prologue |
| --- | --- | --- |
| `BYTE FUNC IncB(BYTE x)` | 1 | direct `STA param` |
| `BYTE FUNC TwoBytes(BYTE a,b)` | 2 | direct `STX`, `STA` |
| `CARD FUNC OneCard(CARD a)` | 2 | direct `STX high`, `STA low` |
| `INT FUNC NegI(INT x)` | 2 | direct `STX high`, `STA low` |
| `BYTE FUNC ThreeBytes(BYTE a,b,c)` | 3 | `SArgs` metadata |
| `CARD FUNC ByteCard(BYTE a,CARD b)` | 3 | `SArgs` metadata |
| `CARD FUNC CardByte(CARD a,BYTE b)` | 3 | `SArgs` metadata |
| `CARD FUNC AddC(CARD a,b)` | 4 | `SArgs` metadata |

Observed prologue shape:

```text
JSR $A0F5
.BYTE <frame-base-low>, <frame-base-high>, <arg-byte-count-minus-one>
```

The body starts after those three metadata bytes. The parameter frame lives
before the trampoline.

Current metadata examples:

| Routine | Frame base | Arg bytes | Metadata |
| --- | --- | --- | --- |
| `B3(BYTE a,b,c)` | `$3012` | 3 | `$12 $30 $02` |
| `B4(BYTE a,b,c,d)` | `$3031` | 4 | `$31 $30 $03` |
| `C2(CARD a,b)` | `$3057` | 4 | `$57 $30 $03` |
| `Mix(BYTE a,CARD b,BYTE c)` | `$307D` | 4 | `$7D $30 $03` |
| `Ptrs(CHAR POINTER p,CARD POINTER q)` | `$30A3` | 4 | `$A3 $30 $03` |
| `ThreeBytes(BYTE a,b,c)` | `$3019` | 3 | `$19 $30 $02` |
| `ByteCard(BYTE a,CARD b)` | `$3041` | 3 | `$41 $30 $02` |
| `CardByte(CARD a,BYTE b)` | `$3058` | 3 | `$58 $30 $02` |

`actionc` now mirrors this boundary in compatible code generation: one- and
two-byte parameter frames use direct stores, while frames of three or more bytes
emit the `SArgs` prologue shape.

Implementation caution: `src/codegen.rs` models `$04E4..$04EE` as compile-time
helper vector slots. The runtime library initializes those slots via directives
such as `SET $4EE=r_Par`; original cartridge probe output calls
cartridge-resident helper entry points such as `$A0F5` because that is the
cartridge environment's initialized `SArgs` target. `actionc` resolves numeric
slot updates, named fixed-address routine updates, and label-valued updates that
point at routines generated in the same program. Compiling the full standalone
runtime library still needs support for Action!'s `PROC name=*()` current-code
routine form.

Current target decision: cartridge-compatible generation is the active model.
The default helper calls use the cartridge-initialized vector contents observed
from probes, not the `$04E4..$04EE` slot addresses themselves.

See `RUNTIME_HELPERS.md` for the standalone runtime `r_Par` implementation,
which backs the `$04EE` helper slot and uses the same three-byte metadata shape.

## Machine Blocks

`STRNAM.COM` confirms that machine block operands are context-sensitive:

- numeric byte tokens remain bytes, so split absolute operands such as
  `$20 $56 $E4` emit `20 56 E4`;
- numeric values larger than `$FF` emit little-endian words, so `$9D $344`
  emits `9D 44 03`;
- symbol operands follow the preceding opcode width, so `$A5 r+1` emits a
  one-byte zero-page operand while `$20 _StrNam` emits a two-byte routine
  address. This includes store-index opcodes such as `$86 name` (`STX zp`) and
  `$84 name` (`STY zp`), which TN uses inside current-location machine-block
  routines.

Fixed-address scalar aliases below `$0100`, such as `CARD r=$86`, are
zero-page slots for code generation. Their address is still usable through
address-of forms such as `@zx`.

`PREZP.COM` captures the TN library's pre-code zero-page setup. A `BYTE
POINTER` declared while CODE/CODEBASE point below the saved load segment is
used as low-memory zero-page working storage and does not appear in the saved
file; TN's `screen^` operations use `(E6),Y` directly. A pre-code `CARD
POINTER` declared the same way is still addressed through the low-memory CODE
cursor (`$E8/$E9` in the probe), but also reserves two bytes in the later saved
output once CODE/CODEBASE are moved there. The first current-location routine
therefore starts two bytes after the origin, while statements such as
`allocp==+2` still compile to zero-page loads/stores against `$E8/$E9`.

`EMPTYPR.COM` confirms that, in compatible code generation, a bodyless `PROC`
emits its public trampoline but no implicit `RTS` body byte. Control therefore
falls through to the following routine body. TN relies on this shape with
`PROC Error()` immediately before `_Cio`.

`ARRFIX.COM` confirms fixed-address sized byte arrays use a four-byte
pointer-pair storage shape. For example, `BYTE ARRAY allocbuf($800)=$2000`
emits `$00 $20 $00 $20` before the following routine. Indexing loads the base
pointer from the first word; the duplicate second word is preserved for
original-compatible layout.

## Routine Locals

Routine-local storage is allocated in the same inline storage block as
parameters, immediately before the routine trampoline:

1. parameter bytes, left-to-right and low-byte-first,
2. local declarations, in declaration order,
3. public routine trampoline,
4. routine body.

`LOCALS.COM` confirms that `SArgs` metadata points at the first parameter byte
even when locals follow the parameters:

```text
ParamLocal storage:
  $3061      a
  $3062..63  w
  $3064      lb
  $3065..66  lw
  $3067      trampoline

ParamLocal prologue:
  JSR $A0F5
  .BYTE $61, $30, $02
```

Local pointer variables use the same two-byte low/high storage shape as globals.

Local array storage follows the same broad byte-array versus non-byte-array
split as globals:

| Declaration form | Routine storage |
| --- | --- |
| `BYTE ARRAY a(n)`, `n <= 256` | inline `n` bytes |
| `BYTE ARRAY a(n)`, `n >= 257` | 4-byte descriptor plus backing storage |
| `CARD ARRAY a(n)` / `INT ARRAY a(n)` | 4-byte descriptor |
| `BYTE ARRAY a` / `CARD ARRAY a` | two-byte pointer |

Sized `BYTE ARRAY` storage keeps the original declared length in storage bytes
2-3 when those bytes exist. For example, `BYTE ARRAY buf(3)` reserves three
inline bytes and initializes the third byte to `$03`; `BYTE ARRAY bytes(4)`
initializes bytes 2-3 to `$04,$00`. This applies to both global and
routine-local inline byte arrays. It is not an indexing descriptor for byte
arrays; it is preserved compatibility metadata that the original compiler emits
inside the inline storage itself.

`ARRTHB.COM` pins the original sized-byte-array boundary exactly: both global
and local `BYTE ARRAY(255)` and `BYTE ARRAY(256)` are inline vtype `$9A`, while
`BYTE ARRAY(257)` is descriptor/backing vtype `$92`. `ARRTHG.COM` and
`ARRTHL.COM` confirm that 320-byte and larger sized byte arrays keep that
descriptor/backing shape.

The current `actionc` compat implementation follows that threshold. `ARRTHL.COM`
is byte-exact. `ARRTHB.COM` still differs only in local inline metadata residue:
original Action! leaves `$01,$32` in bytes 0-1 of the local
`BYTE ARRAY(255)` and `BYTE ARRAY(256)` storage, while `actionc` zero-fills
those bytes and preserves the length metadata in bytes 2-3. `ARRTHG.COM` still
differs in descriptor pointer words for large globals because original Action!
selects different unsaved backing addresses than `actionc`'s deterministic
post-segment skipped ranges.

`LOCARR.COM` confirms local non-byte array descriptors in the routine storage
block, with backing pointers assigned after the code segment:

| Local | Descriptor | Pointer | Byte size |
| --- | --- | --- | --- |
| `words` as `CARD ARRAY(3)` | `$300A..$300D` | `$30D3` | `$0006` |
| `nums` as `INT ARRAY(2)` | `$300E..$3011` | `$30CF` | `$0004` |

The descriptor slots remain in declaration order, but the backing storage is
assigned after the saved load segment in reverse local declaration order. In the
probe, `nums` backing starts immediately after the segment at `$30CF`, and the
earlier `words` backing follows at `$30D3`.

Dynamic local inline byte-array indexing uses absolute indexed addressing
(`LDX index` / `LDA array,X` or `STA array,X`). Dynamic local non-byte array
indexing loads the descriptor pointer through `$AE/$AF` and then uses
`($AE),Y`.

## Return Values

Function return values are delivered through the same zero-page ABI area:

| Return type | Location |
| --- | --- |
| `BYTE` / `CHAR` | `$A0` |
| `CARD` / `INT` | `$A0` low, `$A1` high |

`RETURNS.COM` confirms that multi-byte return values use little-endian zero-page
layout, but original code often emits high-byte-first instruction order:

- `CARD FUNC RetC() RETURN($1234)` stores `$12` to `$A1`, then `$34` to `$A0`.
- `INT FUNC RetI() RETURN(-2)` stores `$FF` to `$A1`, then `$FE` to `$A0`.
- callers copy `$A1` to destination high byte before copying `$A0` to the low
  byte.

`RETFLOW.COM` confirms that original Action! emits result setup immediately
followed by `RTS` at each `RETURN` site. It does this for:

- early `RETURN(expr)` in a `FUNC`,
- `PROC RETURN` inside an `IF`,
- `RETURN(expr)` inside a `WHILE` loop.

Original code can still preserve unreachable joins or final returns after
exhaustive branches, so byte-for-byte matching may require keeping some of that
fallthrough structure rather than performing reachability cleanup.

## Conditions And Comparisons

`BOOLS.COM` confirms these branch shapes:

- byte `=` / `#` uses `EOR` and branches on zero/non-zero,
- unsigned `CARD <` / `>=` compares low byte with `CMP`, subtracts high byte
  with `SBC`, then branches on carry clear/set,
- signed `INT <` / `>=` uses the same two-byte subtract shape, then branches on
  negative/plus from the high-byte result,
- scalar `AND` / `OR` in conditions are bitwise expressions, materialized in
  `$AE`, then tested for non-zero.

`SIGNEDGE.COM` confirms that original Action! does not add sign-difference or
overflow handling for signed `INT` comparisons. As a result, sign-different edge
cases can disagree with mathematical signed ordering:

| Condition | Mathematical truth | Original outcome |
| --- | --- | --- |
| `32767 < -1` | false | true |
| `-32768 < 1` | true | false |
| `1 < -32768` | false | true |
| `32767 >= -1` | true | false |

`actionc` currently emits explicit sign-difference handling for these cases, so
this is a semantic improvement but not original-compatible byte behavior.

## Arithmetic Helpers

`ARITH.COM` confirms the arithmetic helper operand ABI:

| Value | Location |
| --- | --- |
| left operand low byte | `A` |
| left operand high byte | `X` |
| right operand low byte | `$84` |
| right operand high byte | `$85` |
| result low byte | `A` |
| result high byte | `X` |

The cartridge/original environment used these helper calls:

| Operation | Helper |
| --- | --- |
| `CARD LSH` | `$B5C0` |
| `CARD RSH` | `$A0E6` |
| signed multiply | `$A000` |
| signed divide | `$A090` |
| signed modulo | `$A0DE` |

Standalone runtime sources also define vector slots `$04E4..$04EC` for the same
helper family. See `RUNTIME_HELPERS.md` before changing codegen helper targets.

In this probe, original Action! calls shift helpers even for `CARD LSH 1` and
`CARD RSH 1`; `actionc` currently inlines single-bit card shifts.

## Array Parameters

Array parameters are passed as two-byte base pointers, not as full descriptors.

For:

```action
PROC Touch(BYTE ARRAY bp, CARD ARRAY cp, BYTE i)
```

the incoming bytes are:

| Parameter byte | Location |
| --- | --- |
| `bp` low | `A` |
| `bp` high | `X` |
| `cp` low | `Y` |
| `cp` high | `$A3` |
| `i` | `$A4` |

When passing array arguments:

- sized `BYTE ARRAY` arguments pass the inline storage base address,
- sized non-byte arrays pass the backing data pointer from their descriptor,
- unsized arrays pass the two-byte pointer stored in the array variable.

Inside the callee, array parameters behave like pointer-backed arrays.

## Array Address Temporaries

Original Action! uses `$AE/$AF` as the array element address temporary for
pointer-backed and descriptor-backed array access.

Observed lowering:

- `BYTE ARRAY` element address: base + index,
- `CARD ARRAY` / `INT ARRAY` element address: base + index * 2,
- element access uses `($AE),Y`.

For dynamic indexes into inline byte arrays, original Action! may use shorter
absolute indexed forms such as `LDA abs,X` and `STA abs,X`.

`LOCARR.COM` confirms the same split for local arrays: dynamic local byte-array
indexing can use absolute indexed `,X`, while dynamic local non-byte-array
indexing uses `$AE/$AF` and `($AE),Y`.

## Pointer Values And Dereference

Pointer values are two-byte addresses and use the same low-byte-first ABI when
passed as arguments.

Original Action! pointer dereference uses `$AE/$AF` as the temporary pointer
pair, then accesses `($AE),Y`.

Observed lowering from `POINTERS.COM`:

- `CHAR POINTER` dereference uses `Y=0`.
- `CARD POINTER` stores high byte first through `Y=1`, then low byte through
  `Y=0`.
- `CARD POINTER` loads high byte first into destination high byte, then low byte
  into destination low byte.
- Pointer assignment to a known address stores the high byte first, then low
  byte.
- `pointer ==+ 1` uses an `INC low` / carry-to-high peephole.

## Record Storage And Access

`RECORDS.COM` confirms basic `TYPE` record layout:

```action
TYPE Pair=[BYTE tag CARD word]
```

Fields are packed in declaration order without padding:

| Field | Offset | Size |
| --- | --- | --- |
| `tag` | 0 | 1 byte |
| `word` | 1 | 2 bytes |

A global record value reserves its packed byte size inline. In the probe,
`Pair rec` occupies `$3009..$300B`.

Passing a record value to a `TYPE POINTER` parameter passes the record base
address as a normal two-byte pointer, low byte in `A`, high byte in `X`.
Pointer-to-record field access computes `base + field_offset` into `$AE/$AF`
and uses `($AE),Y`. Multi-byte fields are stored little-endian, though the
original compiler may emit high-byte stores/loads first with `Y=1`, then low
byte with `Y=0`.

The manual grammar confirms that normal `TYPE` record fields are fundamental
variable declarations only. Pointer fields, array fields, and arrays of records
are not direct language forms; the manual recommends "virtual record" layouts
over byte arrays for those cases. This explains the rejected probe variant
`TYPE Pair=[BYTE tag CARD word CHAR POINTER ptr]`.

## Global Array Storage Forms

Original compatible storage forms are:

| Declaration form | Storage |
| --- | --- |
| `BYTE ARRAY a(n)` | inline `n` bytes |
| `CARD ARRAY a(n)` / `INT ARRAY a(n)` | 4-byte descriptor plus post-code backing storage when uninitialized |
| `CARD ARRAY a=[...]` / `INT ARRAY a=[...]` | inline backing data plus two-byte pointer cell |
| `BYTE ARRAY a` / `CARD ARRAY a` | two-byte pointer |

Descriptor-backed arrays store:

| Descriptor bytes | Meaning |
| --- | --- |
| 0..1 | backing data pointer |
| 2..3 | backing byte size for uninitialized sized arrays; `$0000` observed for initialized sized non-byte arrays with inline backing data; unsized initialized non-byte arrays do not reserve this word and following storage may overlap it |

See `../../docs/ACTION_STORAGE_LAYOUT.md` for the shorter compatibility anchor.

## String Storage

`STRING` is commonly defined as:

```action
DEFINE STRING="CHAR ARRAY"
```

`STRINIT.COM` confirms that string initializers for `CHAR ARRAY` storage are
length-prefixed:

| Declaration | Stored bytes |
| --- | --- |
| `STRING empty(0)=""` | `$00` |
| `STRING one(0)="A"` | `$01 $41` |
| `STRING hello(0)="HELLO"` | `$05 "HELLO"` |
| `STRING quoted(0)="A""B"` | `$03 "A\"B"` |
| `CHAR ARRAY fixed(6)="ATARI!"` | `$06 "ATARI!"` |

Element index 0 is the length byte. Character data starts at index 1. `(0)` with
a string initializer auto-sizes to the length byte plus the string bytes.
For a sized `CHAR ARRAY` string initializer, the declared size is the character
count/capacity and storage still includes the extra length byte; for example,
`CHAR ARRAY raw(3)="XYZ"` stores `$03 "XYZ"` in four bytes.

`STRLIT.COM` confirms that string literals can be passed directly as
`STRING`/`CHAR ARRAY` arguments. The literal is emitted as length-prefixed
storage inline in the routine body. The compiler jumps over the literal before
continuing with executable code:

```text
$3036: JMP $303C
$3039: $02 "HI"
$303C: LDX #$30
$303E: LDA #$39
$3040: JSR Take
```

The caller passes the address of the literal length byte (`$3039`).

`STRPASS.COM` confirms that named `STRING` values and named `CHAR ARRAY`
values use the same pointer ABI at call boundaries. `Take(hello, 4)` passes
`hello` as `A=$00`, `X=$30`, with the byte index in `Y`; because the full
argument frame is three bytes, the callee uses the `SArgs` helper to store
`s` at `$3010..$3011` and `i` at `$3012`. `TakeRaw(raw)` passes `raw` as
`A=$06`, `X=$30` and uses the direct two-byte prologue.

Indexed reads through string/character-array parameters use `$AE/$AF` as the
computed effective address and `LDA ($AE),Y` with `Y=0`. Dynamic byte indexing
adds the index to the pointer low byte and carries into the high byte.

`STRMUT.COM` confirms that initialized `STRING` storage is mutable in-place.
For global storage and constant indexes, string element reads/writes lower like
ordinary `BYTE ARRAY` accesses: `text(1)='Z` emits `LDA #$5A; STA $3001`.
Index 0 remains the length byte, so `bytes(0) = text(0)` copies the string
length.

Bare assignment to an unsized array variable updates the stored two-byte base
pointer. Assignment from a named array/string stores that source base address.
Assignment from a string literal emits fresh length-prefixed inline storage in
the routine body, jumps over it, and stores the literal address into the array
pointer. This differs from indexed assignment, which mutates the current
storage bytes.

`STRLOC.COM` confirms that local initialized `STRING` and initialized
`CHAR ARRAY` values are fixed load-segment storage, not stack storage. They are
emitted immediately before the owning routine trampoline and constant indexed
accesses use direct absolute addresses.

## Probe References

- `outputs/original/FUNC.notes.md`: function layout and return basics.
- `outputs/original/ABICALLS.notes.md`: origin capture caveat for ABI probes.
- `outputs/original/NESTED.notes.md`: nested calls and trampoline targets.
- `outputs/original/ARRPAR.notes.md`: array parameter ABI and `SArgs` usage.
- `outputs/original/SARGS.notes.md`: `SArgs` prologue boundary and metadata.
- `outputs/original/ARGTHR.notes.md`: direct versus `SArgs` parameter-frame
  threshold.
- `outputs/original/LOCALS.notes.md`: parameter/local routine storage layout.
- `outputs/original/LOCARR.notes.md`: local byte and non-byte array storage
  layout.
- `outputs/original/RETURNS.notes.md`: function return ABI and small argument
  prologue behavior.
- `outputs/original/RETFLOW.notes.md`: early return and per-site `RTS` behavior.
- `outputs/original/BOOLS.notes.md`: comparison and bitwise condition branch
  shapes.
- `outputs/original/SIGNEDGE.notes.md`: signed comparison overflow behavior.
- `outputs/original/ARITH.notes.md`: arithmetic instruction shape and cartridge
  helper addresses.
- `outputs/original/RECORDS.notes.md`: packed record layout and
  pointer-to-record field access.
- `MANUAL_CROSSCHECK.md`: cross-check of probe findings against the official
  manual and errata.
- `outputs/original/STRINIT.notes.md`: length-prefixed string initializer
  storage.
- `outputs/original/STRPASS.notes.md`: named string/character-array parameter
  passing and indexed reads.
- `outputs/original/STRMUT.notes.md`: mutable string storage and constant
  indexed reads/writes.
- `outputs/original/STRLOC.notes.md`: local initialized string storage layout.
- `outputs/original/STRLIT.notes.md`: string literal argument storage and
  passing.
- `RUNTIME_HELPERS.md`: arithmetic, shift, modulo, and `r_Par` helper analysis.
- `outputs/original/ARRAYS.notes.md`: sized array storage and array indexing.
- `outputs/original/ARRREF.notes.md`: unsized array pointer storage.
