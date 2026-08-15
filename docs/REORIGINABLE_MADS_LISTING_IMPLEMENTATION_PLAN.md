# Re-originable MADS Listing Implementation Plan

Snapshot date: 2026-08-15.

Status: implemented.

This is the implemented follow-up to
`MADS_COMPATIBLE_LISTING_IMPLEMENTATION_PLAN.md`. Both generated listing
variants now use this re-originable contract.

The implementation retains resolved relocation provenance in `CodegenOutput`,
uses semantic or stable ordinal display symbols, and renders word/low/high and
relative references as MADS expressions. Compatibility/classic,
optimized/classic, selectable SemIR-native classic, and MIR6502 are covered by
the same formatter contract. `tools/check-mads-listings.sh` currently validates
12 compiler/origin cases and 40 complete MADS load-file assemblies, including
two cross-origin pairs.

## Goal

Allow a generated MADS listing to be assembled at a different program origin
by changing one generated origin definition, without recompiling the Action!
source:

```asm
ACTIONC_ORIGIN = $3000
        ORG ACTIONC_ORIGIN
```

Changing `ACTIONC_ORIGIN` to `$4000` must move the complete main payload and
every address whose meaning is relative to that payload. Addresses that are
fixed by Action! source meaning, such as hardware registers, OS entry points,
resident-library routines, and explicit numeric machine-code operands, must
remain fixed.

This feature is **re-originable assembly**, not a runtime-relocatable object
format. The assembled executable still has a fixed load address and contains
no runtime relocation table.

The existing unedited-listing guarantee remains in force: assembling a listing
without changing `ACTIONC_ORIGIN` must reproduce the complete actionc load file
byte for byte, including segment headers and `RUNAD`.

## User-visible Contract

The supported edit is deliberately narrow:

1. generate either MADS listing variant;
2. change only `ACTIONC_ORIGIN`;
3. leave the fixed `ORG $02E2` `RUNAD` segment unchanged;
4. assemble with MADS using ordinary Atari executable output.

For a valid new origin:

- every generated routine, block, static, string, storage object, array
  backing, local, parameter home, spill, and compiler-generated code/data
  target moves by the origin delta;
- full, low-byte, and high-byte address materializations follow their target;
- internal address addends are preserved;
- relative branches continue to target the same generated blocks;
- instruction widths and payload layout remain unchanged;
- fixed-address symbols and explicit absolute numeric values do not move;
- `DTA A(proc_main)` writes the relocated entry point into the fixed `$02E2`
  segment.

An origin is valid when the main segment and every relocated expression fit in
the 16-bit address space and the chosen memory range is usable by the target
program. The listing cannot determine whether a new range conflicts with the
OS, cartridge, display memory, fixed Action! storage, or application-specific
memory use.

Re-originating an existing listing is not required to produce exactly the same
instruction sequence as compiling the Action! source directly at the new
origin. A direct compilation may eventually make origin-sensitive layout or
optimization decisions. The re-originated listing must instead preserve the
original instruction sequence and program meaning while relocating all
generated address references.

Generated address/byte comments describe the original actionc artifact. They
may become stale after editing `ACTIONC_ORIGIN` and are not assembler input.
The header must state this explicitly.

## Non-goals

- Do not introduce a linker, object-file format, or runtime relocation loader.
- Do not add a second listing mode or formatter.
- Do not re-run lowering, layout, optimization, or instruction selection from
  the listing formatter.
- Do not infer relocations merely because a byte or word happens to equal an
  address in the output segment.
- Do not relocate explicit numeric addresses written by the user.
- Do not make actionc invoke MADS during normal compilation.
- Do not change the semantics of `--origin`; recompiling at a chosen origin
  remains supported independently of this feature.

## Current State

The implementation already has most of the semantic and emission machinery:

- `Emitter` records label patches with `Absolute16`, `AbsoluteLow8`,
  `AbsoluteHigh8`, and `Relative8` kinds;
- inline assembly carries structured relocation target, byte-selector, addend,
  and source-use information;
- static initializers carry structured low/high/word relocations through
  SemIR, verifier-clean NIR, MIR6502, and both classic paths;
- storage and routine maps provide semantic names and final placement facts;
- the MADS formatter already emits symbolic routine/control-flow/storage
  operands where the final map makes them unambiguous;
- anonymous control-flow labels are currently derived from their absolute
  addresses, for example `L3008`; this naming must be replaced as part of the
  re-originable listing contract;
- `.A` and `.Z` suffixes already preserve the emitted instruction width.

The main missing fact is provenance after final emission. `Emitter::finish()`
resolves patches into bytes and discards the patch/label relationship before
`CodegenOutput` reaches the listing formatter. The formatter can therefore
recognize ordinary full-address operands in many cases, but it cannot reliably
distinguish an address byte from an unrelated numeric constant.

Retaining emitter patches is necessary but not sufficient. Some internal
storage operands are emitted numerically after layout is known. References to
an array element, record field, local slot byte, or other `base + offset`
location must gain explicit emission provenance too. A storage range in
`CodegenMap` can provide a readable base label after a site is known to be
output-relative, but address equality alone cannot establish that fact: an
explicit numeric operand may intentionally equal the original address of an
internal object. This must be audited before the listing can claim a general
re-origining contract.

## Architecture And Ownership

This remains an emission/artifact feature.

- SemIR continues to own whether a source expression denotes an address and
  whether a numeric address is fixed.
- NIR continues to own target-independent structured data and inline-assembly
  relocations. No new executable NIR form is expected.
- MIR6502 continues to own instruction selection, layout, and physical storage
  decisions.
- Emission continues to bind labels, resolve patches, and own final relocation
  provenance.
- `CodegenOutput` carries the minimum resolved provenance required to describe
  its final bytes.
- `src/compiler/artifacts.rs` converts final bytes, map facts, and resolved
  provenance into MADS expressions. It must not consult SemIR or re-run a
  backend.

The output relocation table is artifact metadata, not executable IR. Targets
should use offsets within the final main segment rather than source names or
raw backend label strings. This keeps the representation independent of the
original origin and avoids exposing backend naming conventions.

## Proposed Output Metadata

Introduce a final-emission record along these lines:

```text
CodegenRelocation {
    value_offset,   // first relocated byte within CodegenOutput::bytes
    target_offset,  // target offset within the same main segment
    addend,
    kind,           // Word16 | Address8 | Low8 | High8 | Relative8
}
```

The resolved value at the original origin is:

```text
Word16/Address8/Low8/High8: output.origin + target_offset + addend
Relative8:                  target_offset - address_after_operand
```

Fixed absolute targets are not represented as output-relative relocations.
They remain literal values or fixed semantic equates. Source spans remain in
the emitter long enough to diagnose patch failures but need not be copied into
the final listing metadata.

Records may originate from unresolved label patches or from emission sites
that already know a final internal storage address. The latter must explicitly
record that the operand is output-relative when writing its resolved bytes.
The absence of a record means that the listing must not move the value.
`Address8` represents a direct zero-page address rather than a selected low
byte; a new origin is valid for such a listing only while the relocated target
still fits in page zero.

The finalizer must validate that:

- every relocation range lies within `CodegenOutput::bytes`;
- every output-relative target is a valid boundary within the main segment;
- relocation slots do not overlap;
- the kind width matches the patched range;
- applying the record at the original origin reproduces the final bytes;
- records have deterministic ordering and no accidental duplicates.

Keep the existing byte-only `finish()` helper for focused emitter tests if that
avoids broad churn. Production output paths should use a new finalization
result containing both bytes and resolved relocation records. Classic,
SemIR-native classic, and MIR6502 must all populate the same output shape.

`CodegenOutput` is currently public, so adding a field is a Rust API change for
clients that construct it with a struct literal. Keep the new type small,
document it, update in-tree constructors, and avoid duplicating the table in
both `CodegenOutput` and `CodegenMap`.

## MADS Rendering Rules

The formatter should first build one origin-independent display-symbol table
for every internal relocation target. Prefer existing semantic routine and
storage labels. Allocate a collision-safe generated label only where no
semantic name exists.

Synthetic labels must not encode either an absolute address or a segment
offset. Use deterministic semantic scope plus declaration/program order, for
example:

```asm
loc_main_1:
loc_main_2:
loc_helper_1:
loc_program_1:
data_generated_1:
```

Anonymous code targets inside a known routine use that routine's sanitized
display name and a per-routine ordinal. Targets outside a known routine use a
program-level ordinal. Anonymous data targets use a distinct data prefix and
an owner name when one is available. Allocate semantic names first, then
synthetic names through the existing case-insensitive collision allocator.
Assign ordinals by deterministic program order so changing only the origin
cannot change any generated name.

Definitions must be possible at any emitted byte boundary, not only at the
current `ListingItem` boundaries. Split data runs when a symbol or relocation
target falls inside them. A storage object inside a descriptor/data run must
become a real label rather than an origin-baked equate. Fixed-address storage
outside the emitted segment remains an equate.

Render output-relative relocations with MADS expressions:

| Relocation position | MADS form |
| --- | --- |
| 16-bit instruction operand | `LDA.A symbol+offset` |
| zero-page address operand | `LDA.Z symbol+offset` |
| low-byte immediate | `LDA #<(symbol+offset)` |
| high-byte immediate | `LDX #>(symbol+offset)` |
| 16-bit data | `DTA A(symbol+offset)` |
| low-byte data | `DTA L(symbol+offset)` |
| high-byte data | `DTA H(symbol+offset)` |
| relative branch | `BNE symbol` |

The parentheses in immediate low/high expressions are required when an addend
is present: in MADS, `#>symbol+4` selects the high byte before adding four,
whereas `#>(symbol+4)` selects the high byte of the complete expression.
MADS 2.1.7 accepts the forms shown above.

Keep `.A` and `.Z` suffixes based on the already decoded opcode, even if a new
origin makes another addressing width possible. Re-origining must not alter
instruction length or branch distances.

If a valid relocation occurs in a byte position that the instruction renderer
cannot express safely, emit that instruction or fragment as byte/data
directives containing the relocation expression. Never fall back to the
already-resolved numeric byte and silently lose relocation behavior.

For storage operands with explicit output-relative provenance:

- an exact internal storage base becomes its semantic label;
- an address strictly inside an unambiguous internal storage range becomes
  `label+offset`;
- overlapping aliases use the most specific mapped symbol and deterministic
  tie-breaking;
- fixed-address storage may use a fixed equate plus an offset, but never gains
  output-relative provenance;
- an address with no output-relative record stays numeric, even if its value
  happens to match an internal symbol at the original origin.

The last rule prevents accidental relocation of hardware addresses and numeric
constants that happen to fall inside the output address range.

## Implementation Slices

### Slice 1: Freeze The Re-origining Contract

- Add a focused listing fixture containing word, low-byte, and high-byte
  references to routines, statics, global/local storage, and inline data.
- Include positive and negative addends, a target inside a data run, forward
  and backward branches, a fixed-address variable, an OS/runtime call, and an
  explicit numeric address equal to an address in the original output range.
- Add pure tests for the supported MADS expression spellings and precedence.
- Document which values move and which remain fixed before changing output
  structures.

This slice should not alter generated machine bytes or existing listing text.

### Slice 2: Preserve Final Emitter Relocations

- Add the final output relocation type and validation.
- Generalize emitter finalization to return bytes plus resolved relocation
  records while retaining the simple byte-only helper for unit tests.
- Convert backend label names to output-relative target offsets during
  finalization.
- Thread the records through AST/classic, SemIR-native classic, and MIR6502
  `CodegenOutput` construction.
- Add emitter tests for word, low, high, relative, addend, bounds, overlap,
  duplicate, and original-origin byte consistency cases.

No listing change is required in this slice. Existing load files must remain
byte-for-byte unchanged.

### Slice 3: Complete Internal-address Provenance

- Audit every origin-derived numeric operand emitted by all production
  backends.
- Classify each as fixed absolute, output-relative label, or mapped internal
  storage plus offset.
- Convert genuinely internal numeric materializations to label fixups where
  practical.
- Where conversion to a label patch is impractical, record a resolved
  output-relative reference at the emission site. Do not derive that
  provenance later from address equality.
- Extend final map facts only where an already-proven internal target needs a
  semantic storage base/range for readable `base+offset` rendering.
- Cover array elements, record fields, descriptor/backing distinctions,
  locals, parameters, spills, strings, routine pointers, return addresses,
  machine-block local labels, and self-modifying code targets.
- Add a differential test that compiles focused fixtures at two layout-stable
  origins and verifies that every origin-dependent byte is explained by the
  final relocation or storage-range metadata.

Do not add origin-specific special cases for individual fixtures. If an
internal address cannot be described generally, keep the listing marked
fixed-origin until the missing representation is fixed.

### Slice 4: Make Display Symbols Origin-independent

- Include every relocation target and internal mapped-storage base in display
  symbol discovery.
- Prefer the existing readable semantic labels and collision rules.
- Replace address-derived names such as `L3008` with deterministic scoped
  ordinal names such as `loc_main_1`.
- Never encode an absolute address or segment offset in a synthetic label.
- Allocate routine, program, and anonymous-data ordinal namespaces in
  deterministic program order after reserving semantic names.
- Split listing data items at all required label-definition boundaries.
- Replace origin-baked equates for symbols inside the emitted segment with
  actual definitions.
- Keep external/fixed storage equates numeric.
- Add tests for aliases, symbols inside data chunks, multiple labels at one
  address, generated-name collisions, and independent per-routine ordinals.
- Compile the same layout-stable fixture at two origins and assert that its
  semantic and synthetic symbol names are identical.

This is an intentional formatter contract change. Address information remains
available in trailing comments, where it cannot be confused with symbol
identity after re-origining.

### Slice 5: Render Relocatable Instructions And Data

- Index final relocation records by patched output offset.
- Render full-address instruction operands from their relocation expression.
- Render low/high immediate address bytes with correctly parenthesized MADS
  expressions.
- Render relocated data as `DTA A`, `DTA L`, or `DTA H` while keeping literal
  runs as `.BYTE`.
- Render storage-range references as semantic base-plus-offset expressions.
- Preserve `.A`/`.Z`, inline-call data handling, source comments, routine/data
  boundaries, and deterministic formatting.
- Introduce `ACTIONC_ORIGIN`, retain the fixed `$02E2` segment, and update the
  header from fixed-origin to the precise re-originable contract.
- Share the implementation between plain and source-annotated listings.

The unedited listing must still assemble byte-for-byte identically before the
cross-origin oracle is enabled.

### Slice 6: Add The Cross-origin MADS Oracle

Extend `tools/check-mads-listings.sh` or add a narrowly named companion check
that, for every selected mode:

1. compiles the fixture at origin A;
2. emits both listing variants;
3. assembles them unchanged and compares the complete load file with origin A
   actionc output;
4. changes only `ACTIONC_ORIGIN` to origin B;
5. assembles both edited listings;
6. compares them with a direct origin B compilation for fixtures whose layout
   is known to be origin-stable;
7. reports and retains all artifacts on failure.

Use at least two origin pairs with different high and low bytes so the oracle
cannot pass while testing only one selector. Cover compatibility/classic,
optimized/classic, SemIR-native classic where directly selectable, and
modern/MIR6502.

Add a Rust-side relocation application oracle as well: applying the final
relocation records to the original payload at origin B must reproduce the
bytes emitted by the edited MADS listing. The direct-recompilation comparison
remains important because it detects missing relocation records that a
self-derived relocation oracle would miss.

### Slice 7: Documentation And Consumer Migration

- Update README and CLI help to describe listings as re-originable rather than
  generally relocatable.
- Document the one-definition edit and warn users not to change `ORG $02E2`.
- Explain fixed numeric addresses, stale original-byte comments, valid-origin
  responsibility, and the lack of runtime relocation metadata.
- Update listing assertions, snapshots, and comparison-tool parsers affected
  by the origin definition or new `DTA` forms.
- Mark the fixed-origin plan as superseded only after all acceptance checks
  pass.

No NIR fixture change is expected. Classify listing changes as formatter-only;
any generated-byte change requires separate investigation.

## Test Strategy

### Emitter And Metadata Tests

- every patch kind survives finalization with the correct offsets and addend;
- applying a relocation at the original origin reproduces resolved bytes;
- invalid targets, widths, overlaps, and ranges are rejected;
- explicit absolute values do not acquire relocation records;
- all production output paths retain equivalent metadata.

### Pure Formatter Tests

- one and only one editable origin definition is emitted;
- `RUNAD` stays at `$02E2` and refers symbolically to the relocated run label;
- word/low/high instruction and data references use the correct MADS forms;
- addends are parenthesized and preserve negative/positive meaning;
- internal storage offsets are symbolic and fixed storage remains fixed;
- labels can be defined inside formerly contiguous data runs;
- synthetic labels contain neither original absolute addresses nor segment
  offsets and remain identical across origins;
- literal bytes that resemble address bytes remain literal;
- plain and source listings contain identical assembly statements;
- formatting is deterministic and ASCII-safe.

### Cross-origin Tests

- unchanged listing -> byte-identical actionc load file at origin A;
- edited origin only -> correct load file at origin B;
- at least one origin pair changes both low and high address bytes;
- relocated entry point in `RUNAD` is correct;
- relative branches and explicit instruction widths remain unchanged;
- classic and MIR6502 satisfy the same artifact contract;
- representative static-initializer and inline-assembly relocations move;
- explicit fixed addresses remain byte-identical.

After focused fixtures pass, sweep maintained small samples. Treat TN and
other large programs as a reportable validation sweep rather than making them
part of every unit-test invocation.

## Risks And Guardrails

- **Lost provenance:** never classify a value as relocatable solely by numeric
  equality. Require an emitter patch or an explicit resolved-reference record
  created at the emission site. Use maps only to name a proven target.
- **Storage interior references:** render `base+offset`; exact-base-only lookup
  is insufficient for arrays, records, and multi-byte slots.
- **Low/high precedence:** always parenthesize the complete expression before
  applying `<` or `>` in immediate operands.
- **Addressing relaxation:** retain `.A` and `.Z` from the emitted opcode.
- **Data splitting:** labels and directives may split presentation runs but
  must neither omit nor duplicate payload bytes.
- **Backend drift:** all production backends must use the same final metadata
  and formatter contract.
- **Origin-sensitive compilation:** use deliberately layout-stable fixtures for
  direct A/B byte comparison and describe the broader semantic contract
  separately.
- **Public API churn:** add one documented output metadata field rather than
  parallel backend-specific structures.
- **False external-oracle success:** compare complete XEX/COM files including
  headers and `RUNAD`, and retain artifacts on failure.

## Acceptance Criteria

The feature is complete when:

- generated listings identify themselves as re-originable MADS assembly;
- synthetic labels use stable scope/order names and never encode addresses or
  segment offsets;
- changing only `ACTIONC_ORIGIN` relocates every generated internal address in
  the focused contract fixture;
- explicit fixed addresses remain fixed;
- low/high/word relocations and addends work in instructions and data;
- internal storage-plus-offset references are covered without numeric
  guessing;
- unedited listings retain their byte-identical full-load-file round trip;
- edited listings pass the cross-origin MADS oracle for all supported backend
  modes;
- both listing variants contain identical assembly statements;
- no backend-generated machine bytes change at the original origin;
- normal compiler use has no MADS runtime dependency;
- documentation distinguishes re-originable assembly from runtime relocation;
- all required project checks pass.

Required final checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
ACTIONC_MADS=mads tools/check-mads-listings.sh
```

Run the new cross-origin oracle explicitly as part of the final slice if it is
kept separate from `check-mads-listings.sh`.

## Suggested Commit Sequence

Keep each major slice independently buildable and leave generated bytes
unchanged until listing rendering intentionally changes:

```text
tests: freeze re-originable MADS listing contract
codegen: retain resolved emission relocations
codegen: complete internal address provenance
artifacts: generate origin-independent scoped listing symbols
artifacts: render re-originable MADS expressions
tools: verify MADS listings across origins
docs: document re-originable generated listings
```
