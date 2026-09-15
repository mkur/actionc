# MIR68K checkpoint — 2026-09-15

Implementation baseline: `5d708bb` on `main`. Development is switching to an
Atari SIO library after completion of the descriptor-alignment milestone.
No known 68K blocker was identified for the currently supported surface during
this checkpoint review. This is a resumption guide, not a claim of complete
Action! or Amiga runtime compatibility.

## Working execution path

Modern Action! source passes through SemIR, verified NIR, shared aggregate ABI
expansion and MIR68K to original-MC68000 instructions. The classic 6502 backend
is not separately ported. MIR68K consumes typed NIR facts without consulting
SemIR. The native ABI uses invocation-local frames, four-byte pointers and
big-endian storage; linked bare images must fit the 24-bit address space.

The public compiler emits either a version-2 bare image bundle with symbols or
a single relocatable Amiga HUNK executable. The independent r68k workspace
executes emitted bytes and checks memory, stack, preserved registers and faults.
MIR65816 still has layout/lowering coverage, not an equivalent execution path.

| Surface | Current support and evidence |
| --- | --- |
| Integers | BYTE/CHAR, CARD/INT and LONGCARD/LONGINT arithmetic, multiply/divide/remainder, comparisons, logical shifts and casts. The portable signed-shift library also executes. |
| Control flow | IF/ELSEIF, CASE, loops, EXIT and RETURN; exercised by focused tests and benchmark ports. |
| Routines | Direct and typed indirect calls, scalar/pointer results, recursion and automatic local storage. Shared ABI expansion also handles aggregate value arguments/results. |
| Memory and arrays | Pointers, absolute data, mutable descriptors, fixed multidimensional arrays, local initialization, records and embedded arrays. Odd accesses, rebinding, overlapping copies and volatile byte traces have focused coverage. |
| Records, unions, variants | Shared layout/value/call lowering is implemented. Records have substantial native execution coverage. Union and variant execution coverage is thinner; see the audit below. |
| Amiga runtime | Shell startup, cleanup, typed faults, raw byte/counted-string output and decimal BYTE/CARD/INT/LONGCARD/LONGINT output. DOS library version 40 is required. |

The [execution contract](MIR68K_EXECUTION_CONTRACT.md) owns ABI and emission
guarantees. [Amiga usage](AMIGA.md) owns supported platform calls and launch
requirements. [Native image format](NATIVE_IMAGE_FORMAT.md) owns artifact layout.

## Completed optimization milestone

All four descriptor-alignment slices are committed:

| Commit | Change |
| --- | --- |
| `7b37952` | Verified NIR tracks alignment established by runtime descriptor stores. |
| `da7b9c0` | Optional runtime alignment guards for ordinary indirect longword accesses. |
| `bc9fafd` | Guards enabled by default after corpus and differential validation. |
| `5d708bb` | Paired MC68000 C comparison extended to shaped matrices and DCT. |

Static proofs and guards preserve mutable-descriptor semantics. Unknown word
accesses remain bytewise; volatile accesses retain their established sequence.
An aligned descriptor cell does not prove that its current pointer is aligned.

Against the pre-milestone baseline `38471f2`, optimized shaped matrix1 fell from
144,051 to 106,651 instructions (26.0%), and shaped DCT from 43,279 to 38,991
(9.9%). Executable sizes grew from 1,332 to 1,494 and 6,658 to 7,558 bytes;
frame sizes and stack traffic were unchanged. These are instruction counts,
not timing measurements. GCC still produces substantially smaller and cheaper
code on these workloads; this milestone does not establish code-quality parity.

See the [completed plan](MIR68K_DESCRIPTOR_ALIGNMENT_IMPLEMENTATION_PLAN.md),
[validation and measurements](MIR68K_DESCRIPTOR_ALIGNMENT.md), and
[paired GCC report](MIR68K_C_COMPARISON.md) for corpus definitions and reproduction.

## Validation at the checkpoint

- The completed milestone passed the native VM suite, MIR68K compiler checks,
  NIR snapshots and sweep. Compiler tests excluded an unrelated untracked sample
  from the working-tree sample scan; that exact parser test separately passed
  against tracked sources using the current compiler library. This qualification
  is recorded in the alignment report.
- Both VM workspaces passed the matrix1 and DCT corpora, including flat/shaped
  forms and their applicable modes/runtimes. The expanded C comparison passed
  1,845 reference executions and 15 default runs, with complete-state checks
  and LF/CRLF instrumentation coverage.
- The [manual vAmiga acceptance record](MIR68K_AMIGA_EMULATOR_VALIDATION.md)
  covers four smoke programs on AmigaOS 3.1, including redirected output,
  return codes and recovery after a deliberate fault. That run used compiler
  `9599469`; it predates the latest optimization work and long-integer console
  extension. It is not a fresh emulator qualification of `5d708bb`, older OS
  versions or AROS. Local validation does not assert the current remote CI state.

### Aggregate execution audit

On 2026-09-15, eight temporary probes compiled and executed successfully in
r68k at this baseline: raw and optimized NIR for a record, an overlapping union
view, and two variant inputs (empty and payload-bearing). Each exercised direct
and typed indirect by-value calls and aggregate returns. Scalar result checks
covered record source preservation, big-endian union byte updates, construction
and CASE payload extraction.

These probes were not committed as regression tests. Existing
[aggregate call tests](../tests/aggregate_indirect.rs) and
[union value tests](../tests/unions_values.rs) primarily establish native
lowering/ABI contracts. The native
[fault tests](../tools/vm68k-runtime-tests/tests/faults.rs) already execute an
invalid variant tag and check terminal behavior. Successful probes are useful
evidence, but do not replace a permanent aggregate execution corpus.

## Remaining boundaries

- REAL operations and foreign machine blocks/inline assembly have no MIR68K
  lowering. Shared REAL representation is still tied to Atari six-byte packed
  decimal storage.
- Arbitrary external ABIs require adapters. The supported Amiga console bindings
  do not imply a general external-call or full Action! runtime implementation.
- Graphics, input, general file APIs, command-line argument handling and
  Workbench application launch are outside the current Amiga runtime.
- Native source ORG, Atari SET code-origin controls, fixed routine placement and
  executable top-level statements are rejected. Entry must be a parameterless
  PROC. Current frame and incoming-argument layouts must fit signed-16-bit
  displacement limits.

Some earlier plans and the initial-subset paragraph in the VM README describe
pre-emitter or pre-fault-adapter limitations. Read them as historical context;
the execution contract, current diagnostics/tests and dated acceptance records
take precedence. In particular, integer multiplication/division, Amiga HUNK
output and the 68K variant fault adapter are already implemented.

## Resume priorities

1. Add permanent 68K aggregate execution tests: records/unions/variants through
   direct and indirect calls, source independence, nested payloads, recursion,
   invalid tags and raw/optimized code. Reuse compiler symbols and the existing
   VM harness; extend HUNK coverage where relocation or runtime behavior differs.
2. Choose the next application milestone. The discussed simple-graphics route
   is an Amiga runtime module using Intuition and graphics.library for screen/
   window lifecycle, Plot/MoveTo/DrawTo, pens and palettes. This is a proposal,
   not an implemented or approved API. Integer drawing coordinates do not
   require REAL support first.
3. Design REAL separately if needed. IEEE storage for 68K, software/OS helpers
   for baseline MC68000 and later optional FPU emission were discussed. Precision,
   rounding, conversions, fault behavior and ABI remain undecided. Generalize
   the shared format contract before adding target emission.
4. Continue measured code-quality work when useful: address-register allocation,
   indexed/postincrement addressing, memory-destination arithmetic and wide
   multiplication are candidates. Preserve odd-address, alias and volatile
   behavior and compare equivalent complete workloads against C.

For resumption commands, use the [r68k runner](../tools/vm68k-runtime-tests/README.md),
[C reference tooling](../tools/mir68k-c-reference/README.md) and
[Amiga smoke procedure](AMIGA.md). Scope checks to the changed consumers as
required by [AGENTS.md](../AGENTS.md); compiler/NIR changes require the broader
checks. No compiler tests need repeating solely for this documentation checkpoint.
