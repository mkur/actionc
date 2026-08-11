# Reusable Compiler API Implementation Note

Status: implemented through Slice 5; ready for `actionc-run` integration.

Snapshot date: 2026-08-11.

## Goal

Provide one supported Rust entry point that compiles an Action! source file and
returns an Atari load-format object without printing, writing output files, or
terminating the process.

The immediate consumer is `actionc-run`:

```rust
let compiled = compile_file(
    source_path,
    &CompileOptions::for_mode(CompileMode::Compatibility),
)?;

let mut disk = AtrImage::from_bytes(MYDOS_ATR)?;
disk.upsert_file("AUTORUN.AR0", compiled.object_bytes())?;
```

The existing `actionc` executable must eventually use the same entry point. A
successful API migration therefore means that the CLI and `actionc-run` cannot
silently acquire different compiler pipelines.

## Current Boundary

`src/cli.rs` currently owns all of these jobs:

- command-line parsing and help;
- input and include loading;
- source-level profile/backend annotation handling;
- semantic analysis;
- SemIR, NIR, and MIR6502 orchestration;
- classic backend selection;
- origin selection;
- diagnostic formatting and `stderr` output;
- object and listing construction;
- atomic output-file writes;
- exit-code policy and `process::exit` calls.

The lower compiler layers already return `Result` in most places. The missing
piece is a side-effect-free orchestration layer that translates their different
error types into one compiler-facing result.

The intended boundary is:

```text
CLI / actionc-run
       |
       v
compiler API: configuration, orchestration, diagnostics, artifacts
       |
       +-- input/include loading
       +-- AST + semantic analysis
       +-- classic backend
       `-- SemIR -> NIR -> MIR6502 backend
```

This does not change ownership inside the compiler pipeline. In particular,
SemIR still owns Action! meaning, NIR still owns normalized typed computation,
and MIR6502 still owns target strategy.

## Public API Shape

Add a public `compiler` module, initially backed by files rather than an
in-memory or virtual filesystem:

```rust
pub enum CompileMode {
    Compatibility,
    Optimized,
    Mir6502,
}

pub struct CompileOptions {
    // Private fields; construct through the methods below.
    mode: Option<CompileMode>,
    origin: Option<u16>,
}

impl CompileOptions {
    pub fn for_mode(mode: CompileMode) -> Self;
    pub fn with_origin(self, origin: u16) -> Self;
}

impl Default for CompileOptions {
    // Preserve current `actionc file.act` behavior. Compiler annotations may
    // fill settings that were not explicitly selected by the caller.
}

pub fn compile_file(
    path: impl AsRef<Path>,
    options: &CompileOptions,
) -> Result<CompiledProgram, CompileError>;
```

`CompileOptions::for_mode` is explicit and overrides source-level compiler
selection annotations, matching `actionc --mode`. `CompileOptions::default()`
retains the existing implicit compatibility defaults while allowing the source
annotations currently honored by `actionc` to fill unspecified settings.

The public modes map exactly to the current CLI presets:

| API mode | Profile | Backend | Backend configuration |
| --- | --- | --- | --- |
| `Compatibility` | compatible/legacy | classic | current compatibility path |
| `Optimized` | modern | classic | current modern classic path |
| `Mir6502` | modern | MIR6502 | optimized MIR6502 configuration |

Do not expose `CodegenSource`, raw profile/backend pairs, NIR pass switches, or
MIR peephole flags in the first public API. They remain internal development
controls. This keeps `actionc-run` tied to the three user-facing modes rather
than to experimental implementation details.

`actionc` still needs its advanced flags during migration. Support them through
a crate-private request type:

```rust
pub(crate) struct CompileRequest {
    profile: Option<CodegenProfile>,
    backend: Option<Backend>,
    codegen_source: CodegenSource,
    mode: Option<CompileMode>,
    origin: Option<u16>,
}

pub(crate) fn compile_file_with_request(
    path: &Path,
    request: &CompileRequest,
) -> Result<CompiledProgram, CompileError>;
```

The public `compile_file` converts `CompileOptions` into this internal request.
The advanced type is not a compatibility promise to external callers.

## Compilation Result

The returned object must be immediately useful to `actionc-run` while retaining
enough information for `actionc --listing`:

```rust
pub struct CompiledProgram {
    object: Vec<u8>,
    output: CodegenOutput,
    expanded_source: String,
}

impl CompiledProgram {
    /// Complete Atari load file, including segment headers and RUNAD.
    pub fn object_bytes(&self) -> &[u8];

    /// Source-oriented listing in the current `actionc --listing` format.
    pub fn source_listing(&self) -> String;

    pub fn origin(&self) -> u16;
    pub fn run_address(&self) -> u16;
}
```

`object_bytes` must not mean raw generated code. It is the complete output of
the existing `format_load_file` operation and can be copied directly into an
ATR as an executable file.

The `CodegenOutput` and expanded source remain private initially. Keeping them
inside the result lets listing generation stay lazy: `actionc-run` pays no
listing/disassembly cost, while `actionc` can request the listing after a
successful compile. A map accessor can be added later if a real consumer needs
it; it is not required for `actionc-run`.

Move the source-listing formatter and its disassembly helpers out of `cli.rs`
into a pure compiler artifact module. Formatting must not read files, print, or
exit.

## Errors And Diagnostics

The library must not call `process::exit`, write to stdout/stderr, or return
already-printed errors. Normalize the current frontend, semantic, NIR, MIR6502,
codegen, and input failures into a single error:

```rust
pub struct CompileError {
    kind: CompileErrorKind,
    diagnostics: Vec<CompilerDiagnostic>,
}

pub enum CompileErrorKind {
    Configuration,
    Compilation,
}

pub enum CompilerPhase {
    Configuration,
    Input,
    Frontend,
    Semantic,
    Nir,
    Mir6502,
    Codegen,
}

pub struct CompilerDiagnostic {
    pub phase: CompilerPhase,
    pub message: String,
    pub site: DiagnosticSite,
}

pub enum DiagnosticSite {
    Source {
        path: PathBuf,
        line: usize,
        column: usize,
        byte_range: Option<Range<usize>>,
        excerpt: Option<String>,
    },
    Ir {
        routine: Option<String>,
        block: Option<String>,
    },
    Unknown,
}
```

`CompileError` should implement `Display` and `std::error::Error`, but callers
must also be able to inspect its kind and all diagnostics. The CLI owns final
text rendering. Compilation failures map to exit code 1; invalid resolved
profile/backend combinations map to exit code 2. Argument syntax and value
parsing remain outside `compile_file` and continue to produce exit code 2
directly in the CLI.

The first implementation should preserve current diagnostic locations and
messages. Include expansion currently cannot always retain a complete source map
when loading fails; improving that is useful, but it is not a prerequisite for
the API extraction and must not be mixed into the first patch.

The existing legacy-routine-retargeting check and NIR/MIR verifier failures are
compiler failures, not CLI policy. Move their non-printing forms behind the
compiler API. The CLI may format the resulting diagnostics, but it must not run
a separate validation path.

## Origin Contract

Represent origin selection as `Option<u16>` rather than using `$3000` as a
sentinel:

- `None` means apply the existing source/default origin rules, falling back to
  `$3000`;
- `Some(address)` means use exactly that address and override source origin
  directives;
- `Some($3000)` is therefore an explicit override, not the same state as
  `None`.

The classic and MIR6502 paths currently reach source-origin selection through
different helpers. The API should centralize the selection contract while
leaving backend-specific layout work in the backend. Add a regression for an
explicit `$3000`, because the current classic API uses `$3000` as both a real
address and a default marker.

## Side-Effect Contract

`compile_file` may:

- read the root source file;
- resolve and read Action! include files relative to their including files;
- allocate memory and perform compiler work.

It must not:

- create `.com`, listing, map, temporary, or cache files;
- print diagnostics or progress;
- read emulator, ATR, cartridge, or OS configuration;
- launch a process;
- terminate the caller;
- depend on the current working directory except through a relative input path
  supplied by the caller.

Output-path validation, directory creation, atomic writes, and exit codes remain
the responsibility of `actionc`. ATR construction and emulator launching belong
to `actionc-run`.

## Proposed File Layout

Start with a small module tree:

```text
src/compiler/mod.rs          public facade and orchestration
src/compiler/diagnostics.rs  normalized compiler diagnostics
src/compiler/artifacts.rs    load object and source-listing construction
src/compiler/validation.rs   compiler-wide pre-backend validation
```

`src/cli.rs` should retain argument parsing, help text, terminal rendering,
output-path policy, atomic writes, and exit-code mapping. It should not contain
backend lowering or emission decisions after the migration.

## Implementation Slices

### Slice 1: characterize the current contract

Add regression coverage before moving orchestration:

- compile `samples/hello-world.act` in all three modes and record exact object
  equality against the current CLI/emit path;
- compile a source with includes and verify mapped diagnostics;
- cover implicit source annotations and explicit-mode precedence;
- cover implicit origin, explicit non-default origin, and explicit `$3000`;
- confirm failed compilation creates no output through the existing CLI.

No production behavior changes in this slice.

### Slice 2: introduce the file-based facade

- Add `CompileMode`, `CompileOptions`, `CompiledProgram`, and `CompileError`.
- Implement the compatibility/classic route first.
- Reuse `load_program_with_expanded_source`, `analyze`, and the current classic
  generator.
- Return a complete load-format object without writing it.
- Keep `actionc` on its existing route until the new result is byte-identical.

Acceptance criteria:

- A library test compiles Hello World without spawning `actionc`.
- The returned bytes exactly match `actionc-emit --emit-load`.
- Invalid source returns diagnostics and does not print or exit.

### Slice 3: add optimized and MIR6502 modes

- Move mode-to-profile/backend mapping into the compiler module.
- Move source annotation precedence into the shared request resolver.
- Route MIR6502 through SemIR, verified/optimized NIR, MIR lowering,
  materialization, verification, and emission exactly as the CLI does today.
- Convert all validation and optimizer helpers from `*_or_exit` to `Result`.
- Implement the explicit-origin contract for both backends.

Acceptance criteria:

- All three API modes match the current object bytes.
- `CompileOptions::for_mode` overrides source annotations.
- Default options preserve current annotation behavior.
- NIR and MIR failures are returned as structured diagnostics.

If this slice touches NIR lowering, verification, or optimization behavior, run
the NIR checks required by `AGENTS.md`. A pure orchestration move should not
change NIR snapshots.

### Slice 4: extract artifact formatting

- Move `format_listing_with_source` and the helper code it depends on out of
  `cli.rs`.
- Make `CompiledProgram::source_listing` call the pure formatter.
- Keep the listing text byte-for-byte identical, including routine boundaries,
  labels, inline-JSR data, source excerpts, and storage ranges.

Acceptance criteria:

- Existing listing tests remain unchanged.
- Default compilation does not construct a listing unless requested.
- Object bytes and map data are unchanged.

### Slice 5: migrate `actionc`

- Leave CLI parsing and output-path checks in `cli.rs`.
- Convert CLI selections into `CompileOptions` or the crate-private advanced
  request.
- Call the compiler API once.
- Render returned diagnostics.
- Write object and optional listing with the existing atomic writer.
- Delete the duplicate compile orchestration from `cli.rs` only after parity
  tests pass.

Acceptance criteria:

- Existing `tests/actionc_cli.rs` passes without expected-output changes.
- The CLI remains silent on stdout after successful compilation.
- Default filenames and atomic-write behavior are unchanged.
- Syntax/configuration errors remain exit code 2; compilation errors remain
  exit code 1.

### Slice 6: consume the API from `actionc-run`

- Add the root dependency on the in-tree ATR library.
- Compile with `CompileOptions::for_mode`.
- Copy `CompiledProgram::object_bytes()` into embedded `MYDOS_ATR` as
  `AUTORUN.AR0`.
- Do not create an intermediate `.com` file.

This slice belongs to the `actionc-run` implementation, but it is the first
external acceptance test for the API boundary.

### Follow-up: share frontend state with `actionc-emit`

`actionc-emit` currently needs intermediate AST/SemIR/NIR/MIR forms rather than
only a final object. Do not expose those forms in the first public API merely to
remove duplication. After `actionc` and `actionc-run` are stable, introduce a
crate-private compilation session for the emit tool if the duplication is still
material:

```text
loaded source -> analyzed session -> requested intermediate/final artifact
```

This is a follow-up, not a blocker for `actionc-run`.

## Validation Matrix

At minimum, cover:

| Case | Expected check |
| --- | --- |
| compatibility | API object equals current legacy/classic object |
| optimized | API object equals current modern/classic object |
| MIR6502 | API object equals current modern/MIR6502 object |
| include tree | relative and case-insensitive include resolution unchanged |
| source annotations | implicit settings honored; explicit mode wins |
| origin | implicit, explicit non-default, and explicit `$3000` |
| invalid syntax | structured source diagnostic, no process exit |
| semantic failure | mapped source diagnostic |
| NIR/MIR failure | phase plus routine/block context where available |
| listing | exact text equality with current `--listing` |
| path with spaces | compile succeeds on supported hosts |
| repeated calls | no leaked global configuration between compilations |
| concurrent calls | independent compilations do not share mutable state |

Run after the final CLI migration:

```sh
cargo test
cargo run --quiet --bin actionc -- \
  --mode compatibility --output target/compiler-api/compat.com \
  samples/hello-world.act
cargo run --quiet --bin actionc -- \
  --mode optimized --output target/compiler-api/optimized.com \
  samples/hello-world.act
cargo run --quiet --bin actionc -- \
  --mode mir6502 --output target/compiler-api/mir6502.com \
  samples/hello-world.act
```

Also compile TN in all three modes and compare object sizes and hashes with a
pre-migration baseline. This is a parity check, not an opportunity to change
generated code.

## Non-Goals

Do not add these to the first implementation:

- `actionc --run`;
- emulator, cartridge, ATR, DOS, or temporary-directory options;
- an in-memory include resolver or virtual filesystem;
- incremental compilation or caching;
- asynchronous compilation;
- a plugin backend interface;
- public AST, SemIR, NIR, or MIR session objects;
- public switches for individual optimizer or MIR materialization passes;
- compiler-owned output-file writing.

The first API should solve one concrete problem well: compile a file into a
load-format object in-process, with predictable settings and inspectable errors.

## Completion Criteria

The reusable API is ready for `actionc-run` when:

- `actionc` and direct API calls use one final-object compilation path;
- all three user modes are supported;
- no library path prints, exits, or writes output files;
- object and listing output match the pre-migration behavior;
- diagnostics are returned with the best currently available source or IR
  context;
- explicit origin and source-annotation precedence have tests;
- `actionc-run` can place the returned object in an ATR without spawning
  `actionc` or creating a temporary `.com` file.
