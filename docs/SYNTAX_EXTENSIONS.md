# actionc Syntax Extensions

This note tracks source syntax that is intentionally accepted by `actionc` but
is not part of strict Action! compatibility. Unless marked modern-only, these
forms are accepted in both `legacy` and `modern` profiles where the owning
backend supports them. They make
source intent explicit without relying on the original compiler's loose typing
behavior. Legacy code may still use many old implicit idioms; modernized code
should prefer these explicit forms, and the modern profile requires them for
some ambiguous routine-address cases.

## Contents

- [32-bit Integers](#32-bit-integers)
- [Compile-Time Constants](#compile-time-constants)
- [Immutable Runtime Bindings](#immutable-runtime-bindings)
- [Comparison Values](#comparison-values)
- [BYTE Enums](#byte-enums)
- [CASE Statements](#case-statements)
- [Variants and Generic Types](#variants-and-generic-types)
- [Fixed-Length Arrays Inside Records](#fixed-length-arrays-inside-records)
- [Volatile Storage](#volatile-storage)
- [ATASCII And Screen-Code Escapes](#atascii-and-screen-code-escapes)
- [Typed Cast Expressions](#typed-cast-expressions)
- [Explicit Address Values](#explicit-address-values)
- [Plain CARD Values Are Not Typed Pointers](#plain-card-values-are-not-typed-pointers)
- [Function Pointers](#function-pointers)
- [INLINE Routines](#inline-routines)
- [Machine Block Label Bytes](#machine-block-label-bytes)
- [Relocatable Static Initializers](#relocatable-static-initializers)
- [MADS-Style Inline Assembler](#mads-style-inline-assembler)
- [Explicit Lexical Blocks](#explicit-lexical-blocks)
- [Compatibility Policy](#compatibility-policy)

## INLINE Routines

`INLINE` requests inlining of a source routine:

```action
INLINE BYTE FUNC Mix(BYTE left,right)
RETURN((left LSH 1) XOR right)
```

Named modules also accept `PUBLIC INLINE PROC` and `PUBLIC INLINE <type> FUNC`.
The modifier is case-insensitive and contextual: variables, routines and
qualified members named `INLINE` remain legal outside modifier position.
Duplicate modifiers, non-routine declarations and `INLINE EXTERNAL` are errors.

All compiler modes accept the preference. It changes no callable identity, ABI,
evaluation order, persistent parameter/local storage or public return slots.
Classic modes retain ordinary calls. MIR6502 owns optimization decisions;
retaining a call is always permitted when legality or cost cannot be proved.
Modern MIR6502 prioritizes requested candidates and allows bounded additional
growth. `ACTIONC_MIR6502_PEEPHOLES=sites` explains applied and declined requests
through the existing optimization report; ordinary builds remain quiet.
Automatic inlining does not require an annotation.

Requested candidates support at most two logical integer parameters (BYTE,
CHAR, INT, CARD, LONGINT or LONGCARD), eight acyclic blocks and 128 MIR
operations. Private scalar scratch must be proven safe to promote. Straight-line
wrappers can retain compiler-owned wide arithmetic helpers. Pointer/aggregate
parameters, recursion and ordinary nested calls remain unsupported. Expansion
must also fit code-growth and trial budgets. See the
[implementation plan](INLINE_IMPLEMENTATION_PLAN.md) and
[Q4.12 validation](INLINE_Q4_12_VALIDATION.md) for the current limits and results.

## 32-bit Integers

`LONGINT` is signed 32-bit (-2147483648..2147483647); `LONGCARD` is unsigned
32-bit (0..4294967295). Classic (Compatibility and Optimized) and MIR6502
generate executable Atari code for both, with cartridge-linked or standalone
runtime. Native 68k/65816 support remains typed
lowering/ABI validation, not executable backend support.

`INT` and `CARD` stay 16-bit on every target. A wide operand or cast before an
operation selects wide computation; a wide assignment destination alone does
not widen an already-narrow expression:

```action
CARD a=[$FFFF], b=[1]
LONGCARD narrow, wide
PROC Main()
  narrow=a+b            ; 0: the CARD addition wraps first
  wide=LONGCARD(a)+b     ; 65536: addition is 32-bit
RETURN
```

Conversions sign-extend signed sources, zero-extend unsigned sources, and
truncate to low bits when narrowing. LONGCARD dominates LONGINT in mixed wide
arithmetic; LONGINT dominates narrow integer operands. Nested narrow operations
stay narrow even inside a wide expression. Existing literal typing is unchanged:
decimal `65535` is INT -1, while `$FFFF` is CARD 65535. Larger decimal literals
infer LONGINT when representable, otherwise LONGCARD; wide hex uses LONGCARD.

Arithmetic wraps at its selected width. Division truncates toward zero;
remainder has the dividend's sign. MIN/-1 wraps to MIN. Runtime division or
remainder by zero invokes Error(101), and does not resume the failed operation.
LSH/RSH are logical shifts; counts at least the operand width produce zero.

Wide types work in variables, initializers, arrays, embedded record arrays,
pointer elements, parameters, FUNC results and typed FUNC POINTER signatures.
Existing library routines keep their declared BYTE/CARD/INT interfaces.
Qualified `SYS.PrintLC`/`SYS.PrintLI` families and the corresponding string
conversion/input routines provide [32-bit decimal I/O](LONG_INTEGER_IO.md)
without widening cartridge runtime entry points or `PrintF` arguments.
Direct conversions between REAL and 32-bit integers remain unsupported.
Wide CASE uses 32-bit comparisons; BYTE/INT/CARD/enum CASE retains its own width.
Constant-step FOR loops retain direction and stop before wrapping past the
induction type's limit, including steps greater than 65535.

These names are contextual types, also available as `SYS.LONGINT` and
`SYS.LONGCARD`. `LONG`/`ULONG` are not built-in aliases; ordinary identifiers
with those names remain legal. See the
[integration and ABI contract](LONG_INTEGER_INTEGRATION_AND_MIR6502_PLAN.md).

## BYTE Enums

Modern classic and MIR6502 support nominal BYTE enums with both runtimes:

```action
TYPE ResultCode=ENUM [OK=0 FAILED=1 BUSY=10 RETRY]
CONST ResultCode Default=ResultCode.OK
ResultCode status=[Default]
ResultCode ARRAY codes(2)=[ResultCode.OK ResultCode.RETRY]

ResultCode FUNC TryStart(BYTE busy)
  IF busy THEN RETURN(ResultCode.BUSY) FI
RETURN(ResultCode.OK)
```

Members are qualified and case-insensitive. Commas are optional. The first
implicit value is zero; each next implicit value is the preceding value plus
one. Here RETRY is 11. `OK=0 FAILED=11 BUSY=10 RETRY` is an error because RETRY
duplicates 11; numbering does not skip occupied values. Duplicate names/values
and values outside 0..255 are errors. No representation annotation is supported.

Each enum is a distinct type. Assignment, arguments, returns, initializer leaves,
and comparisons require the same enum. `ResultCode(raw)` explicitly converts an
integer to the low byte; `BYTE(status)`, `CHAR(status)`, `CARD(status)`, and
`INT(status)` explicitly expose its unsigned representation. All 256 values are
defined, including unnamed values. There are no membership traps. Arithmetic,
truthiness, loop counters, and array indexes require an explicit integer bridge.

Enums work in globals, locals, arrays, record fields and embedded arrays,
pointers, parameters, FUNC results, and `ResultCode FUNC POINTER reader`.
Named ARRAY pointer-cell rebinding and advancement retain their existing pointer
meaning; embedded arrays cannot be rebound. Neither operation is enum arithmetic.
An array value is a pointer, not an enum element; use indexing or `array^` for an
element. Array/pointer compatibility retains the exact enum element identity.
Existing declaration rules remain: `status=[ResultCode.OK]` initializes a value;
`status=$0600` binds storage at an address. Partial arrays/records retain zero-fill
even when zero has no member name. Initializer lists accept members and enum
CONSTs; use a typed CONST for a computed enum initializer. Assembly/data numeric
interfaces use a numeric CONST such as `CONST Opcode=BYTE(ResultCode.OK)`.

TYPE declarations follow existing global/module/routine/BEGIN scopes. A PUBLIC
enum exports its type and members; `USE Library AS API` and `USE ALL FROM Library`
preserve its identity without importing bare member names. Compatibility rejects
ENUM. Wider enums, aliases with duplicate numeric values, reflection, and
parameterized callable-pointer syntax are not included.

See [the runnable state-machine sample](../samples/enum-case.act), which prints
`0, 1, 2, 3, 0` on separate lines:

```sh
cargo run --bin actionc -- --mode optimized --runtime standalone -o /tmp/enum-case.xex samples/enum-case.act
```

## CASE Statements

Modern classic and MIR6502 support integer and enum CASE with both cartridge-linked and
standalone runtimes:

```action
CASE key OF
WHEN 0 THEN
  HandleZero()
WHEN 1 TO 9, 13 THEN
  HandleCommand()
ELSE
  HandleOther()
ESAC
```

The selector can be BYTE, CHAR, CARD, INT, LONGINT, LONGCARD, or an enum and is
evaluated exactly once. Wide selectors require MIR6502 for Atari execution.
Labels are compile-time integer constants; inclusive ranges retain the
selector's signedness. Descending ranges, duplicates, and overlapping labels
are errors. Label values must fit without implicit truncation. Existing literal
typing applies: `$FFFF` or `CARD(65535)` denotes unsigned 65535, whereas decimal
`65535` alone is INT -1.

Enum selectors require constant labels of the exact same enum, such as
`WHEN ResultCode.OK, ResultCode.RETRY THEN`. Numeric labels, other enums, and
enum ranges are rejected. Named-member coverage does not eliminate the no-match
path, because unnamed byte representations remain valid.

Arms do not fall through. ELSE is optional, unique, and last; an unmatched CASE
without ELSE continues after ESAC. RETURN exits the routine; EXIT exits the
nearest enclosing loop. Arms do not introduce scopes: use BEGIN/END for local
declarations. A FUNC still needs a return path when no arm matches.

The CASE/OF header, each WHEN/THEN header, ELSE, and ESAC each occupy their own
physical source line. CASE, OF, WHEN, and ESAC remain contextual identifiers,
so calls such as `Case()` and `When()` remain legal. ENDCASE is not an alias.
Compatibility rejects CASE. Modern guards use `WHEN label IF condition THEN`:
match first, evaluate the guard once, and continue to the next arm if false.
The selector remains captured even if a guard changes its original storage.
Later arms may repeat earlier guarded labels, including an unguarded fallback.
Overlaps between unguarded arms and duplicate labels within one header remain
errors. A guarded interval may overlap earlier unguarded labels if some values
remain reachable. Guards do not establish exhaustiveness.
`WHEN _ IF condition THEN` is a guarded catch-all; bare WHEN _ and guarded ELSE
are not supported.

## Variants and Generic Types

Modern classic and MIR6502 support tagged nominal variants, including record,
union and variant payloads, immutable value snapshots, value parameters/results, nested
constructor/literal patterns and ordered guards. Both Atari runtimes are supported.

```action
TYPE Option<T>=VARIANT [NONE SOME [T value]]
TYPE Buffer<T>=[T ARRAY values(8)]
TYPE TreeOf<T>=VARIANT [EMPTY NODE [T value TreeOf<T> POINTER left,right]]
```

Use explicit applications such as `Option<BYTE>` and qualified constructors such
as `Option<BYTE>.SOME(7)`. For a named, non-generic variant, local
`USE ALL FROM MaybeByte` makes `NONE` and `SOME(n)` available in expressions and
CASE patterns from that statement to the end of its routine or BEGIN block.
Nested scopes inherit the opening; conflicting visible names are errors, and
qualified constructors remain available. Local USE does not open enums or modules
and does not accept AS or explicit generic arguments. Nullary constructors omit
parentheses. Each instance is nominal; equal layouts do not make types compatible.
Recursive data requires
explicit pointers and storage management; there is no hidden allocation or GC.
Variant tag zero is invalid, and a value read validates active inline variants
before exposing payloads. Malformed tags invoke Error(105) on Atari, including
before ELSE. Tags, padding and native layouts are not a portable serialization ABI.
For CASE over variants without inline nested variants, constructor dispatch
performs that validation: invalid tags reach the final unmatched fault path.
ELSE and wildcard guards accept only valid tags. Nested values retain early
validation before any pattern test, payload binding or user guard. See
[CASE validation lowering](VARIANT_CASE_VALIDATION.md).

See the [variant tutorial](tutorials/VARIANTS.md) for complete examples, immutable
binding restrictions, coverage rules, generic limits and the backend matrix.
Compatibility rejects these extensions. Native targets have verified type/layout
and ABI planning, but executable variants still require native Error adapters.

## Untagged Unions

Modern source may define `TYPE WordView=UNION [CARD word BYTE low]`.
Members share offset zero; grouped entries overlap too. Use a nested record for
sequential fields. Size is the maximum member extent rounded to target alignment.
There is no tag, active-member check or implicit clear on a member write.

Whole copies include padding and are overlap-safe. LET snapshots and aggregate
call boundaries follow ordinary value semantics. Generic definitions such as
`TYPE Overlay<T,U>=UNION [T first U second]` retain nominal instance identity.
Records, fixed arrays, enums, data pointers and nested unions are supported;
inline VARIANT, REAL and callable pointers are rejected transitively.
Positional/string initialization of union-containing storage is rejected.
Existing RAM bindings/aliases and object-level VOLATILE remain available.

Scalar arithmetic uses the selected member's ordinary type and width; whole
unions cannot be used as numbers, truth values or CASE selectors. Compatibility
rejects UNION definitions. See [the tutorial](tutorials/UNIONS.md) for byte order,
pointer sharing, initialization, backend limits and the runnable sample.

## Fixed-Length Arrays Inside Records

Modern classic and MIR6502 on Atari support arrays stored directly inside a
record, with either ActionCart or Standalone runtime:

```action
CONST Count=100
TYPE Buffers=[INT ARRAY x(Count),y(Count)]
Buffers data=[1 2 3],copy
INT POINTER first=data.x

PROC Main()
  data.y(99)=first(1)
  copy=data
RETURN
```

Bounds must be positive compile-time constants. Storage is inline: this example
has 200 bytes for `x`, then 200 for `y`, with no member-array descriptor.
BYTE, CHAR, INT, CARD, REAL, enum and complete supported record elements are covered.
Classic's existing restriction on pointer-valued record fields remains, including
arrays of pointers. Incomplete and recursive by-value layouts are rejected.

Direct, local, absolute, nested and pointer-based records support element access,
as do arrays of records (`rows(r).x(i)`). `SIZEOF(data.x)` is 200,
`ELEMENTS(data.x)` is 100, and layout queries do not execute their operands.
An exact element-pointer or matching array-parameter context permits field
decay; explicit address-of and pointer casts are also available. Member arrays
cannot be rebound or assigned as whole arrays.

Flat initializer lists visit fields and elements in declaration order and
zero-fill missing values. Address leaves such as `[@data.x(1)]` use the normal
relocation machinery. Static pointer initializers require known storage and
constant indexes; runtime pointer/index expressions belong in routine statements.
This extension does not add scalar non-pointer subobject-alias declarations.

Whole-record assignment copies every embedded byte, including records larger
than 255 bytes. Destination and source places are evaluated once, in that order;
self-copy and overlapping aliases preserve whole-value semantics. No runtime
bounds checks are added. Compatibility/legacy continues to reject array fields.

## Compile-Time Constants

`CONST` declares a typed scalar value evaluated by the compiler:

```action
CONST BYTE TOP_BLANK_ROWS=4
CONST BYTE FIRST_VISIBLE_VCOUNT=2+TOP_BLANK_ROWS
CONST CARD DISPLAY_LIST_A_BASE=$5000,
      DISPLAY_LIST_B_BASE=DISPLAY_LIST_A_BASE+$400

PROC Draw()
  CONST BYTE LAST_ROW=159
  BYTE row

  FOR row=0 TO LAST_ROW DO
    ; ...
  OD
RETURN
```

The scalar type is optional:

```text
CONST [BYTE|CHAR|CARD|INT] name=expression [, name=expression ...]
```

Without it, each entry's type is inferred using normal Action! expression
typing. With it, the declared type applies to every entry in that declaration
and has exactly the same wrapping and truncation behavior as an explicit cast.
For example, `CONST BYTE MASK=$1FF` is equivalent to
`CONST MASK=BYTE($1FF)` and produces `$FF`. Use separate declarations when
constants need different declared types.

Constants may be global or local to a routine. Names are case-insensitive and
use the ordinary local-before-global lookup order. Entries are evaluated from
left to right and may refer only to constants already visible at that point;
forward references are rejected.

Constant expressions support numeric and character literals, parentheses,
unary `+` and `-`, explicit `BYTE`, `CHAR`, `CARD`, and `INT` casts, and the
arithmetic and bitwise operators `+`, `-`, `*`, `/`, `MOD`, `LSH`, `RSH`,
`AND`, `OR`, and `XOR`. Calls, storage references, strings, addresses, and the
current-location `*` value are not constant expressions.
The modern profile also permits scalar comparisons and their composition in
CONST expressions; the result of each comparison is BYTE 0 or 1.

A constant has no address and allocates no storage. It works anywhere its
typed value is accepted, including array bounds, initializers, `SET`, fixed
routine addresses, loop bounds, and inline assembler operands. `CONST` is an
`actionc` extension supported by both compiler profiles and both backends; the
original Action! cartridge compiler does not recognize it.

`DEFINE` remains available for textual type aliases, directive macros, and
machine-byte macros. `CONST` does not change those expansion rules.

## Immutable Runtime Bindings

Modern source supports Rust/OCaml-style immutable local bindings:

```action
LET count=ReadCount()
LET count=count+1
LET CARD limit=ReadLimit()
```

The syntax is `LET [type] name=expression`, with one binding and a required
initializer. Each initializer executes once whenever control reaches that
statement, including on each loop iteration or routine invocation. The second
`count` is a new binding whose initializer reads the first; it is not an
assignment. `CONST` still means a compile-time value, and ordinary typed variable
declarations remain mutable. LET is not a static storage initializer.

Without an annotation the binding retains the expression's canonical type.
An annotation applies ordinary assignment conversions, including integer
narrowing, without widening intermediate calculations. For CARD operands,
`LET LONGCARD result=a*b` wraps the product at 16 bits before conversion;
`LET result=LONGCARD(a)*b` computes a 32-bit product.

Place LET directly in a routine or an explicit `BEGIN`/`END` statement list,
before or after other executable statements. IF/CASE arms and loop bodies
require an explicit block:

```action
FOR i=0 TO 9 DO
  BEGIN
    LET current=ReadCount()
    PrintBE(current)
  END
OD
```

The name becomes visible after its initializer until the end of its containing
block or routine. Sequential shadowing is legal, including parameters, ordinary
variables, types, and module aliases. Earlier references retain their original
meaning; leaving an explicit block restores the outer bindings. Ordinary
same-scope duplicate declarations are still errors.

Assignment, compound assignment, and using a binding as a FOR counter are
errors. Initially, taking its storage address, making a static storage alias,
or naming its home in machine code/inline ASM is also rejected. A pointer
binding is immutable, not its pointee: `LET p=@value` permits `p^=2`, but not
`p=@other`. A volatile initializer captures one value; reading the binding does
not reread the hardware. Unused bindings do not discard initializer effects.

Supported values are scalar integers, enums, native REAL, typed data pointers,
and typed callable pointers. Use `@Routine` to form a callable value. Array
values require an explicit pointer annotation or cast; LET does not own an
array. Complete supported records and variants can be immutable snapshots,
including inline arrays; their storage cannot be exposed through addresses or
aliases. Explicit pointer fields retain mutable pointees. Enum identity, nominal
aggregate types and callable signatures remain checked. Runtime
LET values cannot be CONST expressions, CASE labels, static initializers, or
array bounds, even when initialized with literals. Unevaluated layout queries
such as `SIZEOF(binding)` remain compile-time values.

Classic and MIR6502 support LET with cartridge-linked or standalone runtime,
including LONGINT/LONGCARD values. Typed indirect calls retain their declared
argument signatures, and native REAL uses the Atari OS floating-point package. LET does not add stack locals or
reentrancy to Atari routine storage. Native 68k/65816 have lowering/ABI checks,
not execution claims.
Global bindings, `LET MUT`, deferred initialization, destructuring, and
expression-form `LET ... IN` are not supported. LET is contextual: ordinary
identifiers such as `Let()` and `let=1` remain valid. The legacy profile rejects
LET bindings, independently of the selected runtime.

See [samples/let-bindings.act](../samples/let-bindings.act), which prints
`1, 3, 2, 3, 4` on separate lines.

## Comparison Values

Modern classic and MIR6502 support `<`, `<=`, `>`, `>=`, `=`, and `#`/`<>`
as expressions producing BYTE 0 (false) or 1 (true):

```action
result=(x<y)
wide=(x>=lo AND x<hi)
PrintBE(x=y)
RETURN(x#y)
```

These values can be assigned, passed, returned, indexed with, or composed with
ordinary arithmetic/bitwise operators. Comparison operands use the existing
promotion rules; assignment to INT/CARD zero-extends the BYTE result.
In value context, AND/OR/XOR evaluate both operands and remain bitwise: for
example, `(x<y) OR 2` produces 2 or 3. This does not change the existing
conditional AND/OR behavior.

This feature is modern-only. The original cartridge does not assign numeric
values to relational expressions. Compatibility rejects value uses during
semantic analysis with `comparison values require the modern profile`;
comparisons in IF, WHILE and UNTIL conditions remain supported.

## Volatile Storage

`VOLATILE` qualifies storage whose contents can change outside the current
Action! routine, most commonly hardware and operating-system registers:

```action
VOLATILE BYTE WSYNC=$D40A,
              VCOUNT=$D40B,
              COLBAK=$D01A

VOLATILE CARD RTCLOK=$0012
VOLATILE BYTE ARRAY POKEY(16)=$D200
```

The qualifier precedes the type and applies to every entry in the declaration:

```text
VOLATILE (BYTE|CHAR|CARD|INT) [ARRAY] declaration-entry
```

Each source read performs one real memory read and each source write performs
one real memory write. actionc does not cache, combine, remove, duplicate, or
reorder those accesses. A compound assignment retains its read and write and
avoids a 6502 read/modify/write instruction when that instruction would add an
observable dummy write.

`VOLATILE` is a compiler-ordering rule; it emits no fence instruction. A
volatile `CARD` or `INT` access still consists of two byte accesses and is not
atomic.

Global and routine-local scalar and array declarations are supported. A scalar
storage alias initialized from volatile storage inherits the qualifier. The
first implementation rejects volatile constants, parameters, record fields,
and pointer declarations; volatile pointer cells and pointers to volatile data
need distinct future syntax.

`VOLATILE` is supported by compatibility, optimized classic, and MIR6502 modes.
It is an actionc extension and is not accepted by the original Action!
cartridge compiler.

## ATASCII And Screen-Code Escapes

String literals and character constants accept textual byte escapes. In
addition to exact and named ATASCII bytes and inverse text, `\{SCREEN:text}`
converts ATASCII text to the internal screen codes consumed directly by ANTIC:

```action
BYTE eol = '\{RETURN}
BYTE inverseA = '\{INV:A}
BYTE screenA = '\{SCREEN:A}
CHAR ARRAY title(0)="\{SCREEN:ACTION!}"
```

Use screen-code escapes only for data read as a character display buffer, not
for `Print`, CIO, or files. See [ATASCII and screen-code escapes](ATASCII_ESCAPES.md)
for the exact forms and conversion table.

## Typed Cast Expressions

Use Action!-style type syntax followed by a parenthesized expression:

```action
BYTE(expr)
CARD(expr)
INT(expr)
CHAR(expr)

BYTE POINTER(expr)
CARD POINTER(expr)
CHAR POINTER(expr)
```

The cast is an explicit promise to the semantic layer and code generator. The
first implementation treats it as a type reinterpretation, not as a generated
numeric conversion.

Typical uses:

```action
Print(CHAR POINTER(menu))
PopUp(BYTE POINTER(@delcancel), 1, 4)
Strcpy(CHAR POINTER(linebuf), CHAR POINTER(@filename))
```

## Explicit Address Values

Use Action!'s existing address-of spelling for places and labels:

```action
@buffer
@delcancel
@DrawMenu
```

For routine/data-block labels, the address value should normally be paired with
a typed pointer cast at the call site:

```action
PopUp(BYTE POINTER(@delcancel), 1, 4)
```

This gives source a readable escape hatch for old Action! idioms such as using
`PROC name=*() [...]` as inline data while keeping the intended pointer type
explicit. Legacy code may still rely on more implicit forms; modernized code
should use the explicit address and pointer spelling.

## Plain CARD Values Are Not Typed Pointers

The original compiler and old Toolkit sources sometimes use `CARD` values as
raw addresses. `actionc` still accepts some of those idioms, especially in the
legacy profile, but a plain `CARD` is not a typed pointer everywhere.

For example, these forms are rejected in both profiles because `p` is only a
`CARD`:

```action
CARD p
BYTE b

p^ = 1
b = p(0)
```

Modernize these sites by declaring the intended pointer type, or by casting an
explicit address at a call boundary:

```action
BYTE POINTER p
BYTE b

p^ = 1
b = p(0)

PopUp(BYTE POINTER(@menuData), 1, 4)
```

The maintained Toolkit and TN samples use this style for old menu/data-block
patterns.

## Function Pointers

Use Action-like routine syntax with `POINTER`:

```action
PROC POINTER handler
BYTE FUNC POINTER keyReader
CARD FUNC POINTER nextItem
```

Assign routine addresses explicitly:

```action
handler = @DrawMenu
keyReader = @Key
```

Call through the pointer with normal call syntax:

```action
handler()
b = keyReader()
```

The first implementation models only the routine kind and return type;
parameterized function-pointer signatures can be added later if needed. Direct
assignment to routine names is rejected in the modern profile:

```action
DrawMenu = OtherProc      ; rejected
handler = @OtherProc     ; accepted
```

## Machine Block Label Bytes

Inside machine blocks, `<name` and `>name` emit the low and high byte of a
symbol address:

```action
PROC Target()
RETURN

PROC JumpVector=*()
[ <Target >Target ]
```

This keeps full label operands unchanged (`[$20 Target]` still means a two-byte
absolute operand) while making byte selection explicit for tables and
self-contained machine code fragments.

## Relocatable Static Initializers

Initializer lists can contain addresses that are fixed after the compiler lays
out storage and routines. Use `<` or `>` in a byte array and `@` in a word
array:

```action
BYTE ARRAY dlist(3)=[$41 <dlist >dlist]

PROC Draw()
RETURN

CARD ARRAY handlers(1)=[@Draw]
```

The compiler emits the low byte, high byte, or complete little-endian word at
the initializer position. Constant addends are supported, for example
`<dlist+4`, and forward references are allowed. An array reference denotes its
element backing address, including for arrays represented internally by a
descriptor.

## MADS-Style Inline Assembler

Use `ASM` and `ENDASM` on their own lines to embed official NMOS 6502
instructions:

```action
BYTE ARRAY pixels(256)
CARD ptr=$A0

PROC Draw()
ASM
    lda #<pixels
    sta ptr
    lda #>pixels
    sta ptr+1

    ldy #0
loop:
    lda (ptr),y
    sta pixels,y
    iny
    bne loop
ENDASM
RETURN
```

The assembler is built into `actionc`; MADS is not needed at compile time.
The supported MADS-compatible subset includes:

- all official NMOS 6502 instructions and addressing modes;
- hexadecimal `$`, binary `%`, decimal, and ATASCII character constants;
- named labels (with an optional colon) and anonymous `@`, `@+`, `@-` labels;
- block-local `name = expression` and `name EQU expression` constants;
- `.z`/`.b` and `.a`/`.w` address-size suffixes;
- checked arithmetic, shift, and bitwise expressions;
- `;`, `//`, and `/* ... */` comments;
- direct references to visible Action! globals, locals, parameters, arrays,
  constants, and routines.

Address selection is deterministic. Numeric addresses below `$100` use a
zero-page encoding where one exists. Ordinary allocated Action! objects use an
absolute encoding. `.z`, `(pointer),Y`, and `(pointer,X)` require the referenced
pointer cell to be provably in zero page, for example:

```action
CARD ptr=$A0
```

Assembler-local names shadow Action! names. Prefix a name with `:` to request
the Action! object explicitly:

```action
BYTE value

PROC Example()
ASM
value:
    inc :value
    bne value
ENDASM
RETURN
```

Low and high address bytes are written `#<name` and `#>name`. A numeric
Action! `DEFINE` or a visible `CONST` can be used directly as an operand; the
compiler diagnoses a byte operand that does not fit instead of truncating it.
A direct `JSR` to an Action! routine is relocated through the normal routine
identity; storage references likewise retain a stable compiler storage
identity rather than a source-name string.

MADS-style self-modification labels can name the first encoded operand byte:

```action
ASM
    lda patch:#0
    clc
    adc #1
    sta patch

    lda source:$ff00,y
    sta source+1
ENDASM
```

For a word operand, the label names its low byte and `label+1` names its high
byte. The instruction must have an encoded operand, so implied and accumulator
forms cannot carry such a label. Reads or writes through an inline-code label
are treated as conservative memory effects by the optimizer.

Analyzed blocks participate in MIR6502 memory-effect and machine-register
liveness analysis. Fall-through and return paths must preserve stack depth.
Operations whose effects are deliberately outside that contract can use
`ASM OPAQUE`, which still receives syntax, opcode, relocation, and zero-page
validation but acts as a full compiler barrier:

```action
ASM OPAQUE
    ; deliberately non-standard machine-state manipulation
ENDASM
```

Macros, conditional assembly, repetition, include/output directives, data
directives, illegal opcodes, and 65C02/65816 instructions are not part of this
initial subset. Keep static data in Action! arrays.

## Explicit Lexical Blocks

The modern profile supports nestable, line-delimited `BEGIN`/`END` blocks:

```action
PROC Main()
  BYTE value

  value=1
  BEGIN
    CARD value

    value=1000
  END

  ; BYTE value is visible again.
RETURN
```

Each explicit block creates one lexical scope. Its ordinary declarations form
a prefix before the first executable statement. LET is an executable binding
statement and may appear among statements; it does not reopen that declaration
prefix. A block may shadow names from an outer block, the routine, a module/global
scope, or the resident library. Lookup after `END` resumes in the parent scope,
and sibling blocks cannot see one another's declarations.

Supported block declarations include scalar and array storage, pointers,
`VOLATILE` and absolute storage, storage aliases, native `REAL`, `CONST`,
`TYPE`, and `RECORD`. Block-local `DEFINE` is not supported because source-text
expansion requires its own scoped parser environment.

An `IF`, loop, or other control-flow body does not create a scope by itself; put
an explicit block inside it when local declarations or shadowing are wanted.
Lexical visibility also does not imply stack allocation. Block locals retain
Action!'s static routine-storage lifetime, so an address may escape the block
even though the declaration's name is no longer visible. LET homes are the
exception: exposing their addresses is currently prohibited.

`BEGIN` and `END` are contextual words rather than lexer keywords. They remain
legal ordinary identifier spellings in compatibility source. A lexical block
is a modern-profile feature and is rejected by the compatibility profile with a
focused diagnostic.

See [samples/lexical-blocks.act](../samples/lexical-blocks.act) for nested
shadowing, a block-local type, a branch-local block, and address escape.

## Compatibility Policy

These extensions are accepted by `actionc`, but they are not proof that the
original Action! compiler accepted the same source. The legacy profile remains
the reference-oriented path for compatibility work and accepts more old
Action!-style implicit idioms. The modern profile uses these explicit forms to
avoid ambiguous routine-address and pointer behavior, and may also use them to
support future IR-based optimizations.
