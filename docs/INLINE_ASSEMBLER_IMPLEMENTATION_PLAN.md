# MADS-Style Inline Assembler Implementation Plan

Status: initial implementation complete; migration and MADS-oracle follow-up remain
Snapshot: 2026-07-27

The initial implementation now provides the integrated `ASM`/`ASM OPAQUE`
frontend, official NMOS 6502 encoding, local labels and constants, stable
Action! storage/routine relocations, classic and MIR6502 emission, zero-page
proof checks, verifier-clean NIR, structured memory effects, and MIR6502
incoming-register liveness. The optional external MADS comparison utility,
per-instruction listing source maps, known-callee refinement inside assembler
blocks, and representative source migrations remain follow-up work; none is
required to use the source feature.

## Goal

Add a small, integrated 6502 inline assembler that uses familiar MADS syntax
and can refer directly to Action! objects. The initial implementation must be
useful for replacing code-oriented `BYTE ARRAY ...=[...]` machine blocks
without importing the full MADS language.

The assembler is part of `actionc`; compiling a source file must not require a
MADS executable. It emits relocatable bytes plus structured effects so both the
classic and MIR6502 backends can handle inline assembly correctly. MIR6502 must
be able to use the same liveness and data-flow infrastructure around an
assembler block that it uses around compiler-generated instructions.

An intended source-level example is:

```action
BYTE ARRAY pixels(256)
CARD ptr=$A0

PROC Example()
ASM
    lda #<pixels
    sta ptr
    lda #>pixels
    sta ptr+1

    ldy #0
    lda (ptr),y
    sta pixels+10
ENDASM
RETURN
```

Within an `ASM` block, Action! objects behave like assembler symbols. Their
spelling is resolved using normal Action! visibility rules, while operand
syntax and instruction selection follow the supported MADS subset.

## Non-goals

The first implementation is not a complete MADS replacement. In particular, it
does not initially support:

- macros or macro commands such as `MVA` and `MWA`;
- `.MACRO`, `.PROC`, `.LOCAL`, `.REPT`, or conditional assembly;
- mnemonic chaining;
- `ORG`, `RUN`, `INI`, `ICL`, `INS`, or output-file directives;
- redefinable `SET` symbols;
- illegal opcodes, 65C02 instructions, or 65816 instructions;
- arbitrary `DTA` data construction;
- branches between an assembler block and the surrounding Action! CFG;
- self-modifying references to instruction operand bytes.

Static tables should continue to use Action! arrays. The first milestone is
code assembly and integration, not a second data-definition language.

## Existing baseline

Action! machine blocks currently enter the AST as `Stmt::MachineBlock`, retain
byte-oriented source payloads, and are emitted conservatively. They are useful
as a compatibility mechanism but hide instruction semantics from compiler
analysis.

The current repository already has two important pieces to reuse:

- the emitter's decoder covers the official NMOS 6502
  opcode/addressing-mode combinations;
- MIR6502 has centralized instruction-effect and liveness infrastructure,
  including bounded decoding of known machine blocks.

The implementation should turn the opcode knowledge into one shared,
declarative ISA table used by encoding, decoding, validation, and effect
analysis. It must not add a second handwritten opcode map.

At the time of this plan, TN lowers 48 machine blocks, of which approximately
41 contain executable code and 7 contain data. The toolkit sources contain
fewer blocks, but KALSCOPE is a useful first migration target because its
machine block exists mainly to preserve deliberate zero-page side effects.

## Source syntax

### Block form

The initial block form is:

```action
ASM
    ; MADS-style source
ENDASM
```

`ASM` and `ENDASM` must be standalone Action! tokens. The body is lexed by a
dedicated, line-aware assembler lexer. The normal Action! lexer is not suitable
because it discards information needed by assembler syntax and assigns
different meanings to characters such as `#`, `%`, `@`, and `*`.

An explicit conservative escape hatch is also planned:

```action
ASM OPAQUE
    ; validated and relocated, but treated as a full machine-state barrier
ENDASM
```

`ASM OPAQUE` still uses the same parser, opcode validation, and relocation
rules. It differs only in its declared effects.

### Supported MADS subset

The first useful subset includes:

- all official NMOS 6502 instructions;
- immediate, zero-page, absolute, indexed, indirect, `(zp,X)`, `(zp),Y`, and
  relative addressing;
- case-insensitive mnemonics and assembler names;
- `.z`/`.b` to require byte/zero-page addressing;
- `.a`/`.w` to require absolute/word addressing;
- hexadecimal `$...`, binary `%...`, and decimal integer literals;
- ATASCII character constants;
- unary `+`, `-`, `~`, low-byte `<`, and high-byte `>`;
- arithmetic, shifts, bitwise operators, and parentheses;
- current-location `*`;
- named labels, with the colon optional;
- anonymous `@`, `@+`, and `@-` labels;
- block-local constants using `name = expression` and `EQU`;
- `;` comments, with `//` and `/* ... */` accepted when unambiguous.

Expression evaluation must be checked. Overflow, an invalid relocation
operation, or an operand that cannot fit its selected addressing mode is a
compile-time diagnostic, never silent truncation.

The supported syntax should follow the public MADS documentation where the
subset overlaps:

- <https://mads.atari8.info/mad-assembler-mkdocs/en/syntax/>
- <https://mads.atari8.info/mad-assembler-mkdocs/en/labels/>
- <https://mads.atari8.info/mad-assembler-mkdocs/en/mnemonics/>
- <https://github.com/tebe6502/Mad-Assembler>

### Namespaces

Assembler-local labels and constants occupy a namespace local to one `ASM`
block. An assembler-local name shadows an Action! object with the same spelling.

MADS global-label notation is used to bypass that local namespace:

```action
BYTE loop

PROC Example()
ASM
loop:
    inc :loop       ; the Action! BYTE, not the assembler label
    bne loop        ; the assembler-local label
ENDASM
RETURN
```

Action! references use normal source visibility and may be forward references
where Action! already permits them. Unresolved or ambiguous names receive an
assembler diagnostic that includes both the assembler operand and the
surrounding Action! scope.

## Action! object semantics

The meaning of an external symbol depends on the operand syntax:

| Assembler form | Meaning |
| --- | --- |
| `#<array`, `#>array` | Low/high byte of the array backing address; no memory read |
| `lda array,x` | Read array storage |
| `sta variable` | Exact write to the variable's storage |
| `lda pointer` | Read the pointer cell itself |
| `lda (pointer),y` | Read the pointer cell and read indirectly addressed memory |
| `jsr Routine` | Call the Action! routine |
| `#<Routine`, `#>Routine` | Low/high byte of the routine address; not a call |
| `constant+2` | Compile-time expression if `constant` is an Action! constant |

The address exported for an Action! array must match the language's existing
`@array` meaning:

- an inline or deferred local array denotes its element backing;
- a fixed-address array denotes its declared address;
- an alias denotes its backing plus the declared offset.

If an object has no single stable representation that satisfies the requested
operand, compilation fails with a targeted diagnostic.

Referencing an ordinary local or parameter from inline assembly forces it to
retain a physical home. Taking an address is not itself a read of the contents.
The retained-home decision must be represented as a structured storage fact,
not inferred later from source text.

## Address-size and zero-page rules

Instruction size must be deterministic and must not change during final
emission.

- A numeric address below `$100` selects zero-page form when the opcode permits
  it.
- A fixed-address object proven to be in zero page selects zero-page form.
- An ordinary allocated local or global defaults to absolute form.
- `.z` or `.b` requires proof that the final address is in zero page.
- `.a` or `.w` forces absolute form even when a zero-page encoding exists.
- `(pointer),y` and `(pointer,x)` require a fixed or otherwise proven
  zero-page pointer cell.

For example, an unconstrained `CARD ptr` cannot silently become a zero-page
indirect operand. The diagnostic should suggest a fixed zero-page declaration,
an explicit supported storage attribute if one exists, or a different
instruction sequence.

Automatic permanent zero-page placement solely because a symbol appears in
inline assembly is deferred. It changes global allocation policy and should not
be coupled to the first assembler implementation.

## Compiler architecture

### Frontend and AST

The Action! lexer recognizes a complete `ASM ... ENDASM` region and preserves
its raw body with source offsets. A dedicated `asm6502` lexer and parser produce
typed instruction, operand, expression, label, and directive nodes with
line/column spans.

The AST represents inline assembly explicitly, for example:

```text
Stmt::InlineAsm {
    mode: Analyzed | Opaque,
    statements,
    span,
}
```

It must not be encoded as a legacy machine-block string.

### SemIR

SemIR owns the source-language meaning of external references. It resolves each
external operand to a stable symbol reference and records:

- storage, static, routine, or constant identity;
- address use versus content read/write;
- addend and low/high-byte selector;
- addressing-size constraint;
- source span;
- call or terminal-control meaning.

Assembler-local labels remain local to the inline program and never become
Action! symbols.

### NIR

NIR must remain target-neutral. It must not contain 6502 mnemonic, addressing
mode, register, or flag enums.

SemIR and the assembler component encode the program into a generic relocatable
inline-code object before it enters verifier-clean NIR:

```text
NirInlineCode {
    id,
    bytes,
    relocations,
    effects,
    control,
    source_fragments,
}
```

Relocation targets use stable identities:

```text
Storage(NirStorageId)
Static(StaticId)
Routine(RoutineId)
InlineOffset(offset)
Absolute(address)
```

Initial relocation kinds are:

```text
Byte
WordLittleEndian
LowByte
HighByte
RelativeByte
```

Effects and control summaries use target-independent domains: storage
reads/writes/address-taking, unknown memory, calls, fall-through, return, and
terminal transfer. Debug source fragments are metadata; executable meaning
must not depend on raw names or expression strings.

The NIR verifier rejects:

- unresolved or string-named executable references;
- invalid stable IDs;
- relocation ranges outside the byte payload;
- overlapping relocations;
- relative targets outside the inline program;
- inconsistent fall-through/terminal control summaries;
- missing or internally inconsistent effects.

### MIR6502 and classic emission

MIR6502 receives one inline-code operation plus its structured payload and
effects. Relocations are resolved only after storage layout and routine labels
are known.

The classic backend consumes the same encoded payload and relocations. It may
initially treat analyzed blocks conservatively, but it must emit the same bytes
and resolve the same Action! objects.

The shared NMOS 6502 ISA table supplies:

- mnemonic and legal addressing modes;
- opcode byte and instruction length;
- operand range requirements;
- register and flag reads/writes;
- memory access category;
- stack effect;
- branch, call, return, and terminal-control classification.

This table replaces duplicate encoder/decoder knowledge. The existing
machine-block decoder should be migrated to it before the assembler depends on
it.

## Effect and control analysis

Each analyzed block gets an internal assembler CFG. Analysis computes:

- registers and flags read before their first definition;
- registers and flags that may or must be written;
- exact external storage reads and writes;
- address-only external references;
- indirect or otherwise unknown memory effects;
- stack-depth balance across all exits;
- calls and their known summaries;
- final known register/flag state where provable;
- fall-through, return, or terminal-jump behavior.

This distinction matters. For example:

```asm
lda #0
sta target
```

does not consume incoming `A`, while:

```asm
sta target
```

does.

MIR6502 must feed the result into the shared effect, liveness, and data-flow
workflow. Inline assembly must not introduce a parallel liveness mechanism.
Known `JSR` targets should use normal known-callee summaries; unknown calls
remain conservative.

An analyzed fall-through block is accepted only when every reachable exit:

- has compatible control behavior;
- has balanced stack depth;
- leaves decimal mode in an allowed state;
- does not branch outside the inline program.

Instructions such as `TXS`, deliberate unmatched stack manipulation, or effects
that cannot yet be described precisely require `ASM OPAQUE`. Opaque blocks are
full register, flag, stack, and memory barriers but still benefit from syntax
checking and relocation.

An `RTS` or a terminal `JMP` may terminate an `ASM` statement when represented
explicitly in its control summary. Jumps into an Action! block and Action!
branches into an assembler-local label are rejected.

## Diagnostics and tooling

Diagnostics should point to the exact assembler token and include the Action!
source context. Required cases include:

- unknown mnemonic or unsupported MADS feature;
- illegal addressing mode for an opcode;
- ambiguous zero-page versus absolute form;
- `.z` without zero-page proof;
- indirect use of a non-zero-page pointer cell;
- branch out of range;
- duplicate or unresolved local label;
- unresolved or ineligible Action! object;
- invalid relocation expression;
- unbalanced stack or incompatible exits;
- unsupported control transfer across the block boundary.

Listings should reproduce readable assembler source alongside emitted bytes and
resolved addresses. Source maps must retain per-instruction spans so runtime
addresses can be traced back to an assembler line.

A developer-only compatibility utility should be able to assemble supported
snippets with both the internal assembler and a pinned MADS version, then
compare bytes and diagnostics. MADS is a development oracle, not a runtime or
CI dependency.

## Implementation slices

### Slice 0 — Contract and compatibility corpus

- Freeze the supported syntax, precedence, and address-size rules in tests.
- Document Action!-versus-assembler namespace precedence and array-address
  semantics.
- Build golden-byte fixtures with a pinned MADS version, including negative
  fixtures for intentionally unsupported syntax.
- Record MADS version and invocation used to produce each golden result.

Exit gate: the subset is reviewable without relying on implementation behavior
as its specification.

### Slice 1 — Shared 6502 ISA table

- Extract the official NMOS 6502 opcode/addressing metadata into one
  declarative table.
- Move existing decoding and instruction-effect users onto the table.
- Add encode/decode round-trip coverage for every supported combination.
- Prove existing emitted binaries remain byte-identical.

Exit gate: one table describes all official encodings and the current compiler
still emits identical code.

### Slice 2 — Standalone assembler parser

- Implement the line-aware lexer, expression parser, addressing parser, local
  labels, anonymous labels, constants, suffixes, and comments.
- Select instruction forms using the shared ISA table.
- Resolve block-local branches and report range errors.
- Match the Slice 0 MADS golden corpus.

Exit gate: numeric, self-contained assembler snippets produce correct
relocatable bytes and useful diagnostics.

### Slice 3 — Numeric end-to-end block

- Add the `ASM` block token, AST node, SemIR representation, generic NIR
  inline-code payload, verifier rules, and both emission paths.
- Initially allow conservative effects.
- Add a first runtime example such as:

  ```action
  ASM
      lda #1
      sta $D01A
  ENDASM
  ```

Exit gate: numeric straight-line assembly works through the complete classic
and MIR6502 pipelines without using legacy machine-block strings.

### Slice 4 — External Action! references

- Resolve globals, locals, parameters, arrays, statics, constants, and routines.
- Add low/high-byte relocations, addends, calls, and forward references.
- Implement retained-home and address-taken storage facts.
- Implement `:name` lookup past assembler-local symbols.
- Add zero-page eligibility diagnostics.

Exit gate: a runtime fixture takes an Action! array address in assembly and
accesses the actual emitted backing in both backends.

### Slice 5 — Indexed and indirect operands

- Enable indexed object references and supported indirect modes.
- Enforce fixed/proven zero-page pointer-cell requirements.
- Test boundary addresses and page-crossing behavior at runtime.

Exit gate: direct, indexed, and indirect external operands encode
deterministically and execute correctly.

### Slice 6 — Local control flow

- Complete named and anonymous branch handling.
- Support local absolute jumps and current-location expressions.
- Construct the internal assembler CFG.
- Model `RTS` and terminal jumps.
- Reject all transfers between assembler-local and Action! CFG labels.

Exit gate: multi-block inline programs have verified internal control and
accurate terminal behavior.

### Slice 7 — Precise effects and liveness integration

- Compute register, flag, memory, call, stack, and exit-state summaries.
- Use known-callee summaries for resolved Action! calls.
- Enforce stack and decimal-mode safety.
- Add `ASM OPAQUE` for valid programs outside the analyzed contract.
- Connect summaries to centralized MIR6502 effects and liveness.

Exit gate: live producers before an `ASM` block are retained, dead producers
are removed, exact external writes invalidate only their target, address-taking
does not count as a content read, and indirect memory remains conservative.

### Slice 8 — Listings, source maps, and diagnostics

- Add per-instruction source maps and readable listing output.
- Print inline-code objects readably in diagnostic IR without introducing
  executable string identities.
- Finish targeted suggestions for unsupported or ambiguous forms.
- Add the optional MADS comparison utility for developers.

Exit gate: an emitted byte and a compilation error can both be traced to the
precise inline assembler source.

### Slice 9 — Migration

Migrate representative blocks gradually:

1. KALSCOPE's deliberate zero-page side-effect code;
2. small TN initialization or boot fragments;
3. PMG-oriented loops;
4. larger TN/LIB leaf blocks where analysis gives a measurable benefit.

Data-only machine blocks remain Action! arrays unless a later design explicitly
adds data directives.

Each migration must compare classic and MIR6502 behavior, run the relevant VM
coverage, inspect the listing, and record size changes. No migration may depend
on a sample-specific compiler special case.

## Required validation

In addition to slice-specific parser, encoder, relocation, and runtime tests,
changes that cross SemIR/NIR boundaries must run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

MIR6502 and compatibility changes should also run the relevant repository
checks, including:

```sh
cargo test asm6502
cargo test --test mir6502_fixtures
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test --test compatibility -- --ignored
```

If exact command names differ when the new test targets are introduced, the
implementation should update this note and the documentation index rather than
silently dropping a validation layer.

## First milestone

The first externally useful milestone is complete when:

- KALSCOPE can express its code machine block as readable MADS-style assembly;
- inline assembly can take and relocate the address of an Action! array;
- both classic and MIR6502 emit and execute valid code;
- verifier-clean NIR contains only stable relocation identities and generic
  inline-code effects;
- MIR6502 sees precise enough effects to optimize safely across a simple block;
- unsupported MADS syntax produces an explicit diagnostic.

That milestone deliberately favors a small, reliable language with strong
compiler integration over broad assembler compatibility.
