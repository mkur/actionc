# MIR68K public CLI, native artifacts and benchmark coverage

Status: implementation in progress. Slices 2 (artifact I/O), 1 (CLI) and 3
(artifact execution) and slice 4a (Statemate) are complete. Dijkstra and Huffman
decoding remain. Baseline: `c753845`, with compiler behavior and measurements
at `8ecf73a`.

Slice 2 validation: all 69 native tests pass, including four artifact tests;
the paired C command retains every measurement in `mir68k-c-comparison.csv`.
The final metadata projection adjustment also passed the artifact target.

Slice 1 validation: native CLI (5), Atari CLI (42), emit modes (3), compiler
API (28), NIR CLI (7), CLI unit (13), and focused native artifact/API tests
pass. Source annotation recognition and source-file collision checks are shared;
legacy SET origins are now explicitly rejected on the native API boundary.
The JSON dependency required an unambiguous empty-slice assertion in one
existing dominance test; compiler analysis behavior is unchanged.

Slice 3 validation: all 72 native tests pass. The public-artifact target also
passes without `ACTIONC_TEST_COMPILER`, exercising the isolated Cargo build
helper. CI receives the root workspace binary explicitly.

Slice 4a validation: 314 native and 628 guarded 6502 Statemate executions pass,
with both newline conventions through the actual source/include loaders. All
16 CASE routines are shared unchanged; the vector generator check passes.

Deliver the four requested areas: public CLI integration, native output,
compile-and-run execution tests, and native Statemate/Dijkstra/Huffman-decoder
acceptance. Keep the original MC68000, current ABI and r68k harness.

## Outcome and scope

The intended public workflow is:

```sh
actionc --target motorola-68000 --origin 0x10000 \
  -o program.native.json --listing program.68k.txt program.act
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml -- \
  --image program.native.json --budget 1000000
```

These commands are now supported; the inspected baseline below predates them. Native
output is a linked bare CPU image with metadata and payload files. It is not
an Amiga executable, an Atari load file, or a relocatable object file.

Complete the artifact contract first, then connect the CLI to it. Execution
order is **slice 2 → slice 1 → slice 3 → slice 4**; numbering preserves the
requested areas. Commit each completed slice after its scoped validation.
Slice 4 has three independent benchmark commits, in the order below.

Amiga startup/I/O, HUNK/ELF output, external ABI adapters, REAL arithmetic,
multidimensional arrays, a classic 68K backend and further optimizer work are
outside this milestone. Existing GCC measurements remain regression evidence.

## Inspected baseline

- `src/compiler/native.rs` already compiles through the source/module loader,
  modern semantics, verified NIR, MIR68K and the native linker. Its options
  include a 32-bit origin, NIR optimization, native promotion, target codegen
  controls, project root and module paths. Native defaults enable promotion
  and allocation together.
- `src/cli.rs` recognizes the 68000 target but its origin parser is 16-bit.
  Compile requests and Atari output handling also assume 16-bit origins, Atari
  runtimes and `.xex`. Native emission is not routed through this path.
- Source annotation resolution is duplicated between CLI inspection and
  `src/compiler/mod.rs`. Explicit flags override annotation defaults. Native
  dispatch must preserve that precedence instead of adding a third parser.
- `src/mir68k/image.rs` owns linked segments, permissions, zero-fill regions,
  entry and absolute/frame-relative symbols. Array metadata includes stride,
  count and backing/descriptor information. The compiler has no r68k dependency.
- `tools/vm68k-runtime-tests/src/artifacts.rs` writes inspection JSON and raw
  segments, but has no reader. Its version-1 type `kind` contains Rust `Debug`
  text. The separate C-reference `.image` loader is a different transport.
- The runner currently compiles source in-process; execution does not prove
  that a public compiler artifact can be loaded independently of source.
- Statemate has 157 vectors and a 201-byte legacy state layout in `state.tsv`.
  Dijkstra has 33 vectors with 16-bit queue addresses and 8-byte wire records.
  Huffman decoding has 185 vectors with 6-byte tree and 35-byte code wire
  records. Their Action! drivers and 6502 tests currently bind fixed addresses.
  None has a native VM test target yet.

## Slice 2: Compiler-owned native artifacts

Move native image serialization out of the VM package into a small
compiler-owned module, for example `src/compiler/native/artifacts.rs`. Keep
emulation, C-reference transport and VM memory policy out of the compiler.

### Format decisions

Use a versioned JSON manifest plus exact binary segment files, following the
existing dump's representation. `-o` names the manifest; its directory is the
base for payload references. The default native name is
`<source-stem>.native.json` in the current directory. Introduce an explicit
format identifier and version 2, distinguishing this executable transport from
the old version-1 inspection dump. Reject unsupported versions clearly.

Retain these properties:

- Canonical target identifier, endianness, pointer width, link-address width
  and resolved entry address. Segment addresses are full-width values; the
  current target still requires all linked extents to fit its 24-bit bus.
- Initialized regions with exact bytes and read/write/execute properties;
  separate zero-fill extents. Avoid flattening sparse memory or materializing
  BSS into a huge binary file. Relocations are already resolved by the linker.
- Stable tagged symbol identities and qualified display names; explicit
  absolute versus frame-relative locations, size and alignment. Preserve
  aliases as symbols without mapping duplicate overlapping regions.
- Stable symbol layout tags for integer, boolean, data pointer, callable,
  record and opaque data, with the widths/signedness needed for inspection.
  Preserve array count, element width, stride, descriptor status and initial
  backing address. Do not parse `Debug` output or display names as semantics.
- Optional human-readable machine listing, generated from physical MIR68K.
  It is inspection text, not promised assemblable/re-originable MADS source,
  and must not be required to load an image.

Define a bounded wire model instead of serializing the full NIR or MIR type
system. Import produces a validated execution image and symbol view; it need
not reconstruct callable signatures or executable NIR. Factor shared image
validation and VM mapping so the serialized and in-memory paths cannot drift.
Use typed JSON parsing, with `serde`/`serde_json` scoped to the wire structures
if needed; do not add hand-written parsing of arbitrary JSON. Update affected
workspace lockfiles when adding compiler dependencies.

### Validation and file handling

Validate before execution: format/target/layout compatibility, checked address
arithmetic, actual payload length, region overlap, even executable ranges and
entry inside executable memory. Validate symbol identity/location forms,
scalar widths, alignment and bounded array metadata. Frame symbols remain
frame-relative and cannot be read as globals after return. Absolute symbols
may describe external/hardware addresses; they do not themselves grant memory
permissions or require the image to allocate that memory.

Resolve payloads relative to the manifest, independently of process working
directory. Emit portable relative paths and reject absolute/escaping payload
references. Compilation and serialization must finish before publishing an
artifact. Stage payloads and publish the manifest last; avoid overwriting
payloads still referenced by an existing valid manifest. Check collisions with
source, listing and output paths, and clean up only owned staging files.

Retain the VM `artifacts::dump` entry point as a thin compatibility wrapper.
Update the measurement/C-comparison consumers to use the shared writer without
changing segment bytes, measured options or their independent C transport.

Primary consumers: `src/mir68k/image.rs`, the new compiler artifact module,
`tools/vm68k-runtime-tests/src/artifacts.rs`, VM symbol access, and measurement
helpers that read the dump.

Acceptance:

- Export/import preserves guest bytes, entry, permissions, zero-fill and all
  supported symbol properties; sparse images and aliases work.
- Reject malformed versions, incompatible targets, overflow, bad entry,
  missing/truncated payloads and invalid metadata before starting the CPU.
- Exercise LF/CRLF JSON, escaped names, paths with spaces, alternate working
  directories and explicit output failures. Binary bytes stay unchanged.
- Existing native tests and the paired C command retain their results and
  measurement columns. Record the public schema in
  `docs/NATIVE_IMAGE_FORMAT.md` and update the execution contract.

Suggested commit: `feat(native): add validated native image artifact I/O`.

## Slice 1: Public CLI integration

Add a small target-aware dispatch boundary to `actionc` and `actionc-emit`.
Route Motorola68000 to `compiler::native::compile_file`; retain the existing
Atari compiler API/output path. Do not force native programs into the 6502
`CompiledProgram` representation or widen every legacy code-generation type.

Resolve flags and source annotations before target-specific validation or
output naming. Reuse one annotation parser for CLI/compiler configuration;
keep explicit CLI settings authoritative and module-loading behavior intact.
A root `;@actionc target motorola-68000` annotation must select the same path
as the flag. Diagnostics must still identify the originating source/module.

### CLI contract

| Setting | Native behavior |
| --- | --- |
| `--target motorola-68000` and existing aliases | Select modern semantics and MIR68K automatically |
| `--backend mir68k`, `--profile modern` | Optional explicit equivalents; incompatible resolved settings are diagnosed |
| `--runtime bare` | Optional explicit name for the existing bare CPU runtime |
| No optimization flags | Preserve `NativeCompileOptions` defaults |
| `--no-opt` | Raw verified NIR; target codegen options remain independent |
| `--no-codegen-opt` | Conservative materialization; does not silently disable NIR promotion |
| `--origin` | Parse decimal, `0x`/`0X` and quoted `$` hex without 16-bit truncation; native default remains `0x10000` |
| `--module-path` | Forward repeated paths through the current module loader |
| `-o`, `--listing` | Native manifest and optional physical machine listing |

Native selection must not treat inherited Atari CLI defaults as explicit
requests for compatibility mode, classic codegen or the cartridge runtime.
Explicit `--mode compatibility/optimized/mir6502`, `--backend classic/mir6502`,
legacy profile and cart/standalone runtime selections are incompatible with
native emission; give actionable configuration errors. Native-only controls
must also be rejected on targets where they have no meaning.

Parse the origin into a sufficiently wide CLI value, then narrow/check it at
the chosen target boundary. Keep the public Atari `with_origin(u16)` contract.
Reject negative, overflowing, odd native or out-of-bus origins and linked
extent overflow. Preserve the native API's diagnostics for source `SET` origin,
unsupported startup, missing entry, external ABI calls and unsupported runtime
operations. Configuration failures remain exit 2; compilation failures remain
exit 1. Neither may create or overwrite a completed artifact.

For `actionc-emit`, preserve token/SemIR/NIR inspection and add the native
emission cases explicitly: `--emit-code` prints address-labelled segment hex,
`--emit-listing` prints physical MIR68K instructions, and `--emit-map` prints
native symbol locations/extents. Atari load bytes, MADS source listings,
6502 MIR/proof modes and other unimplemented target-specific combinations get
clear diagnostics. Update the existing test that expects missing 68K codegen;
retain meaningful rejection coverage for unimplemented 65816 emission.

Primary files: `src/cli.rs`, `src/compiler/mod.rs`, `src/compiler/native.rs`,
CLI help, root README, `tests/actionc_cli.rs`, `tests/cli_emit_modes.rs`,
`tests/nir_cli.rs`, and a focused native CLI integration target.

Acceptance: the proposed compile command produces a valid artifact equal in
execution content to the native API for the same resolved options. Cover flag
order, aliases, annotation precedence, module paths, origins above 64 KiB,
explicit conflicts and preservation of existing files on failure. Existing
Atari CLI output/defaults and NIR inspection remain compatible.

Suggested commit: `feat(cli): compile Motorola 68000 native images`.

## Slice 3: Execute the emitted artifact

Add explicit `--image MANIFEST` mode to the existing r68k runner, preserving
positional source mode. Image mode loads the compiler artifact and never
compiles source. Accept the instruction budget; reject compile-only options
such as origin/optimization overrides and source-mode dumps. A linked image
cannot be relocated by changing a runner flag.

Reuse the current trampoline, protected memory, register/stack completion
checks, fault latch and instruction accounting. Image-format validation belongs
to compiler artifact I/O; VM-reserved address conflicts belong to the harness.
Symbol-driven scalar and array access must work with the imported symbol view.
Array access follows a mutable descriptor's current guest value, not just the
manifest's initial backing address.

Add an integration helper that invokes the actual `actionc` executable. In CI,
pass the exact root build's binary via `ACTIONC_TEST_COMPILER`. For standalone
VM tests, use one Cargo-aware discovery/build helper with correct target-dir
and Windows `.exe` handling. Never pick an unrelated compiler from PATH or
launch nested Cargo against an already locked build directory.

Compile each source/configuration once, then load the artifact into fresh VMs
for input cases. Include a runner subprocess smoke test; keep detailed state
assertions in the VM test API. Delete/move the source and relocate the artifact
bundle before loading in a different working directory to prove independence
from compilation state.

Acceptance cases:

- Globals and signed values, initialized data plus BSS, pointers/arrays,
  qualified symbols and frame-relative metadata; mixed widths and big-endian
  values must survive serialization.
- Normal calls/returns and typed runtime faults retain their distinct VM
  outcomes; a fault cannot resume. Permission and ABI guards remain active.
- Two full-width origins, default and raw NIR, plus a focused conservative
  codegen case. Use paired LF/CRLF source/manifest cases and space-containing
  paths rather than multiplying every axis across the entire benchmark corpus.
- Invalid artifacts fail before any instruction executes; compiler diagnostics
  and VM failures have clear nonzero process results.

Update `tools/vm68k-runtime-tests/README.md` with both workflows and wire the
binary location into the existing CI job. Keep r68k solely in its VM workspace.

Suggested commit: `test(mir68k): execute public compiler artifacts in r68k`.

## Slice 4: Statemate, Dijkstra and Huffman decoder

Make three small benchmark ports, each with a commit. Follow the existing
shared-kernel/target-driver pattern used by matrix1 and SHA. Extract algorithm
and type declarations into shared includes; keep fixed addresses, completion
signals and host packet handling in the 6502 adapter. Native drivers allocate
ordinary globals/arrays and return through the native entry contract.

Preserve the pinned C sources, all vector bytes, graph inputs, algorithms,
integer widths and coverage expectations. The old little-endian memory packets
remain a reference wire format, not a native memory layout. Decode them into
logical scalar/record values. Native addresses and array strides come from
emitted symbols; native pointer transport variables use `ADDRESS` or typed
pointers instead of 16-bit `CARD`.

The image currently has record sizes but no field-reflection table. Avoid a
full reflection/NIR expansion for these tests: add a native-driver layout-query
command that exports `SIZEOF`/`OFFSETOF` results through named scalar symbols.
Run that probe once per compiled artifact and query only the record fields
needed by the adapters. Obtain array backing/count/stride from image metadata,
and check field ranges against those extents. Test-only layout queries and
command dispatch stay outside the shared algorithm. Do not infer native record
padding or pointer offsets in host code.

### 4a. Statemate

Split the existing `statemate.act` so both drivers use the same routines and
all 16 CASE statements. Use `state.tsv` to decode the 201-byte reference packet:
byte flags/bit arrays remain exact bytes; INT and LONGCARD fields become numeric
values before native stores. Compare every field for all **157 vectors**,
including timestamp wrap, noncanonical flags, commands and microstep state.
Retain the 6502 adapter's complete guarded-memory checks.

Suggested commit: `test(mir68k): execute portable Statemate reference cases`.

### 4b. Dijkstra

Share the existing algorithm and Node/Row/QueueItem declarations. Translate
legacy queue links from `0x4001 + 8 * slot` into `null` or a logical slot index;
map those indexes to the native pool's current base and compiler-derived stride.
Perform the inverse conversion for comparisons. Reject links outside the pool
or into the middle of a record; do not truncate native addresses.

Use record layout queries for scalar fields, the next link and nested row-array
storage. Check all **33 vectors**: every node distance/predecessor, queue record,
link topology, header/control result, counters and unchanged graph. Preserve
pool exhaustion and the upstream increment-before-check behavior, including
slot 999 and start-equals-end cases. This requires no multidimensional arrays.

Suggested commit: `test(mir68k): execute portable Dijkstra reference cases`.

### 4c. Huffman decoder

Keep iterative tree construction/decoding and the existing valid-stream
contract. Decode little-endian header, code-table and tree fields into logical
values. Convert legacy links from `0x6501 + 6 * slot` to native tree addresses
and back to indexes. Preserve byte strings and bit arrays exactly; the 35-byte
wire code record does not determine native record stride/padding.

Check all **185 vectors**, including 256-bit codes: complete output and poison
tail, reservoir/bit counters, code lengths/presence/bits, tree symbols and links,
root identity, node count and untouched logical records. Keep padding checks
separate from defined field values. This port does not add recursion or invent
error semantics for malformed streams outside the pinned oracle's contract.

Suggested commit: `test(mir68k): execute portable Huffman decoder reference cases`.

### Common benchmark acceptance

Execute each full corpus from CLI-produced artifacts in raw and optimized NIR:
**375 vectors × 2 = 750 native reference executions**, excluding layout probes
and small workflow tests. Compile once per driver/configuration, not per vector.
Keep 6502 acceptance in both existing backends and runtimes. Exercise LF/CRLF
through the actual changed source/include generation and vector parsing paths.
Native tests compare semantic state, pointer identities and buffer boundaries;
6502 tests retain their existing transport checks.

Run each generator's `--check`; add adapter/layout tests without regenerating
or weakening expected outputs. If a port exposes a compiler gap, isolate a
general fix with a focused regression and the appropriate compiler checks;
do not add sample-name special cases or duplicate a modified algorithm.

## Validation and completion

Scope local checks to each slice and its changed consumers:

- Artifact I/O: focused format and VM emission/symbol tests, the complete native
  suite after shared loader/access changes, and one unchanged paired C run
  because its dump path changes.
- CLI: relevant CLI/configuration/source-annotation tests, native compile tests
  and representative existing Atari invocations. Update help/docs with those
  behaviors; do not rerun the entire 6502 VM corpus for command routing alone.
- Workflow: compiler-subprocess and image-runner tests; complete native suite
  after integration. Keep compiler building outside per-vector loops.
- Each benchmark: its two VM test targets, its generator check and relevant
  source/include construction tests. Full compiler/NIR checks are unnecessary
  for an adapter-only change.

If implementation changes shared NIR, semantic lowering, verifier, printer or
their contracts, run the required repository checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Benchmark generator checks are:

```sh
python3 tools/generate_statemate_vectors.py --check
python3 tools/generate_dijkstra_vectors.py --check
python3 tools/generate_huff_dec_vectors.py --check
```

Preserve binary and ATASCII bytes. Normalize host text before newline-sensitive
instrumentation and exercise the real LF/CRLF path, including embedded includes.
Keep unrelated worktree changes out of commits and use an isolated checkout if
an unrelated untracked sample interferes with required compiler checks.

At integration, retain current native/GCC benchmark inputs and record any
measurement change with its cause. Run the existing Linux/Windows/macOS CI
workflow without removing coverage. Start CI without waiting for it to finish;
report the tested compiler commit and pending/result status accurately.

Completion means the public workflow runs independently of source, artifact
metadata/bytes survive validation and loading, all 750 native reference cases
match, affected 6502 cases and generators pass, and documentation describes
supported modes and diagnostics. Commit the final completion record separately
if necessary. Amiga platform output and further language work follow this
milestone.

## Related contracts

- [MIR68K execution boundary](MIR68K_EXECUTION_CONTRACT.md)
- [Native ABI and automatic storage](NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md)
- [NIR target shape](NIR_TARGET_SHAPE.md)
- [Current code-quality measurements](MIR68K_C_COMPARISON.md)
- [Native VM runner](../tools/vm68k-runtime-tests/README.md)
