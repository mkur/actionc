# ENUM and CASE Design

The base language described here is implemented in the modern profile. See the
[syntax reference](../SYNTAX_EXTENSIONS.md#byte-enums) and
[implementation plan](ENUM_AND_CASE_IMPLEMENTATION_PLAN.md) for validation and
explicitly deferred follow-ons. Native 65816/68k checks are lowering canaries,
not executable runtime validation.

Status: base ENUM and CASE implemented; guards and guarded wildcards deferred.

The [implementation plan](ENUM_AND_CASE_IMPLEMENTATION_PLAN.md) defines the
delivery slices and checks. Review decisions incorporated here include TYPE-based
BYTE-only enums, checked previous-member numbering, ESAC, ELSE, and enum function
results. Guards and guarded wildcards are a separate follow-on, not initial scope.

The Oscar64 test-porting work is outside this workstream. Neither feature depends
on completing those ports or the deferred volatile work.

## Recommended Direction

- Add both features to the modern profile. Keep runtime selection independent:
  modern programs using them must work with standalone or cartridge services.
- Make enums distinct, named scalar types, not groups of untyped constants.
- Introduce named types through TYPE, using ENUM as the type description on its
  right-hand side, consistently with existing record declarations.
- Support only BYTE-backed enums in the first version: 8 bits, values 0 through
  255, and no representation annotation. Wider and signed enums are deferred.
  Never widen an enum implicitly as its member list grows.
- Make CASE a statement with one selector evaluation, constant labels, optional
  ELSE, and no fallthrough. Support integers independently of enums.
- Reuse constant evaluation, scalar layout/ABI, comparisons, and ordinary CFG
  lowering. Do not begin with jump tables or a new target-specific dispatcher.

```action
TYPE RunState=ENUM [Idle Running Stopped]
TYPE ResultCode=ENUM [Ok=0 Failed=1 Busy=10 Retry]

RunState mode

PROC Main()
  mode=RunState.Idle

  CASE mode OF
  WHEN RunState.Idle THEN
    mode=RunState.Running
  WHEN RunState.Running, RunState.Stopped THEN
    mode=RunState.Stopped
  ELSE
    ; Handles an unnamed value received from memory or an explicit cast.
    mode=RunState.Idle
  ESAC
RETURN
```

The base syntax below is implemented; the separate guard extension remains
deferred.

## 1. ENUM Contract

### Declaration, identity, and representation

```text
enum-declaration := TYPE identifier '=' ENUM
                    '[' enum-member ( [ ',' ] enum-member )* ']'
enum-member      := identifier [ '=' constant-expression ]
```

At least one member is required. Whitespace separates members; line breaks are
formatting and commas are optional. The declaration uses TYPE and brackets,
not a separate `ENUM name ... ENDENUM` declaration form.

Every enum has BYTE representation: exactly 8 unsigned bits, with values 0
through 255, on 6502, 65816, and 68k. There is no `:BYTE`, `:CARD`, or `:INT`
annotation in this first version. Diagnose representation annotations explicitly
rather than silently ignoring them. Wider and signed representations require a
future language extension; they are not needed for feature completion here.

Alignment, pointer width, and argument placement follow the target's existing
rules for BYTE values and typed pointers. No host-sized enum ABI is added.

TYPE is the named-type declaration mechanism; ENUM describes one kind of type.
Existing `TYPE Name=[fields]` records retain their syntax. Future unions, tagged
variants, or aliases can use other right-hand-side descriptions, but are not
part of this implementation. In particular, payload-bearing variants must not
silently change the byte representation or semantics of an enum.

Each declaration creates a distinct type, even if another declaration has the
same representation and members. Enum identity is resolved semantically, not
by comparing source spellings or storage widths. An import of an existing type
preserves its identity; it does not create a new enum.

### Member values

- The first implicit value is zero. Each later implicit value is the immediately
  preceding member's value plus one, including after an explicit assignment.
- An explicit assignment uses the existing integer constant-expression rules.
  Its evaluated numeric result must fit BYTE (0 through 255), without
  an implicit narrowing conversion. Automatic increment is checked and never
  wraps. An explicit scalar cast still has its ordinary conversion semantics.
- Duplicate member names, case-insensitively, and duplicate numeric values are
  errors. Numeric aliases are deferred; ordinary CONST aliases remain possible.
  Consequently, an enum may declare at most 256 distinct members.
- Member expressions may refer to already available constants and earlier
  members of this enum, but not later members or themselves. Numeric arithmetic
  on an earlier enum member requires an explicit scalar conversion.

For example, `TYPE Codes=ENUM [First=254 Last]` is valid; adding another implicit
member is an overflow error. There is no widening option in the first version.
`TYPE Codes=ENUM [Bad=256]` and `TYPE Codes=ENUM [Bad=-1]` are errors, whereas an
explicit `BYTE(256)` requests the existing wrapping conversion to zero. Use an
explicit in-range value such as 255 when an all-bits-set error code is wanted.

In `TYPE ResultCode=ENUM [Ok=0 Failed=1 Busy=10 Retry]`, Retry is 11. Assigning
explicit byte values and continuing implicit numbering does not require support
for signed or word-sized enums.

In `TYPE ResultCode=ENUM [Ok=0 Failed=11 Busy=10 Retry]`, Retry also becomes 11,
which duplicates Failed and is an error. Numbering neither skips occupied values
nor continues from the maximum value seen earlier in the declaration.

This does not introduce a second, arbitrary-precision arithmetic language for
constant expressions. Existing typed expression arithmetic is evaluated first;
the resulting member value and the implicit increment are then checked.

### Names and scopes

Members are qualified: `RunState.Idle`, not bare `Idle`. They are not inserted
into the surrounding ordinary symbol namespace. Two enums can both have an
`Idle` member without a collision.

The enum type itself occupies the existing ordinary namespace, with existing
case-insensitive duplicate and shadowing rules. Consequently, `RunState mode`
is valid, but an enum named `State` and a variable named `state` in the same
scope are not a way to distinguish a type from a value.

Allow declarations in the same declaration prefixes as named record types:
global/module, routine-local, and explicit modern lexical blocks. No new
implicit scope is introduced. Use normal declaration visibility and source-order
rules; do not add forward enum declarations.

`PUBLIC TYPE Name=ENUM [...]` exports the type and all its members. Qualified
module/import paths such as `Game.RunState.Idle` resolve through module and enum
identities.
`USE ALL FROM` may expose the enum type under its normal import rules, but must
not inject its members as bare values. Public signatures containing enum types
must respect the project's type-visibility rules.

### Type checking and operations

The complete feature must support an enum wherever an existing BYTE scalar is
stored or passed: variables, parameters, function results, typed callable
signatures, pointers, arrays, and scalar or inline-array
record fields. This does not add new general pointer-field syntax or aggregate
returns.

Enum function results are part of the first complete delivery, not an optional
follow-on. Extend existing FUNC POINTER return typing as well; parameterized
function-pointer declaration syntax remains outside this feature's scope.

```action
CONST RunState InitialMode=RunState.Idle
RunState ARRAY modes(8)
TYPE Worker=[RunState current RunState ARRAY history(4)]

RunState FUNC ReadState()
RETURN(RunState.Running)
```

Assignments, arguments, returns, and typed initializers require the same enum
identity. A raw integer, another enum, or a pointer is not implicitly accepted
merely because its representation fits. Function-pointer compatibility must
retain this distinction even when machine calling conventions are identical.

Equality and relational comparisons are allowed between values of the same enum.
Ordering is by the unsigned byte value, not by the member's declaration position.
Modern comparison-as-value still produces BYTE zero or one.

There is no implicit conversion to an integer or truth value. Arithmetic,
bitwise operations, loop counters, and numeric array indices require an explicit
integer conversion. Bit flags are not a special enum mode in this version.

Explicit conversions are:

- `BYTE(mode)`, `CARD(mode)`, `INT(mode)`, or `CHAR(mode)`: apply the existing
  integer conversion rules to the underlying value.
- `RunState(raw)`, where `raw` has a fundamental integer type: convert to the
  BYTE representation and attach the enum identity. The source integer may be
  wider or signed; that does not make the resulting enum wider or signed.
- Conversion between two enum types requires the explicit integer bridge.
  REAL and pointer conversions likewise require existing explicit intermediate
  conversions; no new direct conversions are introduced here.

Type names in conversion syntax must be resolved and classified, rather than
treated as routine calls or record names based on their spelling.

### Unnamed values are valid representation values

An enum describes named values, not a promise that memory can contain only those
values. Hardware, external code, pointer writes, casts, and storage zero-fill can
all produce an unnamed value. Every bit pattern of the byte representation
therefore has defined behavior.

`RunState(17)` is allowed whether its argument is constant or computed at runtime.
There is no membership check, trap, or undefined behavior. Comparisons and casts
use the stored numeric value. A CASE on it selects ELSE, or continues after the
CASE if no arm matches and ELSE is absent.

In particular:

- The optimizer must never assume that merely naming every declared member makes
  the remaining path unreachable; that is not proof of covering all 256 values.
- Existing zero-fill rules still mean numeric zero, not "the first member".
  They do not establish a named-member invariant.
- Declaration initialization retains Action!'s existing storage/backing meaning;
  enum syntax does not add constructors or block-entry initialization.
- Constants and member names allocate no runtime storage or automatic name table.
  Printing member names and checked conversions would be separate library or
  language features.

This deliberately favors predictable low-level, cross-target behavior over a
closed enum model that would require checks at every external-memory boundary.

## 2. CASE Contract

### Syntax

```text
case-statement := CASE expression OF
                  case-arm+
                  [ ELSE statement* ]
                  ESAC
case-arm       := WHEN case-label ( ',' case-label )* THEN statement*
case-label     := constant-expression [ TO constant-expression ]
```

Use explicit WHEN arms because `:` already separates Action! statements, and
modern equality expressions can resemble assignments. A Pascal-style bare
`expression:` arm header would create unnecessary ambiguity with existing body
statements. THEN reuses the existing conditional-body delimiter; CASE/ESAC follows
the existing IF/FI and DO/OD closing convention. ENDCASE is not an alias.

ENUM, CASE, OF, WHEN, and ESAC remain contextual identifiers, without historical
cartridge token IDs. Recognize them only in their complete structural contexts.
Existing calls, assignments, declarations, and fields named `Case`, `When`,
`Of`, `Esac`, or `Enum` must remain legal where previously legal. ENUM has its
type-description meaning after `TYPE name=` in a complete enum definition.

For the initial grammar, the CASE header occupies its own physical source line
and ends in OF; each WHEN header occupies its own line and ends in THEN. ELSE
and ESAC occupy their own lines. Bodies start on subsequent lines. Comments
are allowed after headers. This follows the existing explicit-block approach
to contextual markers; it does not make newlines general expression tokens.
Relaxing header layout can be considered separately if needed.

### Selector and labels

Selectors may be BYTE, CHAR, CARD, INT, or an enum. REAL, records, pointers,
callable values, and strings are rejected. A modern comparison expression is a
valid selector because its result is BYTE.

Restricting enum storage to BYTE does not restrict integer CASE to BYTE:
ordinary CARD and INT selectors and their existing signedness remain supported.

Evaluate the selector exactly once on entry, before dispatch. In particular,
the compiler must not repeat a function call, pointer load, or hardware read
for each WHEN. Labels are compile-time constants and never execute at runtime.

For integer selectors:

- Labels are integer constant expressions. Their evaluated numeric values must
  fit the selector's type without implicit wrapping or reinterpretation.
  A BYTE selector rejects `256`; a CARD selector rejects an INT value of `-1`.
  An explicit cast requests the existing conversion and is checked afterward.
  Existing literal typing still applies: decimal `65535` has type INT and value
  `-1`; write `$FFFF` or `CARD(65535)` for the unsigned CARD value `65535`.
- Comma-separated values and inclusive `low TO high` ranges are supported.
- Signed comparisons retain signed meaning. For example, `-4 TO -1` on an INT
  selector covers four negative values.
- Descending ranges and all overlaps are errors, including duplicate constants,
  a constant inside a range, or overlapping ranges within the same arm.

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

For enum selectors, each label must be a constant of the exact same enum type:
a member, typed CONST, or explicit enum conversion. Bare numeric labels and
members of other enums are rejected. Unnamed constants such as `RunState(17)`
are valid labels under the representation-value contract. Duplicate normalized
values are errors. Enum ranges are deferred; callers may explicitly convert
the selector to an integer when numeric range matching is intended.

Do not expand a large integer range into thousands of equality labels. Preserve
it as a typed interval and initially dispatch with lower/upper comparisons.

### Execution, scope, and control flow

- Execute the one matching arm and continue after ESAC. There is no fallthrough
  and no new BREAK statement.
- ELSE is optional, last, and unique. Without it, no match means no body executes.
- Empty arm bodies are allowed. At least one WHEN arm is required.
- Arm bodies do not implicitly create lexical scopes. Use BEGIN/END for local
  declarations or shadowing, with the existing declaration-prefix rules.
- RETURN exits the routine. EXIT still exits the nearest enclosing loop,
  crossing a CASE if necessary. EXIT in a CASE outside a loop remains an error.
- Nested CASE, IF, and loops bind their own delimiters. An inner ELSE must never
  be consumed as the enclosing CASE's ELSE.
- CASE is a statement, not a value-producing match expression.

Initial source return-flow analysis is conservative: without ELSE, retain the
possible no-match path, even if every named enum member is listed. With ELSE,
combine arm flow facts using the same rules as IF/ELSE. An explicitly empty ELSE
must remain distinguishable from an absent ELSE in the source representation.

No arm's effects may be speculated into another path. Existing observability
rules for calls, absolute storage, pointer access, volatile facts, and arithmetic
faults continue to apply. CASE does not introduce a new runtime error policy.

### Guard and wildcard extension boundary

Keep source arms in order. A later guard extension can use:

```action
CASE mode OF
WHEN RunState.Running IF CanAdvance() THEN
  Advance()
WHEN RunState.Running THEN
  Wait()
WHEN _ IF CanRecover() THEN
  Recover()
ELSE
  ReportFailure()
ESAC
```

These guards are not part of the initial delivery. In that extension, match the
label first, evaluate its guard only on a match, and continue to subsequent arms
when it is false. The selector remains captured once even if a guard changes
the source variable. Preserve guard calls/effects and source-order priority.

The initial unguarded language rejects overlapping labels. With guards, repeated
values after guarded arms become useful; an earlier unconditional arm that fully
covers a later arm makes the later arm unreachable. Generalize coverage checks
at that time instead of embedding permanent global disjointness into dispatch.
None of this permits duplicate enum member values.

ELSE is the unconditional, unique, final fallback. Represent it semantically as
an unguarded catch-all arm, preserving explicit presence and source span. The
future `WHEN _ IF condition THEN` is a guarded catch-all, not another spelling
of ELSE. Bare `WHEN _ THEN` and guards on ELSE are not proposed aliases. Other
identifier uses of `_` must not become globally reserved.

Variant payload bindings and their arm-local scopes are later language work;
ordinary scalar CASE arms still need BEGIN/END for local declarations.

## 3. Compiler Integration

### Existing boundaries that need extension

The current compiler has useful machinery to reuse, but several representations
must be generalized rather than bypassed:

| Area | Current shape | Required design change |
| --- | --- | --- |
| `src/lexer.rs`, `src/parser.rs` | Original keyword token IDs plus contextual extensions; colon is a statement separator | Contextual declarations/headers and nesting-aware CASE body delimiters; preserve old identifier uses |
| `src/ast.rs` | TYPE directly contains record fields; no enum type definition or CASE statement; FUNC result is `FundType`; CONST annotations are fundamental or REAL | Generalize TYPE to a name plus a record/enum definition; add CASE and named enum constant/result syntax, including callable results |
| `src/semantic.rs`, `src/semantic/types.rs` | Scalar types, named records, typed constant facts, resolved symbols/scopes | Add nominal enum identity and representation facts, member binding, conversions, and exact compatibility |
| `src/semantic/ir.rs` | Typed expressions and structured IF/loop flow | Preserve typed enum constants and validated CASE selector/arms/default presence; update all flow/effect walkers |
| `src/nir/facts.rs`, `src/nir/lowerer.rs` | Integer types and explicit compares, Branch, Goto, temps | Erase enum representation only after semantic checks; lower CASE to ordinary typed CFG |
| `src/codegen/semir.rs` | SemIR-to-classic AST projection | Project resolved enum representation and a captured-selector compare chain without redoing language semantics |

Do not put enum names into the existing record-only `Named(String)` path and
teach backends to guess what they mean. Use a resolved enum type identity and
fact table, with readable names retained as metadata.

Generalize `TypeDecl` to carry a type-definition kind rather than always a list
of record fields. Implement only Record and Enum definitions in this workstream;
leave unions, tagged variants, and aliases for later extensions rather than
building unsupported type machinery now.

The function-result change is a real integration task, not parser sugar. Extend
named scalar result syntax and callable signatures without accidentally enabling
unsupported aggregate results or introducing an unboxed recursive AST type.
Avoid turning an enum FUNC into a BYTE FUNC before return/argument checking.

### SemIR owns the language decisions

Semantic analysis owns declaration identity, representation/range checks,
qualified member lookup, typed constants, operator legality, argument/return
compatibility, label normalization, overlap diagnostics, and loop-exit meaning.

Reuse the existing constant evaluator for numeric payloads. Extend typed constant
facts to retain enum identity as well as representation/bits; the current
scalar-only `ConstValue` cannot alone express a nominal enum constant. Do not
build a separate arithmetic evaluator for member declarations or CASE labels.

A validated SemIR CASE carries one typed selector and ordered arms whose patterns
are normalized typed singletons/intervals or an explicit final catch-all for ELSE.
Retain source spans for diagnostics. There are no unresolved label expressions.
Boundary validation checks type/domain consistency, interval validity, the
initial unguarded disjointness policy, and nested statement facts. Type checks
must survive constant materialization, not disappear when a member becomes a
literal. Add focused checks at the existing semantic boundary; this does not
require a new general-purpose SemIR verification framework.

### NIR and MIR reuse

At SemIR-to-NIR lowering, enums become U8 computation/storage types.
Keep source enum information in debug/type metadata where needed, not executable
strings. Source callable compatibility is established before ABI lowering;
equal machine signatures are not evidence that source enum types are compatible.

Capture the selector as one typed value, then emit equality or interval compares
and Branch/Goto edges to arm blocks and the common continuation. Join construction
must respect terminated arms, returns, and loop exits. CASE must not push an
entry onto the loop-exit stack. Use comparisons for ranges, not overflow-prone
subtract-and-test rewrites.

The first implementation uses a straightforward source-order comparison chain.
Existing verified-NIR folding, branch simplification, CFG cleanup, and MIR
comparison/branch selection can then run unchanged or with small generic fixes.
NIR's existing integer CFG is also consumable by the 65816 and 68k lowerers.

Do not add a Switch terminator merely to implement the source syntax. If measured
programs later justify dense tables or balanced trees, design a general costed
dispatch facility, retaining typed case facts in an explicit IR contract if
necessary. Table selection, index width, bounds checks, layout, and target costs
would be a separate optimization project, not enum-specific mechanics.

### Classic backend

Modern classic compilation must consume the same verified semantics. Extend the
projection to lower enum storage/signatures to their resolved BYTE forms and
expand CASE to a selector capture followed by ordinary IF-style comparisons.

Use compiler-owned capture storage with collision-free identity, allocated through
existing routine/storage machinery. Assign it on every CASE entry; do not mistake
it for a declaration initializer. Distinct nested cases need distinct captures
unless normal liveness proves reuse safe. Capture lifetime ends after dispatch,
and existing target routine/reentrancy rules still apply.

The projection may need to emit several statements for one source statement and
collect synthetic declarations. That is preferable to copying the selector into
each condition or re-parsing generated source. Modern classic already uses this
projection in both runtime modes; preserve that route and test both default and
explicit codegen-source choices. Do not reroute legacy AST-only compilation.

## 4. Profiles and Runtime Compatibility

- The parser remains profile-neutral; compatibility-profile semantic analysis
  diagnoses either construct as requiring the modern profile.
- Modern/classic and modern/MIR6502 must agree under both standalone and
  cartridge-linked runtime selection. Neither construct needs a runtime helper.
- Existing legacy source/token behavior remains unchanged, including contextual
  identifier spellings. This is a language-extension boundary, not permission
  to reproduce incorrect legacy arithmetic.
- The future original-compiler-in-a-VM mode is distinct: these source extensions
  are not promised to the original compiler. Generating code which calls
  cartridge services does not require that compiler to parse the source.
- 65816 and 68k validation initially uses the capabilities of their current
  harnesses. Successful NIR/MIR lowering must not be reported as native runtime
  execution unless an execution test was actually run.

## 5. Proposed Implementation Slices

The [implementation plan](ENUM_AND_CASE_IMPLEMENTATION_PLAN.md) is the authority
for slice numbering, dependencies, acceptance gates, and commands. It separates
integer CASE, the TYPE representation migration, enum scalar semantics, routine
results/callables, storage/visibility integration, and final enablement.

Each slice should be a focused, independently verified change. Do not silently
accept an unsupported use by degrading an enum to an untyped integer. Keep
incomplete combinations behind explicit capability checks until their slice is
complete; do not advertise the full feature early.

CASE comes first because it is independently useful and exercises existing typed
control flow without depending on the enum/result-type integration. Optimization
work should follow correctness and measurements, not delay this baseline.

## 6. Acceptance Tests

Cover at least:

- Enum implicit/explicit numbering, byte boundaries 0 and 255, rejection of
  negative and above-255 member values, increment overflow, 256-member coverage
  and attempted overflow, duplicate names/values, rejected representation
  annotations, empty lists, and forward/self references.
- Exact enum assignment, parameter, return and callable typing, rejected numeric
  mixing, explicit conversions, equality/ordering, and constant-folded/runtime
  parity. Include same-spelled enums in different scopes/modules.
- Arrays and nested record fields/inline arrays, pointer reads/writes, typed
  initializer flattening/zero-fill, sizes/strides, and copies. An enum without a
  named zero must not imply different initialization behavior.
- CASE first/middle/last matches, default and absent default, empty arms, nested
  cases/IFs/loops, EXIT, RETURN, and explicit lexical blocks inside arms.
- Integer boundary labels, negative ranges, full-domain ranges without expansion,
  duplicate values after folding/casts, overlapping/reversed ranges, cross-enum
  labels, and rejected nonconstant/unsupported selectors.
- A selector function with an observable call counter, and selectors sourced
  from fixed/escaped/pointer memory. Verify one evaluation in execution tests and
  one retained load/call in the relevant IR/effect assertions.
- No speculative execution of an unselected arm, especially a call, observable
  store, or division-by-zero fault. Reuse the existing fault test mechanism.
- Unnamed enum values from both casts and external memory; named-member coverage
  must not remove the default/no-match path or wrongly prove a FUNC returns.
- Contextual identifiers, comments/header layout, malformed headers, missing
  delimiters, duplicate ELSE, and precise modern-only diagnostics.
- Verifier-clean snapshots and 65816/68k lowering canaries; existing Atari ABI
  baselines should remain unchanged for programs not using these extensions.

Run focused parser/semantic/projection/VM checks for each slice. For compiler
boundary changes, the repository's required baseline includes:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Runtime coverage must exercise modern/classic and modern/MIR6502, standalone and
cart linkage, using the relevant existing harnesses. Record actual test coverage
and distinguish a cart-entry test double from an original-cartridge run.

## 7. Deliberately Deferred

Wider or signed enum representations and representation annotations, numeric
member aliases, flags enums, enum iteration/reflection, automatic member name
strings, checked/closed enum types, implicit enum-indexed arrays, enum ranges,
unions, payload-bearing variants, guards and guarded wildcards, CASE expressions/
payload matching, fallthrough, and costed jump tables are not part of this
design's first implementation.

The implementation plan preserves extension points for those features without
implementing them speculatively. No compiler changes accompany this design
document.
