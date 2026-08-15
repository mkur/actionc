# MADS-Compatible Listing Implementation Plan

Snapshot date: 2026-08-13.

Status: implemented; its fixed-origin limitation is superseded by
`REORIGINABLE_MADS_LISTING_IMPLEMENTATION_PLAN.md`.

The existing `--listing`, `--emit-listing`, and `--emit-source-listing`
interfaces now share one MADS-compatible formatter. The optional
`tools/check-mads-listings.sh` oracle validates complete load-file round trips
across compatibility/classic, optimized/classic, and modern/MIR6502.

## Goal

Make the existing generated listings valid MADS assembly that can be assembled
into the same Atari load file produced by `actionc`.

The first contract is deliberately fixed-origin and byte preserving:

```text
Action source
    -> existing compiler pipeline
    -> final CodegenOutput
        -> actionc Atari load file
        -> MADS-compatible assembly source
            -> MADS Atari load file
```

For an unedited export, the two Atari load files must be byte-for-byte
identical, including segment headers and `RUNAD`. MADS remains an optional
external validation tool; neither `actionc` nor the compiler library gains a
runtime dependency on MADS.

Before this work, the listing was close to MADS syntax but remained a
presentation artifact:

```text
3000  4C 03 30  JMP $3003
```

Its address and byte columns were not assembler input, it had no `ORG`, and it
did not emit the load-file `RUNAD` segment. Simply stripping those columns was
also unsafe because MADS may shorten an absolute operand in page zero unless
the original addressing width is made explicit.

A hand-written conversion of the current Hello World listing, with an origin,
address-width suffixes, and `RUNAD`, has already been checked with MADS 2.1.7.
MADS reproduced the complete 41-byte load file exactly. That probe established
the contract now maintained by the formatter and regression workflow.

There is one listing syntax, not separate human and MADS formats. Address
and generated-byte information moves from presentation columns into comments,
so the listing remains useful for inspection and diffs while also becoming
assembler input.

## User Interface

Keep the existing interfaces:

```sh
actionc-emit --emit-listing samples/hello-world.act > build/hello-world.asm
mads build/hello-world.asm -o:build/hello-world.xex -s

actionc samples/hello-world.act \
  --output build/hello-world.com \
  --listing build/hello-world.asm
```

`--emit-listing` produces MADS assembly with routine/data boundary comments.
`--emit-source-listing` produces the same assembly with additional Action!
source-location comments. The `actionc --listing <file>` output continues to
use the source-annotated form through `CompiledProgram::source_listing()`.

No new listing option or output path is added. The existing object/listing path
collision checks and atomic-write behavior remain in effect.

This is an intentional listing contract change. Tools or tests that parse the
old leading address and byte columns must be migrated to consume the new
instruction text and trailing comments. The listing should not become a
structured interchange format; maps remain the appropriate interface for
machine-readable address metadata.

The generated file calls itself a fixed-origin MADS assembly listing in its
header comment. It does not claim to be original source or relocatable compiler
IR.

## Output Contract

For every successful compilation supported by this feature, both
`--emit-listing` and `--emit-source-listing` must satisfy the assembly contract:

- the exported source is deterministic;
- MADS 2.1.7 can assemble it with the documented command;
- every byte in `CodegenOutput::bytes` is emitted exactly once and in order;
- the first MADS segment starts at `CodegenOutput::origin`;
- the final segment writes `CodegenOutput::run_address` to `$02E2`;
- the assembled load file equals `format_load_file(output)`, not merely its
  main payload;
- source comments and readable routine/data boundaries are retained where the
  code map provides them, with source excerpts included only in the
  source-listing form;
- routine entries, global storage, parameters, and locals use deterministic,
  collision-safe MADS labels derived from existing code-map facts, while
  control-flow-only targets retain address-derived labels;
- symbolic operands retain explicit `.a` and `.z` suffixes, so replacing a
  numeric address with a semantic label cannot change instruction width;
- changing listing presentation cannot change compiler lowering or emitted
  machine code.

Uninitialized or fixed-address storage that is absent from the Atari load file
must remain absent from the MADS export. This is a reconstruction of the final
load artifact, not a full Atari memory image.

The supported MADS invocation must use ordinary Atari executable output. Raw
binary options such as `-b` are outside the contract because they omit the
segment structure being compared.

## Example Shape

The exact whitespace can be settled by formatter tests, but the generated
shape should resemble:

```asm
; Generated by actionc. Fixed-origin MADS assembly listing.
; Reassembly is expected to reproduce the unedited actionc load file.

        org $3000

; ===== PROC Main $3000..$301C =====
proc_main:
; 3:3 statement call | PrintE("Hello, world!")
        jmp.a L3014             ; $3003: 4C 14 30

; ===== DATA inline string literal $3006 =====
        .byte $0D,$48,$65,$6C,$6C,$6F,$2C,$20
        .byte $77,$6F,$72,$6C,$64,$21

L3014:
        ldx #$30                ; $3014: A2 30
        lda #$06                ; $3016: A9 06
        jsr.a $A46C             ; $3018: 20 6C A4 ; PrintE
        rts                     ; $301B: 60
; ===== END PROC Main =====
        .byte $60               ; $301C: 60

; Atari RUNAD segment.
        org $02E2
        dta a(proc_main)
```

Generated byte and address columns move into comments so they remain useful
when reading or editing the file. The byte comments describe the original
compiler artifact; editing the assembly is allowed to make those comments
stale and is not part of the byte-identical round-trip guarantee.

## Ownership And Architecture

This is an artifact-formatting feature.

- SemIR continues to own Action! meaning.
- NIR continues to own normalized typed computation.
- MIR6502 continues to own target strategy and physical machine choices.
- Existing classic or MIR6502 emission continues to produce final bytes,
  addresses, maps, and `RUNAD`.
- `src/compiler/artifacts.rs` consumes that final `CodegenOutput` and formats
  both listing variants with one MADS-compatible renderer. It derives display
  aliases from `routine_addresses`, `routine_ranges`, and `storage_symbols`;
  it does not recover names from SemIR or NIR.
- CLI code continues to select and write the existing listing artifacts.

The formatter must not inspect SemIR or NIR, choose instructions, allocate
storage, relocate compiler identities, or run a second optimization path. Both
backends must use the same formatter after producing `CodegenOutput`.

The current `listing_items` stream in `src/compiler/artifacts.rs` already walks
routine instructions, known data ranges, storage initializers, and remaining
bytes in final address order. Reuse or minimally generalize that stream. Do not
duplicate code/data range discovery in parallel formatters.

## Instruction Rendering

Render from `DisassembledInstruction` fields, especially `AddressingMode`, not
by parsing the current human-readable `text` field or mechanically stripping
columns from the old listing.

Use the decoded addressing mode to preserve instruction width:

| Decoded mode | MADS form |
| --- | --- |
| implied | `rts` |
| accumulator | `asl` (MADS treats `asl a` as a reference to label `A`) |
| immediate | `lda #$12` |
| zero page | `lda.z $80` |
| zero page X/Y | `lda.z $80,x` |
| absolute | `lda.a $0080` |
| absolute X/Y | `lda.a $0080,x` |
| indirect | `jmp ($1234)` |
| indexed indirect X | `lda ($80,x)` |
| indirect indexed Y | `lda ($80),y` |
| relative | `bne L3040` or a fixed numeric target |

The `.z` and `.a` suffixes are encoding requirements, not cosmetic choices.
They prevent MADS from selecting a different zero-page or absolute opcode when
the operand value fits both forms. Apply them consistently whenever MADS
supports both widths, including low-valued absolute operands introduced by
fixed storage or generated machine code.

Official decoded NMOS 6502 instructions must have an explicit renderer for
their mode. Bytes that the disassembler intentionally does not recognize stay
`.byte` data. Do not silently use `.byte` as a permanent fallback for a decoded
instruction whose MADS rendering has not been implemented; add the renderer
and a focused test instead.

Inline bytes following special runtime calls, static data, and bytes outside a
known instruction range remain explicit `.byte` directives. The formatter
must preserve the existing special treatment of inline `r_Par`/SARGS data.

## Labels And Comments

Use address-derived labels such as `L3040` for internal branch, jump, and call
targets. They are deterministic, valid in MADS, and independent of Action!
case-folding or source naming rules.

- Emit a generated label only for a target inside the main output segment and
  on a known item boundary.
- Use the generated label in relative branches and direct `JMP`/`JSR` operands
  when available.
- Keep resident-library, OS, hardware, and other external targets numeric.
- Keep Action! routine and storage names in comments during the first slice.
- Do not expose raw Action! names as assembler symbols until collision,
  character-set, scope, and case-folding rules have a separate design.

This avoids label collisions between routines, locals, statics, builtins, and
MADS reserved words while retaining readable source and map annotations.

All generated assembly syntax must be ASCII. Sanitize source excerpts and
display names before placing them in comments so ATASCII bytes, control
characters, or non-ASCII source text cannot make an otherwise valid MADS file
unreadable. Comment sanitization is presentation-only and must not alter code
or data bytes.

## Implementation Slices

### Slice 1: Freeze The Contract With Focused Fixtures

Status: complete.

Add small fixtures that cover:

- a routine with forward and backward relative branches;
- direct `JMP` and internal/external `JSR` targets;
- zero-page and absolute accesses to values below `$0100`;
- absolute X/Y and zero-page X/Y forms;
- indexed-indirect and indirect-indexed pointer accesses;
- accumulator and implied instructions;
- inline string/static data and an undecoded byte;
- inline parameter bytes after a recognized runtime call;
- a run address different from the main segment origin;
- source comments containing characters that require sanitization.

Keep these fixtures small enough that an encoding mismatch points directly to
one formatter rule.

### Slice 2: Convert The Shared Listing Formatter

Status: complete.

In `src/compiler/artifacts.rs`:

1. minimally generalize the existing final-address `listing_items` traversal;
2. collect internal control-flow targets and assign stable address labels;
3. add an addressing-mode-driven MADS instruction renderer;
4. add a `.byte` renderer for data items;
5. add ASCII-safe source and boundary comments;
6. emit the main `ORG` before the payload;
7. emit the `$02E2` `RUNAD` segment using `output.run_address`;
8. share the assembly renderer between the boundary-only and source-annotated
   variants;
9. return one deterministic string with a trailing newline policy covered by
   tests.

`format_listing_with_boundaries` and `format_listing_with_source` keep their
API roles but intentionally change their textual contract. The only difference
between them should be the additional Action! source comments.

### Slice 3: Migrate Listing Consumers

Status: complete.

No CLI parsing or compiler API expansion is required. Keep
`--emit-listing`, `--emit-source-listing`, `--listing`, and
`CompiledProgram::source_listing()` wired as they are.

Update consumers of the old columns in the same vertical slice:

- adjust CLI and compiler API assertions to the MADS-compatible shape;
- update formatter snapshots and document them as an intentional listing
  contract change;
- update `tools/compare-codegen.sh` normalization, operation extraction, and
  byte-count/symbol-summary logic;
- keep routine/data comments regular enough for comparison reports, or move
  address-sensitive reporting to the existing map artifact;
- verify `tools/test-compare-codegen.sh` remains green;
- update help and usage descriptions without adding new flags.

Do not make `actionc` invoke MADS. Users assemble or edit the generated listing
separately.

### Slice 4: Add The External MADS Oracle

Status: complete.

Add a developer check, for example:

```sh
tools/check-mads-listings.sh
```

The check should:

1. locate MADS from `ACTIONC_MADS` or `PATH`;
2. report the detected version;
3. generate MADS source and the compiler load file in a fresh temporary
   directory;
4. assemble with the documented non-interactive command;
5. compare the complete output files with `cmp`;
6. retain or report useful artifacts on failure;
7. cover compatibility/classic, optimized/classic, and modern/MIR6502.

Absence of MADS must be reported clearly. If the oracle is added to CI, pin the
MADS version and verify the downloaded artifact rather than silently testing
against an arbitrary release. The Rust test suite must still run without an
installed MADS executable.

## Test Strategy

Use three layers of tests.

### Pure Formatter Tests

- Snapshot a small complete MADS export.
- Assert both `ORG` blocks and the exact `RUNAD` value.
- Assert `.a` on low-valued absolute modes and `.z` on zero-page modes.
- Assert internal labels are defined exactly once and external targets stay
  numeric.
- Assert data and inline-call metadata use `.byte`.
- Assert comments contain only the allowed textual character set.
- Assert repeated formatting is deterministic.
- Assert boundary-only and source-annotated listings share identical assembly
  statements.
- Update existing listing snapshots as an intentional formatter contract
  change; generated machine bytes must remain unchanged.

### CLI Tests

- `--emit-listing` and `--emit-source-listing` each produce valid MADS source.
- Their existing mutual-exclusion behavior remains unchanged.
- `actionc --listing` writes the same source-annotated form atomically.
- Failed compilation leaves no object or listing behind.
- Explicit modes, low-level profile/backend flags, and source annotations use
  the expected backend before formatting.
- `tools/compare-codegen.sh` still produces useful normalized, operations, and
  symbol-summary artifacts from the new format.

### MADS Round-Trip Tests

For each oracle fixture:

```sh
actionc-emit <compiler options> --emit-load source.act > actionc.xex
actionc-emit <compiler options> --emit-listing source.act > actionc.asm
mads actionc.asm -o:mads.xex -s
cmp actionc.xex mads.xex
```

Repeat representative cases with `--emit-source-listing` so source comments
are also proven harmless to MADS.

Compare the entire load file. A payload-only comparison would miss an incorrect
origin, segment boundary, or `RUNAD`.

Include at least this initial matrix:

| Profile/backend | Fixture purpose |
| --- | --- |
| compatibility/classic | ordinary legacy-compatible code and resident calls |
| optimized/classic | shortened and removed instruction shapes |
| modern/MIR6502 | materialized MIR output and generated control flow |
| both backends | low-address absolute versus zero-page encoding |
| both backends | inline data, storage initializers, and machine-block bytes |

After focused fixtures pass, run the oracle across maintained small samples.
Treat TN and the Toolkit as a reportable sweep rather than making every normal
unit-test invocation assemble large programs externally.

## Documentation

Update:

- `README.md` with one generate-and-assemble example;
- `USAGE.md` with the exact fixed-origin and byte-identity contract;
- `actionc` and `actionc-emit` help descriptions, without adding options;
- `docs/README.md` to link this plan while it remains active.

Document MADS as an optional consumer of the artifact, not a compiler
requirement. State the validated MADS version and command. Explain that both
listing forms are assembler input and that the source-listing form adds only
comments.

## Rejected Alternatives

### Add A Parallel MADS Listing Mode

Rejected because it would duplicate formatter behavior, CLI surface,
documentation, snapshots, and consumer expectations. The existing address and
byte information fits naturally in assembly comments, so one listing syntax
can serve inspection, diffing, source annotation, and reassembly.

### Strip Listing Columns With A Script

Rejected as the product contract. A text filter cannot reliably preserve
absolute versus zero-page encoding, identify all embedded data, emit `RUNAD`,
or remain stable when the human listing changes.

### Emit Only `.byte` For The Main Payload

This would reproduce bytes but would not provide a useful assembly listing.
`.byte` remains the correct representation for data and unknown opcodes, not
for all generated instructions.

### Generate MADS From NIR Or MIR6502

Rejected because it would create another lowering/emission path and risk
reconstructing final placement facts. The final `CodegenOutput` already owns
the bytes and map needed for an exact artifact formatter.

### Invoke MADS From `actionc`

Rejected for the initial feature. It would add executable discovery, version
compatibility, process management, and failure-policy concerns without being
needed to export assembly.

## Risks And Guardrails

- **Address-width relaxation:** always render from `AddressingMode` and force
  `.a`/`.z` where needed.
- **Branch drift:** emit fixed-width instructions and stable labels before
  comparing bytes; no formatter directive may insert payload bytes.
- **Inline data mis-disassembly:** reuse current data/routine/source maps and
  special inline-call metadata handling.
- **Invalid source comments:** sanitize comments to ASCII assembler-safe text.
- **Label collisions:** use address labels only in the first implementation.
- **Backend divergence:** both classic and MIR6502 must format the same final
  output type through one implementation.
- **False oracle success:** compare full XEX/COM files, including headers and
  `RUNAD`, and fail on a missing MADS output.
- **External-tool churn:** pin and report the oracle version; keep normal builds
  independent of MADS.

## Acceptance Criteria

The feature is complete when:

- `actionc-emit --emit-listing` produces readable MADS input;
- `actionc-emit --emit-source-listing` produces the same assembly with Action!
  source comments;
- `actionc --listing <file>` writes the source-annotated artifact atomically;
- no parallel MADS listing mode or output path is introduced;
- generated compiler bytes are unchanged;
- listing snapshots and comparison tools are intentionally migrated to the new
  textual contract;
- the focused MADS 2.1.7 oracle matrix reassembles byte-for-byte identical
  complete load files;
- absolute and zero-page modes are protected by explicit regression tests;
- inline data and `RUNAD` round-trip correctly;
- classic and MIR6502 use the same formatter;
- help and user documentation explain the fixed-origin contract;
- all required project checks pass.

Required checks after implementation:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
ACTIONC_MADS=mads tools/check-mads-listings.sh
```

No NIR fixture change is expected. If any compiler or listing fixture changes,
classify it explicitly as a formatter-only change or investigate it as an
unintended code-generation change.

## Suggested Commit Sequence

Keep the work in independently verifiable slices:

```text
artifacts: make generated listings valid MADS assembly
tools: migrate listing comparison consumers
tools: verify MADS listing round trips
docs: document MADS-compatible listings
```
