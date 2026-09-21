# Experimental native 65816 o65 writer and reference relocator

Status: implementation in progress.

| Slice | Status | Checks |
| --- | --- | --- |
| 1: relocation facts/profile | Complete | 39 existing root checks, 3 o65 checks; native execution/memory/interop/indirect/stack faults |
| 2: writer | Complete | 6 o65 tests; independent Python decoding, vasm fixture, wide fields/carry/gap/index checks |
| 3: reference relocator | Complete | 11 o65 tests; hand-authored load/carry fixture, every truncated prefix, corruption and placement/binding rejection |
| 4: CLI integration | Complete | 7 existing CLI tests, 5 o65 CLI tests; LF/CRLF, explicit imports, module/input protection |
| 5: execution qualification | Pending | |
Baseline: actionc `fc9892a`, native ABI v1, existing JSON image v3.

## Objective and completion criteria

Produce a self-contained experimental o65 application from verified native
MIR, then load its serialized bytes at more than one placement without
recompilation. Execute raw and optimized artifacts on the independent native
VM with the existing ABI, stack guards and interrupt contracts.

This implements the recommendation in the
[o65 assessment](MIR65816_O65_ASSESSMENT.md). The experiment succeeds when:

1. The writer preserves all admitted symbolic references and emits standard
   o65 native-65816 relocations with 32-bit structural fields.
2. A host reference relocator needs only the file, a placement request and
   explicit platform bindings. Compiler IR, an original JSON image and a
   compiler-generated sidecar are not required to load it.
3. The same file bytes execute correctly at two upper-RAM placements, including
   distinct data offsets and relocated import addresses. Raw and optimized
   modes each have their own artifact, reused unchanged across placements.
4. Invalid files, placements and bindings fail before publishing a loaded
   image or writing guest memory. Existing JSON output stays compatible.

An Exec816 runtime loader, kernel/bootstrap repackaging, general static linking,
foreign calling conventions, new pointer types and new register allocation
remain subsequent work. No Exec compiler pin changes in these slices.

## Ownership and proposed code boundaries

| Area | Responsibility |
| --- | --- |
| `src/mir65816/image.rs` and a new `src/mir65816/relocation.rs` | Retain typed placement/fixup facts and share existing validation with the fixed image path |
| New `src/mir65816/o65/` | Profile, binary encoder, bounded decoder and host reference relocation API |
| `src/compiler/native65816.rs` | Add an experimental compilation result/API alongside `Prepared::compile`; preserve safe artifact publication |
| `src/bin/actionc-65816.rs` | Explicit experimental output selection; retain current JSON defaults |
| New `tools/inspect_o65.py` | Independently decode wire fields/relocations for conformance evidence; no compiler dependency |
| New `tests/mir65816_o65.rs`, `tests/actionc_65816_o65_cli.rs` | Encoding, relocation, rejection and CLI contracts |
| New native `tests/o65.rs` and a small support adapter | Execute the serialized and relocated artifact on the qualified VM |

Emission owns the new representation. SemIR and NIR retain their existing
meaning and verifier contracts. Use stable identities through placement;
never discover fixups by disassembling bytes or searching display names.
The relocator may share format constants and public ABI constants with the
writer, but must not call compiler placement or patching helpers.

## Experimental profile decisions

Use the identity `actionc.o65.experimental.v1`. Its version is independent of
both the o65 header version and `action65816.native.v1`. Freeze its binary
descriptor and option schemas in a new `MIR65816_O65_PROFILE.md` in slice 1.

### Container and placement

- One native-65816 executable module: wide structural fields, bytewise
  relocation, BSS clearing required. No chained modules, simplified paged
  relocation, weak/common symbols or object archives in this profile.
- Emit all `.size` fields as little-endian 32-bit values: segment bases and
  sizes, stack hint, import/export counts, import indices and symbol values.
  CPU addresses remain checked 24-bit values. Empty segments have zero base
  and length. Use o65's four-byte alignment setting and reject stronger
  storage alignment until it is represented by the profile.
- Use section-relative virtual layouts with original bases zero; these are
  independent section address spaces, not allocations overlapping bank zero.
  Put code first in text, using the current routine bank-containment rule,
  followed by readonly allocations and the profile descriptor. Fill gaps
  deterministically with zero. No routine executes through a gap or into data.
- Allocate initialized writable objects in data. Materialize each object's
  zero tail there so the object remains contiguous. Put entirely zero-filled
  writable objects without initialization relocations in BSS. Readonly zero
  storage stays in text. Resolve aliases to their owner's section and offset;
  aliases never allocate or initialize a second copy.
- Actual text bases must be multiples of 64 KiB. Text occupies one contiguous
  extent, possibly spanning banks, preserving all code offsets within banks.
  Data and BSS move independently at four-byte-aligned bases and can cross
  banks. Check all half-open extents against the 24-bit address space and
  check actual routine extents against the existing unused-last-byte rule.
- Require nonempty application allocations to be in upper RAM for this
  experiment. The o65 zero segment remains empty. Platform memory maps own
  bank-zero DP/stack/vector storage and all permitted/reserved load regions.
  Absolute data aliases remain external addresses and are never initialized.

This layout intentionally differs from arbitrary JSON `LinkOptions` layouts.
The o65 path gets separate options; it must not silently flatten existing
`read_only_origin`/`zero_fill_origin` requests or collect gaps across unrelated
fixed-address objects into a huge payload.

### Entry, bindings and ABI metadata

Export `__a816_o65_profile_v1` for a length-delimited readonly descriptor in
text and `__a816_entry_v1` for the existing MIR program entry. The entry keeps
its declared native signature; the host caller supplies that signature's
arguments. The experiment does not invent a process-startup calling convention.

The descriptor is ordinary retained data, not an unregistered OS-option
number or metadata that must survive unknown header-option forwarding. It
contains checked section-relative ranges/offsets, not host pointers:

- Magic, descriptor length/version, required-feature bits, target and ABI
  identity, pointer widths and byte order.
- Entry contract, code/routine extents, object extents/alignment/permissions,
  and section placement requirements. Include enough map information to
  validate bank containment and let execution fixtures locate declared data.
- Named import contracts: binding kind, compiler signature identity plus
  explicit physical arguments/results, checked-stack requirement, stack-peak
  contract, return behavior, IRQ effect and permitted execution domain.
- Required task/IRQ headroom and the configured NMI allowance. Frame and call
  maps remain local costs; unknown whole-task stack bounds remain unknown.
- Bounds needed to validate all admitted address expressions before patching,
  including any one-past references. Do not infer full address bounds from
  only a stored low/high byte.

The schema must specify integer widths, count/length limits, string encoding,
offset bases, required fields and rejection of unknown required features.
Validate descriptor ranges against the actual decoded file; metadata is not
proof that arbitrary machine code follows the ABI.

Current imports use numeric symbol/signature IDs and final addresses.
For the experiment, an explicit compiler options table maps existing stable
interface IDs to case-sensitive ASCII linker names matching
`[A-Za-z_][A-Za-z0-9_]*`. Validate uniqueness, declared interfaces and structural
ABI facts; do not derive names by parsing routine display strings. IDs are
intra-compilation references, not the loader's public binding names. Signature
IDs alone are insufficient to establish compatibility across producers.

At load time a separate provider table maps those names to addresses, extents
and matching contracts. Resolve all imports eagerly. Reject missing, duplicate,
extra or incompatible bindings. Initially admit declared external routines
whose physical interface is retained; diagnose runtime bindings without that
information rather than consulting SemIR or weakening checks.

Always represent `__a816_stack_overflow_v1` as a distinct required raw,
nonreturning import. Its contract is not an ordinary Action procedure's
contract. User bindings cannot redefine reserved profile names. Preserve
the existing rules for ordinary calls and the two explicit IRQ-state helpers.

No stack or DP allocation is performed by the reference relocator. The host
supplies domains and initializes D, floor/ceiling and CPU entry state. Write
zero (unknown) to the generic o65 stack-size field; use the descriptor and
platform contract for admission. Preserve all generated reservation checks,
callee obligations and interrupt headroom.

### Relocation semantics and initial exclusions

Retain a typed patch site `(section, offset, encoding)`, target identity and
signed addend before absolute address resolution. Normalize aliases once.
Resolve local PER/branch displacements only after final intra-text layout.
Do not add an o65 relocation to a displacement that is invariant under the
permitted text movement.

| Retained expression | Encoding/policy |
| --- | --- |
| Complete 24-bit address | `SEGADR` |
| Explicit low/high/bank byte | `LOW`/`HIGH`/`SEG`, including required carry bytes |
| 24-bit address in four-byte storage | `SEGADR` over three bytes and a verified zero fourth byte |
| Absolute address plus addend | Resolve and range-check once; no load-time relocation |
| Full one-/two-byte value pointing into a movable section | Reject initially; do not reinterpret checked narrowing as low-byte/low-word extraction |
| General 32-bit arithmetic relocation | Reject |
| `ImageEnd` | Reject in this profile; independent section movement changes its current meaning |

The codec should understand/test standard `WORD` relocations, including external
fixtures, without using them to silently truncate compiler address values.

For internal section targets, require the effective target offset after the
signed addend to be inside that section or exactly one-past. Negative addends
are allowed when this condition holds. Record/check any one-past requirements
before permitting an extent ending at `$1000000`. Diagnose expressions outside
these bounds as unsupported for this experimental profile.

Initially require import addends to be zero. This covers direct calls, imported
function pointers and the fault adapter while avoiding loss of significant
addend bits in split external relocations. Reject nonzero import addends
explicitly; extending them needs a separate representability/bounds contract.

Relocate a section reference using its destination minus original base, and an
undefined reference using its resolved import value. Reconstruct carry bits
before selecting the byte. Use checked host arithmetic and explicit target
widths. Validate complete expression bounds before applying modular byte
selection; never silently accept a wrapped full 24-bit address.

## Commit-sized implementation slices

Each slice ends with its checks, documentation update and a dedicated commit.
Preserve unrelated working-tree changes and never stage the entire worktree.

### Slice 1: retain typed relocation facts and freeze the profile

1. Add the profile contract with the decisions above and annotated binary
   examples. Pin the specification/tool versions used for conformance.
2. Separate existing validation, placement and patch-site collection enough to
   produce a verified relocation-bearing representation. It contains section
   contents, routine/data ranges, target maps and typed fixups. Treat the
   proposed module split as a narrow extraction, not an image-linker rewrite.
3. Keep `image::link`, `Prepared::compile`, JSON schema/version and all existing
   diagnostics compatible. Share validation that is truly common; keep o65
   layout policy separate from fixed-image placement.
4. Cover both `Code::fixups` and `Mir65816Data::relocations`, including aliases,
   descriptors/backing storage, external entries and the overflow adapter.
   Every patch site must be classified once; overlapping writes are rejected.

Acceptance: existing fixed layouts still produce the same bytes and maps in
both modes. Focused tests prove target identity/addend retention, alias-cycle
rejection, patch bounds and all explicit profile exclusions. No NIR changes
should be required; if retained interface facts prove insufficient, stop that
feature with a diagnostic and scope any IR addition separately.

### Slice 2: deterministic o65 writer and independent wire checks

1. Encode the header, terminated options, text/data, undefined names, two
   relocation streams and exports. Emit the descriptor as part of text before
   final offsets are frozen. Ordering is deterministic and timestamp-free.
2. Sort patch sites and encode displacement runs correctly: the initial cursor
   is section start minus one, `$FF` advances by 254, and zero terminates a
   stream. Carry bytes and wide import indices must appear in the specified
   order. Reject duplicate/overlapping sites, unknown encodings and overflow.
3. Add the independent Python inspector and small hand-calculated binary
   fixtures. Check nonzero original bases, symbol-value interpretation and
   all five relocation encodings against small vasm/vlink fixtures with
   committed source, commands, versions and hashes. Do not treat stock vlink's
   wide-image writer as an oracle.
4. Prove header lengths/values for 65,535, 65,536 and 65,537-byte sections and
   symbol offsets above `$FFFF`; include relocation gaps of 1, 254, 255, 508
   and a longer gap, and import indices requiring more than 16 bits.

Acceptance: an independent decoder agrees on exact field widths, offsets,
payloads and relocation records. Byte fixtures cover low-byte carry, bank-byte
carry and positive/negative internal addends. Repeated writes are identical.
Wide structure encoding must not be inferred merely from the native CPU bit.

### Slice 3: bounded decoder and reference relocator

1. Decode with explicit limits before allocating from counts or lengths. The
   structural decoder may read narrow external fixtures, but the application
   profile requires wide mode. Keep structural decoding separate from profile
   admission so an ordinary o65 object is not mistaken for an Exec application.
2. Parse the entire file, descriptor, exports and both relocation streams;
   reject truncation, invalid segment/type IDs, unsupported mode/version bits,
   duplicate names, malformed options/strings and trailing chained payloads.
3. Validate requested regions, platform reservations, import extents, text-bank
   alignment, data alignment, code containment and ABI requirements. Check
   initialized storage/BSS/import/fault-adapter overlaps before mutation.
4. Resolve every binding and build a validated patch plan from serialized
   records only. Reject out-of-range sites, carry fields, indices, expression
   bounds and conflicting writes. Patch a private copy, clear BSS, resolve
   entry/exports and return a verified `RelocatedImage`.
5. Return loaded regions, zero-fill, entry, resolved bindings and validated
   descriptor maps. Do not manufacture a JSON `Image` with missing routine
   maps or weaken `Image::verify` to make the test harness accept the result.

Acceptance: hand-authored files load to independently calculated bytes at two
placements. Late failures leave input bytes, existing outputs and guest memory
unchanged. Parsing arbitrary/truncated small inputs does not panic or allocate
according to unchecked 32-bit counts. The input file remains reusable.

### Slice 4: compiler and command-line integration

Add a library API for experimental compilation and a public host reference
relocation API. Expose writer selection through the existing driver:

```text
actionc-65816 --format o65-experimental --o65-options options.json \
  [-o program.o65] [--no-opt] [--module-path directory] source.act
```

This is a proposed interface, unavailable until this slice lands. JSON remains
the default; `--layout` retains its existing meaning. Reject ambiguous option
combinations, including JSON layout options in o65 mode. The experimental
options carry profile identity, NMI allowance and explicit interface-name
bindings, not final load addresses. Provider addresses and placement belong
to the reference relocator's separate typed input.

Reuse validate-before-open, temporary-file publication and source/layout input
protection from `native65816::write`. Protect the new options file and module
inputs too, including canonical-path aliases. A failed compilation must leave
an existing output unchanged. The reference relocator is a host library and
test utility in this slice; a second production CLI is not necessary.

Acceptance: real CLI outputs load through the byte decoder in both modes;
defaults/help/errors/output protection are covered. Existing JSON CLI tests
stay green. Any new newline-sensitive source/assembly instrumentation is
validated with LF and CRLF; binary fixtures are never newline-normalized.

### Slice 5: emitted-code execution and integration qualification

Add a narrow native harness adapter that maps a validated `RelocatedImage`,
sets up the existing ABI domains and starts an independently assembled caller.
Keep the current JSON loader tests. Share bus/CPU setup only as needed and
retain region permissions and canary checks.

For each optimization mode, compile/serialize once and discard the compile
result before loading. Use two placement/provider configurations, with text
at different banks and data offsets chosen to force low/high/bank carries.
The loader receives no original MIR, `MachineProgram` or fixed JSON image.
Expected semantic outputs come from independent constants/reference assembly.

| Case group | Required evidence |
| --- | --- |
| Calls and pointers | Direct/indirect calls, stored local/imported function pointers, data/backing pointers, aliases, full/split references and a four-byte address container |
| Placement and storage | Cross-bank data access, zero tails, BSS initialized from nonzero host memory, readonly preservation, untouched absolute aliases, multiple code banks and final valid routine bytes |
| ABI and clobbers | Mixed-width arguments/results, helpers clobbering all permitted registers/64 DP scratch bytes, preserved D/DBR/I and balanced stack |
| Guards | Ordinary calls succeed; a deliberate reservation failure reaches the relocated raw fault import before stack corruption |
| Preemption | Two tasks sharing code with distinct DP/stack domains; reuse seeded IRQ/NMI schedules and selected instruction-boundary injection from existing tests |
| Rejection | Text shifted by a non-bank delta, overlap/reserved memory, bad alignment, address overflow, missing/wrong ABI bindings, malformed late relocation and unsupported expressions |

Change helper and fault-adapter addresses between placements so tests cannot
pass with accidentally baked-in host addresses. Check two independent loads
of the same artifact do not share mutable loader buffers. Do not claim all
globals become task-private: global sharing remains an application decision.

Use a synthetic structurally valid section fixture for exact size boundaries,
and a separate multi-routine generated program for code-bank execution. Do not
relax the existing single-routine size limit to exercise a 64 KiB header.
Snapshot/mapping checks must distinguish routines, readonly data, descriptor
bytes and padding instead of assuming the entire text segment is executable.

Acceptance: the matrix passes in host debug and release builds. Record artifact
hashes, placement/binding inputs, code/data/BSS/descriptor/relocation sizes,
relocation counts, VM results and observed stack use. Extend `qualify.py` input
hashing to include the new inspector and conformance fixtures. Commit a scoped
qualification record and update the profile/emission documentation with the
actual supported subset; leave unsupported capabilities explicit.

## Validation by slice

Commands below name both existing targets and proposed targets added by this
plan; proposed targets must exist before these commands are used.

- Slice 1: `cargo test --test mir65816_emission --test mir65816_abi --test
  mir65816_contract --test actionc_65816_cli`; add the new relocation cases to
  the appropriate target. Run native `execution`, `memory`, `interop`,
  `indirect` and `stack_faults` through the qualification runner because the
  fixed-image consumers share linking code.
- Slices 2/3: `cargo test --test mir65816_o65` plus the independent inspector's
  fixture checks. Run each codec/loader suite after its corresponding changes;
  include its wide-field and malformed-input cases in release qualification.
- Slice 4: `cargo test --test actionc_65816_cli --test actionc_65816_o65_cli`
  and the o65 integration target.
- Slice 5: run the complete isolated native workspace once in debug and once
  in release, using `python3 tools/native65816-runtime-tests/qualify.py` and
  its `--release` form. This covers shared harness/context consumers as well
  as the new `o65` target. Do not repeat unchanged passing matrices per slice.
- If implementation changes NIR, semantic lowering, verifier or printer
  contracts, additionally run the mandatory NIR snapshots, sweep and full
  `cargo test` from `AGENTS.md`. Otherwise scope root tests to affected code.

External tools are conformance aids, not compiler/loader runtime dependencies.
Routine tests should use committed tiny fixtures with reproducible provenance;
regenerating them must check the recorded tool identity. Do not vendor tools or
download an unpinned current release during tests.

## Adoption gate

Completion qualifies an experimental file writer and host reference relocator,
not an Exec816 runtime loader or vbcc ABI interoperability. A subsequent Exec
slice must use its actual memory allocator, reject unsuitable free extents,
resolve OS imports, initialize task domains and establish application lifetime
and unload ownership. Keep the XEX cold-bootstrap path independent until that
separate integration is qualified. Preserve the public ABI and stack guards
throughout.
