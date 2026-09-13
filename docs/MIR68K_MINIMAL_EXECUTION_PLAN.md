# Minimal MIR68K Execution Plan

Status: implementation in progress. Slices 0–3 are implemented. The first
source-to-execution gate passes: an ordinary LONGINT global receives 42 in raw
and optimized modes, at two origins, from LF and CRLF files. Scalar arithmetic,
control flow, calls and benchmark acceptance remain in slices 4–6. Cross-platform
qualification will run in CI when slice 6 lands.

## Objective and completion gate

Compile ordinary Action! source into original MC68000 machine code and execute
it in **r68k**, with observable results, bounded execution, and useful failure
diagnostics. Keep the existing SemIR -> verified NIR -> MIR68K boundary.

The first milestone is a generated program that stores `42` in an ordinary
global, returns through the native ABI, and leaves the stack balanced. The
complete milestone adds integer computation, branches, loops, calls, pointer
access and one-dimensional arrays, then runs the existing TACLeBench insertion
sort vectors with optimization both enabled and disabled.

Tests locate data through compiler-emitted symbols. Benchmark source does not
need a 6502 memory map. The runner executes emitted instruction bytes; it never
interprets NIR or implements Action! arithmetic on the host for the guest.

## Inspected baseline

| Area | Available now | Work needed for execution |
| --- | --- | --- |
| [Verified backend handoff](../src/backend.rs) | Target/layout checks and verified NIR input | Retain this boundary through materialization and emission |
| [MIR68K representation](../src/mir68k/mod.rs) | Scalar operations, addresses, calls, frames and relocations | Complete executable facts, physical instructions and verification |
| [MIR68K lowering](../src/mir68k/lower.rs) | Big-endian constants and independent native frame/call planning | Storage allocation, temp homes, encoding and linking |
| Entry selection | NIR already has `NirRoutine.entry.program` | Preserve its `RoutineId` in the native image; do not search for `Main` |
| Native ABI | A6 frames, even stack argument slots, D0 scalar and A0 pointer results | Emit these plans, including spill storage and captured arguments |
| [Compiler API](../src/compiler/mod.rs) | Module loading, semantic analysis and linking | Native image API; existing output and origins are Atari/u16-specific |
| VM tests | 6502 harness and C-reference benchmark vectors | Separate r68k adapter and target-aware result serialization |

Several current canary omissions must be addressed before calling its output
executable:

- `Compare` retains a width but loses operand signedness. `Binary.signed`
  recognizes I16 but not I32.
- Routine result types/homes and parameter/temp types are not retained as a
  complete materialization contract.
- Block parameters and edge arguments are dropped. Executable lowering must
  preserve their parallel-transfer semantics.
- `Mir68kData` has a display name but no storage ID. Uninitialized globals,
  descriptor cells, zero-fill tails and routine-static objects need explicit
  allocation records, not just projected initializer bytes.
- Byte-selection relocations lose their byte selector in `lower_data_image`.
- `CopyBytes` lowering loses source/destination volatility.
- `spill_bytes` is zero; the current frame check does not prove that future
  temp/spill storage, outgoing arguments and saved registers are disjoint.

These are planned fixes, not failures observed in generated 68K code: there is
no executable 68K backend yet. Existing canary success remains a lowering claim.

## Scope and implementation choices

### Initial source coverage

Support BYTE/CHAR, INT/CARD, LONGINT/LONGCARD and target-sized ADDRESS/SIZE,
with their existing Action! widths and conversions. Include constants, loads,
stores, address-of, addition/subtraction, negation, bitwise operations, logical
shifts, signed/unsigned comparisons, IF/CASE, loops, direct calls and typed
indirect calls. Include ordinary globals, automatic locals, fixed arrays,
array parameters, scalar record fields and fixed-size memory copies.

Preserve NIR's evaluation order, intermediate widths, explicit initialization,
volatile accesses and captured addresses. Native automatic storage must remain
invocation-local, including across recursive calls.

The initial emitter diagnoses unsupported multiply/divide/remainder, REAL,
foreign machine code, external OS/library calls and native runtime services.
It also rejects executable top-level statements until their startup ordering
has an explicit executable contract. Library declarations and static data
initializers remain supported. Every reachable terminator must be resolved;
unresolved fallthrough must never become accidental execution of adjacent code.
A terminal fault/exit path must not be converted into a successful return to
the completion trampoline.

Full aggregate-by-value acceptance, Amiga Hunk files and startup, OS services,
graphics, interrupts, a native assembler language, multidimensional arrays,
register-allocation optimization and performance tuning are later work.

### CPU and image

Use the existing `Motorola68000` / `Generic68k` / `Motorola68kNative` target
layout: big-endian storage, four-byte pointers, and a 24-bit linkable address
range. Keep pointer representation distinct from physical address decoding.
Only original MC68000 instructions and addressing modes are legal.

Return a structured native image containing initialized segments, explicit
zero-fill regions, the entry address, target/layout identity, and symbols.
Use checked target-sized addresses and extents throughout. Data and code
relocations use stable IDs and checked addends. Aliases refer to existing
storage; automatic objects do not receive static addresses.

The harness owns reset vectors, the entry trampoline, stack and guard regions.
Start with a 1 MiB test memory configuration, an image origin at `0x00010000`,
and a separate stack near the top of that memory. Validate segment/stack/vector
separation, and test a second origin so a hardcoded address cannot pass by
accident. These are runner settings, not language or backend layout rules.

The trampoline calls the emitted entry and reaches a reserved completion trap.
Completion is accepted only at the expected trampoline location with the
expected stack and preserved registers. STOP, timeout, stray traps and invalid
instruction/address exceptions are distinct failures. A reserved trap is a
test transport detail; it must not redefine an existing NIR runtime fault.

### API and tooling

Add a focused native API under `src/compiler/native.rs`, with proposed types
`NativeCompileOptions` and `NativeCompiledProgram`, and a `compile_file` entry
point in that module. Its options include a full-width origin, NIR optimization
selection, project root and module search paths. Initially it accepts only the
Motorola68000 target and a bare CPU test environment.

Reuse frontend loading, diagnostics and reachable-program selection. Extract
only the shared preparation needed; do not route a native image through
`CodegenOutput`, Atari load-file packaging, `Runtime::ActionCart`, or the
6502 standalone runtime. Compiler generation remains independent of r68k.

Add `tools/vm68k-runtime-tests` as an independent Cargo test crate, following
the existing VM-test workspace pattern. It depends on actionc and an exactly
pinned r68k release, initially qualifying **0.2.2**, with a committed lockfile.
Keep the emulator dependency out of the compiler's production dependencies.

A small development runner in that crate accepts a source path, origin,
optimization setting and execution budget. It calls the native compiler API,
runs the image, and reports typed symbols or a diagnostic. It can dump image
segments and a versioned JSON manifest for inspection. A general redesign of
`actionc`/`actionc-emit` output selection is a follow-up, not a prerequisite.

### Symbols and test data

The image's symbol table is generated from compiler storage/layout facts and
final link addresses. Each exported entry includes a stable identity, qualified
display name, storage category, address or frame-relative location, extent,
alignment, and enough type information to decode supported scalars and arrays.
Array metadata distinguishes a descriptor cell from the element backing and
records element width/stride and count. Automatic-local metadata identifies
its routine/frame object; it must not invent one global address for that local.

Start test access with globals and arrays. Decode existing little-endian vector
fields into numeric values, then encode them for the selected target; never
copy a little-endian scalar buffer directly into 68K RAM. Preserve raw byte
arrays as bytes. Keep expected arithmetic results independent of the emitter
and emulator. Do not port every existing 6502 adapter in this task.

## Delivery slices

### 0. Qualify r68k and establish the runner

Implement a CPU/memory adapter with reset/stack setup, single-instruction
execution, bounded history, register inspection, and distinct completion,
exception, memory-violation, stopped/halted and budget-exhaustion results.
Qualify the pinned crate on Linux, Windows and macOS.

Use `ConfiguredCore`, a custom `AddressBus`, and exception callbacks. Step with
the callback-aware API so exceptions cannot disappear into default handling.
Confirm the API's cycle budget behavior rather than treating cycles as retired
instructions. Record the pre-step PC and instruction bytes for diagnostics.

`AddressBus` reads are not fallible results. Latch out-of-range/protected-memory
accesses in the adapter and terminate the run after the step; report them as
harness memory violations, separately from architectural address exceptions.
Guard checks must tolerate legal instruction prefetch into mapped code padding.
The initial harness does not claim full hardware bus-error emulation.

Check independently encoded programs for byte/word/long transfers, arithmetic
flags and branches, JSR/RTS, LINK/UNLK, big-endian memory, completion and illegal
instructions. Deliberately execute odd-address word/long accesses and an
infinite loop. Verify repeated and parallel VM instances do not share state.

**Gate:** the harness can distinguish success from each failure class without
any actionc-generated instructions. Preserve the literal encodings and their
manual references as independent evidence for later encoder tests.

### 1. Complete the MIR68K execution contract

Extend the current MIR in small changes, preserving the existing public
lowering entry points:

- Retain entry identity, complete scalar type/signedness facts, parameter and
  result homes, temp definitions, block parameters and typed edge arguments.
- Retain source and destination volatility on memory copies.
- Give every data allocation a stable ID, extent, alignment, initialization and
  relocation plan, covering globals, statics and array descriptors/backing.
- Preserve relocation encoding, including selected-byte fragments. Unsupported
  relocation forms must produce a diagnostic rather than a truncated address.
- Use typed routine identities for calls and relocations. Builtin names remain
  diagnostic metadata until a supported runtime binding exists.
- Strengthen verification for missing storage/callees, conflicting temp types,
  edge argument arity/types, malformed entries and unresolved executable forms.

Obtain facts from verified NIR. Where a needed layout fact is actually absent
from NIR, add that specific typed fact and verifier rule in a separate vertical
slice; do not recover it by consulting SemIR from the backend. Entry selection
already exists and does not require a new source convention.

**Gate:** focused negative tests reject the known omissions; raw and optimized
fixtures preserve the facts required for emission. Existing MIR6502/MIR65816
consumers retain their contracts. Snapshot changes are identified as MIR
contract corrections or, if needed, explicit NIR metadata changes.

### 2. Add physical instructions, encoding and image linking

Keep semantic MIR and physical machine instructions distinct inside
`src/mir68k`. Suggested modules are `machine`, `materialize`, `verify`,
`encode` and `image`; introduce them only as their slices need them.

Define typed opcodes, operand sizes, registers, legal effective-address forms,
machine block identities, and symbolic relocations. The encoder writes bytes
from these forms; it does not parse assembly strings or make source-language
decisions. Add a readable physical-MIR dump for failures.

Begin with MOVE/MOVEA, immediate constants, address materialization, absolute
JMP/JSR, conditional branches, LINK.W, UNLK and RTS. Add integer operations as
slice 4 requires them. Validate legal size/address combinations and even code
alignment. Compare representative encodings with the independent slice-0
programs and execute encoded forms in r68k.

Use conservative fixed-size branch sequences initially: absolute jumps and
an inverted short condition around an absolute jump where appropriate. This
avoids needing branch relaxation before correct execution. Check every
displacement and relocation before writing output; do not emit 68020-only
long branches, scaled-index forms or LINK.L.

Allocate initialized data, zero-fill storage and code, then resolve IDs to
addresses. Verify zero-fill tails, descriptors, aliases, pointer initializers,
routine-address initializers and overflow/overlap rejection. Keep native data
alignment consistent with the access proofs used by materialization.

**Gate:** the emitter produces a valid image with deterministic layout and
symbols, and encoder/image tests detect bad instructions, missing relocations
and truncated addresses.

### 3. Execute the first compiled Action! program

Connect the native compilation API to frontend preparation, verified NIR,
MIR68K, physical materialization and image emission. Support ordinary scalar
globals, literal assignment with its necessary casts/temp homes, and a
parameterless PROC return first.

Use a source fixture such as:

```action
LONGINT result
PROC Main()
  result=42
RETURN
```

Select the routine through `entry.program`, resolve its address, load the
image, and invoke it through the harness trampoline. Zero only compiler-owned
zero-fill regions; poison other test memory so missing initialization is
visible. Read `result` by symbol, and verify its big-endian bytes, stack balance,
code protection and guards.

Reject a library-only source or invalid entry signature with a focused native
compilation diagnostic. Unsupported instructions/forms fail compilation even
if the emitter has already accumulated some internal bytes; never return a
partial successful image.

**Gate:** the same source compiles and runs with optimization on/off and at two
origins, including an origin above 64 KiB. This is the first end-to-end milestone.

### 4. Materialize integer operations and control flow

Use stack homes for all temps initially and a small documented scratch-register
set. Load operands, perform the operation, and store the result at its resolved
width. Values needed after a call or another operation must have durable homes.

Implement typed addition/subtraction, bitwise operations, negation, casts,
comparisons and logical shifts at 8/16/32 bits. Use explicit sign/zero extension
and narrowing from NIR. A register's unchanged upper bits must not accidentally
become part of a widened result. Comparison values are canonical Action! 0/1,
not the native Scc true byte. Signed and unsigned branch conditions are distinct.

Preserve Action! shift-count behavior, including zero and counts at/beyond the
operand width; a register shift's hardware count masking is not the language
contract. `MATH.INTEGER.AsrI`/`AsrLI` remain ordinary Action! library code and
can be qualified after calls are available.

Materialize CFG edge arguments as parallel copies. Split edges or use temporary
homes where necessary; never overwrite a source before another edge assignment
reads it. Support loop backedges and joins in optimized as well as raw NIR.

**Gate:** host oracles cover integer boundaries, conversions, signedness,
condition values, IF/CASE, loops, side effects and edge-copy cycles. A program
reading a host-supplied global and calculating a result proves the inputs are
not being folded away.

### 5. Emit native calls, frames and compound memory access

Finalize frames after allocating temp and edge-copy homes, including any saved
registers and the maximum outgoing argument area. Verify non-overlap and check
all LINK.W/frame-displacement limits; diagnose oversized frames instead of
truncating offsets.

Keep the planned stack-first convention. With JSR and LINK A6, incoming argument
offsets start at `A6+8`; stack slots are rounded to an even size. Define byte
placement explicitly: a one-byte argument occupies the first byte of its slot,
with padding outside the value. Caller stores and callee loads use the same
rule. Scalars return in the low bits of D0, pointer/callable values in A0.

Initially reserve D0/D1/A0/A1 as scratch, preserve D2-D7/A2-A5, restore A6 through
UNLK, and restore A7 on return. Store call results before reusing scratch.
Capture every argument and an indirect callee before populating the outgoing
area; nested evaluation must not overwrite an outer call's prepared arguments.
Verify mutable and address-taken parameter copies and automatic initializer
execution on each invocation.

Implement global/frame/parameter addresses, pointer dereferences, record-field
offsets and indexed arrays using full-width address calculations. Synthesize
stride calculations for the original 68000 instead of assuming a scaled-index
instruction. Use native word/long accesses only when alignment is proven;
otherwise assemble/disassemble the value with ordered byte accesses. Do not
eliminate, duplicate or widen observable accesses; packed values use the defined
bytewise fallback. Preserve fixed-copy overlap semantics. Copies also provide
the mechanism for compiler-lowered local array initialization.

**Gate:** execute nested and indirect calls, more than four arguments, BYTE/INT/
LONGINT/pointer returns, caller-live values, mutable parameters, address-taken
locals, automatic arrays, packed accesses, and a bounded recursive call. Verify
frame isolation, preserved registers, result bytes, stack balance and guards.
Faulting deliberate odd word instructions must fail while compiler-generated
safe accesses to an odd-address packed value succeed.

### 6. Run insertion sort and establish CI coverage

Move the existing insertion sort fixture's absolute addresses and test driver
into its 6502 adapter, retaining one maintained portable algorithm. Adapt the
68K driver through ordinary globals and symbol metadata. Preserve the original
C reference, vectors, operation order and expected results.

Run all 209 existing insertion sort vectors through raw and optimized NIR on
68K. Compile each configuration once, reset memory/state per vector, and compare
every array element, statistic and status. Check guards and preserved inputs.
Use the same numeric vectors for both targets, converting endianness only at
the memory boundary. Run the affected 6502 insertion sort target after this
fixture refactor; avoid unrelated benchmark rewrites.

Add the 68K test crate to the existing Linux/Windows/macOS CI matrix with
`cargo test --locked`. Exercise LF and CRLF through actual source loading and
vector parsing. Keep a small native rejection suite so unsupported source
features cannot silently produce apparently runnable images.

**Gate:** generated insertion sort passes every reference case in both NIR
optimization settings; focused ABI/arithmetic/memory tests and cross-platform
CI pass. Document the supported execution subset and remaining diagnostics.

## Verification scope

| Changed area | Required checks |
| --- | --- |
| Plan/documentation only | Content, relative links and whitespace; no compiler suite |
| r68k adapter only | Adapter qualification tests and independent byte programs |
| MIR68K or encoder only | Focused MIR68K/encoder/image tests, existing native ABI/type integration tests, affected 68K execution tests |
| SemIR/NIR contracts or shared frontend preparation | Relevant fixtures, NIR sweep and root compiler suite; representative Atari compatibility checks |
| Insertion sort portability | Both affected VM targets; generator check if vectors/generator change |
| Shared runtime/code generation behavior | Broaden execution testing to affected consumers, following AGENTS.md |

For any SemIR/NIR contract change, run the repository-required checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

The planned native suite command, once the crate exists, is:

```sh
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml
```

Use focused test filters during development; the complete native suite remains
in CI. Do not rerun all 6502 VM benchmarks after an isolated encoder/harness
change. Report unrelated worktree failures separately from regressions.

## Follow-up after this milestone

Add typed multiplication/division/remainder and the native non-returning fault
adapter next. Original 68000 multiply/divide instruction widths need explicit
legalization for full-width LONGINT/LONGCARD arithmetic. Preserve the language's
division-by-zero and overflow contract. This opens the next benchmark group:
binary search where division is still present, matrix1, SHA, DCT and ADPCM,
with focused coverage identifying each remaining dependency.

Then expand public CLI/output-format integration and Amiga platform adapters.
Optimization starts from measured emitted code and VM execution after correctness
is established. Multidimensional arrays remain independent frontend work.

## References

- [Native ABI and automatic storage plan](NATIVE_ROUTINE_ABI_AND_AUTOMATIC_STORAGE_IMPLEMENTATION_PLAN.md)
  defines the existing call/frame contract and records the r68k decision.
- [NIR target contract](NIR_TARGET_SHAPE.md) defines layout, storage and verified
  backend ownership.
- [r68k 0.2.2 API](https://docs.rs/r68k/0.2.2/r68k/),
  [ConfiguredCore](https://docs.rs/r68k/0.2.2/r68k/cpu/struct.ConfiguredCore.html),
  and [AddressBus](https://docs.rs/r68k/0.2.2/r68k/ram/trait.AddressBus.html)
  are the integration references; qualification remains required.
- [Motorola programmer's reference](https://www.nxp.com/docs/en/reference-manual/M68000PRM.pdf)
  is the instruction-encoding reference. Its family-wide entries must be checked
  for original MC68000 applicability.
- [Insertion sort port](../fixtures/runtime/tacle/insertsort/README.md) supplies
  the first complete benchmark oracle.
