# Native 65816 constant addresses and BYTE indexing

Status: slices 1–3 implemented and checked; slices 4–5 remain planned.

Implement five small compiler slices, committing each after its focused checks.
The last implementation commit carries final qualification and measurements.
Keep the existing ABI, stack guards, image formats and Exec compiler pin.

## Objective and baseline

Reduce unnecessary address construction in native 65816 code, starting with
constant symbol addresses and then CARD-indexed BYTE accesses. Keep source
evaluation order and every observable memory access intact.

The initial measurements used Exec816
`bbb8f2c72ad238201e83528a5ad90cc4f95e7b39` command sources and local actionc
`7b078ea157e8986b0bfcfa281ea6312abf4d9bd3`. These are the current-compiler
measurements, rather than the older compiler pinned by the demo:

| Optimized command | Machine code, with guards | o65 file | Descriptor |
| --- | ---: | ---: | ---: |
| HELLO | 929 | 2,359 | 1,073 |
| CAT | 3,749 | 7,403 | 2,931 |

HELLO spends 81 bytes constructing `@greeting(1)`. Directly materializing that
symbol plus one into the same three-byte home suggests 14 bytes, including SEP.
This is a local instruction-count estimate, not a promised whole-program delta.
Its two `greeting(0)` reads, including LONGINT conversion, occupy 88 and 90
bytes. Plain `@newline` occupies 28 bytes.

CAT.Filename is 1,136 bytes. It contains five `text(index)` reads and two
`path(used)` writes with CARD indices and BYTE elements. These are acceptance
examples for general compiler behavior, not patterns identified by app names.

Freeze source hashes and compiler inputs before implementation, then compare
each slice to its immediate predecessor. Preserve raw and optimized baselines,
fixed-image listings and o65 measurements. Do not rebuild the complete Exec
qualification matrix for each slice.

## Existing implementation and ownership

Verified NIR already supplies the required address, type and storage facts.
[MIR lowering](../src/mir65816/lower.rs) retains structured addresses with a base,
displacement and optional index/stride. Its routine temp table retains NIR types,
so a two-byte value need not be guessed to be unsigned from its width.

The observed HELLO shape is a short chain: AddressOf(global), then
AddressOf(indexed temporary, constant one). Its BYTE reads similarly use an
AddressOf(global) producer followed by Load(indexed temporary, constant zero).
Optimizing only a directly written symbolic operand would miss these chains.

Relevant implementation points:

- [select.rs](../src/mir65816/emit/select.rs): `prepare_address`,
  `address_to_pointer`, `pointer_value`, Load/Store/AddressOf selection.
- [pointer_values.rs](../src/mir65816/emit/pointer_values.rs): existing fast paths
  for captured-pointer AddressOf. Preserve their coverage and fallbacks.
- [liveness.rs](../src/mir65816/emit/liveness.rs): exhaustive MIR operand uses,
  including address operands, edge arguments and terminators.
- [selected.rs](../src/mir65816/emit/selected.rs): existing LdaByte symbolic
  fixups, LdaLong/StaLong, TAY and long-indirect indexed instructions.
- [relocation.rs](../src/mir65816/relocation.rs): shared relocation collection;
  both fixed linking and o65 consume stable targets and addends.

Implement target selection in a small emitter module, proposed
`emit/addresses.rs`, with a checked per-routine address-selection plan. Reuse
existing MIR forms and the tracked emitter. Pass read-only MIR data/placement
facts to the planner where an object extent is required. Do not inspect SemIR,
parse printed IR, or infer addresses from source/debug names.

Keep the prepared MIR and allocated frame unchanged in these slices. The plan
may omit an unused pure address materialization from emitted instructions after
checking all its uses; removing its reserved home is separate work. This avoids
introducing another MIR preparation pipeline or weakening the linker's check
that machine code belongs to the prepared program.

## Common constraints

- Represent a known address as a stable data identity plus an addend. Preserve
  the distinction between global storage, static templates and array backing.
  An array descriptor's stored pointer is a memory value, not its own address.
- Learn symbolic provenance only from typed address-producing operations and
  explicit symbolic values. Never turn a Load, incoming pointer, call result or
  initializer into a constant address. Mutable array contents stay mutable.
- Limit producer-chain folding to one basic block and clear candidate chains at
  calls and other ordering barriers. No propagation through joins, backedges,
  edge parameters or memory. Initially follow AddressOf chains only; casts and
  general PointerOffset/Binary arithmetic retain their existing paths.
- Preflight the complete candidate, final homes and all uses before emitting
  instructions or suppressing a producer. A declined candidate leaves the
  existing path intact. Invalid MIR/homes remain errors, not silent fallbacks.
- Count operand occurrences across the whole routine, including repeated
  operands, unreachable blocks, calls, returns and edge arguments. Keep a
  producer whenever an unfused use remains.
  Preserve source-span ownership, including zero-byte spans for omitted pure
  operations, and verify replay and final layout metadata.
- Distinguish modular runtime pointer arithmetic from checked relocation
  addends. Fold a nonzero symbolic offset only when MIR storage/placement facts
  prove it remains within the valid object extent, allowing one-past for
  AddressOf only. Unproved aliases, wrapping offsets and unsupported geometry
  use runtime arithmetic. Do not convert a valid wrapping computation into a
  relocation overflow error or relax existing link/load checks.
- BYTE loads/stores remain exactly one byte. Do not cache contents, widen a
  memory access, remove a read across a call, or speculate/reorder a read/write.
  Volatile cases are eligible only with the same single target access and an
  exact trace regression; otherwise retain the existing path.
- Use existing per-domain scratch and private homes. No additional bank-zero
  reservation, allocator-whitelist expansion, stack pushes or ABI change.
  Check DP residents before touching PTR/INDEX scratch; unsupported conflicts
  retain the established path. Report the reserved bank-zero delta as zero.

## Slice 1 — Materialize symbolic AddressOf directly

Implemented. HELLO saves 48 code/file bytes and CAT saves 60 in both compiler
modes, with unchanged frames, guards, relocations and bank-zero reservations.
See the [slice measurements](benchmarks/65816-address-selection/README.md).

Commit intent: `65816: materialize symbolic addresses directly into temp homes`.

For a direct known data symbol and an admitted constant displacement, emit its
three address bytes straight into the checked destination home using existing
`ReferenceOp::LdaByte` fixups with the same target/addend and selectors 0, 1, 2.
Avoid the intermediate PTR writes and reads. Start with stack homes and leave
other geometries on their existing path; do not add new relocation encodings.

This directly improves `@newline` and the base producers in HELLO before chain
folding. Preserve full three-byte values, destination canaries and normal mode
repair. No array-content read is involved.

Checks: symbolic globals/static values, zero and proven nonzero displacements,
initialized/BSS storage, existing aliases, A8/A16 entry, highest legal stack
home, unused fourth-byte canaries, and two independent data/BSS placements.
Check serialized o65 execution as well as fixed linking so split-byte addends
and bank carries are tested through the real relocation path.

## Slice 2 — Fold constant address chains

Implemented. Both commands save another 55 code/file bytes in both modes.
The 81-byte baseline HELLO address chain now occupies 14 bytes, including its
entry width change. Pure producers disappear only after complete operand-use
accounting; allocated homes remain unchanged. One-past folding is conservatively
deferred because an object may end at `$1000000`; a relocated execution at that
placement confirms the fallback wraps to zero instead of failing at link time.

Commit intent: `65816: fold local symbol and constant-index address chains`.

Recognize the observed AddressOf(global) -> AddressOf(temp[index]) chain.
Initially admit constant nonnegative indices with stride one and checked
constant displacement. Resolve it to the original stable symbol plus the
combined addend; use slice 1's direct materialization for the final result.
Keep larger strides and signed/subtracting arithmetic on the existing path.

Build candidate plans before selection. Omit a pure producer only when all its
uses have accepted replacement plans. A base shared with an ordinary consumer
must still be materialized. Do not eliminate a capture from memory, even if its
initializer happens to name the same symbol.

Checks: offsets 0, 1, 255, 256 and 65,535 where object extents permit; explicit
bank carry under relocation; one-past AddressOf; nested constant AddressOf;
shared producers; returning/passing the address; call/barrier boundaries; edge
uses; mutable pointer descriptors; and fallback for offsets that may wrap or
exceed the proven extent. The HELLO `@greeting(1)` chain must stop using a
three-byte runtime addition and redundant base materialization.

## Slice 3 — Direct constant-index BYTE accesses

Implemented for ordinary one-byte loads and immediate/captured stack-byte
stores. Volatile accesses conservatively retain their established paths. HELLO
saves another 112 code bytes and 190 file bytes in both modes, including four
fewer relocation proofs; CAT is unchanged. Exact traces verify that reads around
a mutating call remain distinct, with matching volatile fallback behavior.

Commit intent: `65816: select direct BYTE accesses at constant symbol offsets`.

Reuse the checked provenance plan for one-byte Loads and Stores. Select
`LDA long symbol+addend` or `STA long symbol+addend` in A8, using existing
relocations and ordinary result/value homes. There is no DBR dependency.
Suppress address producers only under slice 2's complete-use rule.

Handle both reads and writes in this slice: each replaces address construction
with an existing direct one-byte instruction. Keep the value capture and
evaluation order supplied by MIR. A known address never proves its contents
constant. In HELLO, retain both greeting-length reads around Write and leave
LONGINT conversion optimization outside this slice.

Checks: first/last object byte, one-past dereference fallback, relocated bank
boundaries, read/write canaries and access traces, a call that mutates the BYTE
between two reads, aliased writes, and constant/captured store values. Mixed
read/write/address consumers must retain any still-needed producer. Record
both code and relocation-count changes; fewer instructions can also reduce the
o65 descriptor, but no format change is part of this work.

## Slice 4 — CARD-indexed BYTE loads using Y

Commit intent: `65816: use Y for CARD-indexed BYTE loads`.

Admit Load width one, stride one, zero residual displacement, a complete
24-bit base and a captured unsigned 16-bit index. Establish unsignedness from
the routine's retained type facts; width alone is insufficient. A direct
parameter without enough type evidence remains on the existing path.

Prepare the base once in the existing legal pointer scratch. Load the complete
index from its checked private home into A16 and execute TAY. Switch A to eight
bits and use `LDA [base],Y`, then capture the one-byte result. Symbolic bases can
use the earlier plan to initialize scratch directly; arbitrary pointer bases
retain their original capture from memory.

Keep Y16 throughout. Do not add the index to PTR or call a generic memory helper
that reloads Y with an immediate. Use long-indirect indexed addressing, which
retains linear 24-bit effective-address behavior. Do not substitute a DBR-bound
absolute indexed access. Keep X residency and the existing zero-index rewrite
correct; a dynamic Y must never be treated as zero.

Initially defer BYTE/wide/signed indices, scaled elements, nonzero residual
displacements, aggregates and AddressOf results. In particular, do not compute
`index+displacement` in Y16 and silently drop its carry.

Checks: runtime indices 0, 1, 255, 256, 32,767, 32,768, 65,534 and 65,535; bases
near bank ends; modulo-24-bit wrap on the VM bus; A8 entry with poisoned hidden
B; mutable parameter/index captures; base/result home overlap; signed/scaled/
wide-index fallback; exact BYTE traces; neighboring canaries; and interaction
with loop-X, mode tracking and checked rewrites.

## Slice 5 — CARD-indexed BYTE stores using Y

Commit intent: `65816: use Y for CARD-indexed BYTE stores`.

Reuse slice 4's address/index eligibility. Prepare the base, capture the full
index in Y, then load the already captured BYTE value or byte constant into A8
and execute `STA [base],Y`. Preflight that value loading does not overwrite Y or
the pointer scratch. Do not call address preparation again after setting Y.
Keep unsupported source homes on the existing path.

Apply to arbitrary captured BYTE pointers and known array symbols, including
CAT's `path(used)=value` and `path(used)=0`. Preserve the original order of
external reads and the final single write. Changes to A/N/Z/Y must be accounted
for through the tracked emitter; no register value survives a call by assumption.

Checks: slice 4's index/bank boundaries with exact destination writes, all 256
BYTE values across representative cases, zero/nonzero immediates, captured
values, source/destination aliasing, read-before-write behavior, scratch and
frame canaries, hidden B, live index/base values after the store, and volatile
ordering. Inject IRQ/NMI at reached preparation/access instructions to check
restoration of Y, scratch and task context through both load and store paths.

## Validation and measurement gates

Use general compiler fixtures rather than importing changing Exec sources into
unit tests. Proposed focused targets are `tests/mir65816_address_selection.rs`
and `tools/native65816-runtime-tests/tests/address_selection.rs`, plus private
selector tests beside the new module. Add only behavior/boundary tests, not
tests that duplicate an implementation's internal branching.

For each slice:

1. Run affected 65816 selector tests, the focused integration target, and
   fixed/o65 relocation tests relevant to changed symbolic references. Exercise
   both raw and optimized source compilation. Check new instruction sequences
   against independently assembled ca65 references where applicable.
2. Run focused emitted-byte tests through the pinned native qualification
   runner, for example after creating the new target:
   `python3 tools/native65816-runtime-tests/qualify.py --test address_selection`.
   Reuse `memory`, `pointer_values`, `o65`, `replay` and `instruction_effects`
   coverage where the slice changes their contracts; do not repeat unrelated
   backend suites. Use independent expected addresses, values and access traces,
   and both incoming interrupt-mask states for the runtime cases.
3. Verify physical effects and fresh replay with `native65816-state-proof`,
   including source spans/fixups and trace-on/off byte equality. All new
   instructions use existing typed forms; no raw-byte emission escape hatch.
4. Rebuild frozen HELLO/CAT with the candidate compiler and report code bytes,
   guard bytes, initialized data/BSS, relocation counts, descriptor/file bytes,
   stack frames and DP reservations separately. Record VM cycles on focused
   compiler probes; do not report fixed links with placeholder imports as
   executed commands. Code must shrink at admitted sites; record any cycle
   tradeoff and reject unexplained regressions.
5. Update the [emission contract](MIR65816_EMISSION_CONTRACT.md), this plan's
   status and the slice measurements, then commit only that slice's work.

Before the final slice's commit, run the full affected native 65816 unit,
integration/CLI and VM suites in the required host modes, plus one final full
Exec qualification against a frozen source tree and explicitly recorded
compiler override. Include real HELLO output/error behavior and CAT filename,
short-write, cancellation and error paths. Do not advance Exec's pin as part of
this compiler series. Keep inputs stable while qualification records hashes.

Follow [AGENTS.md](../AGENTS.md) for test scoping. If shared NIR, semantic,
verifier or printer contracts change despite this plan's backend scope, also
run fixture snapshots, the NIR sweep and full `cargo test`; do not claim a
backend-only exemption. Normalize host fixture text before newline-sensitive
processing and verify LF/CRLF through the actual path whenever such handling
is introduced or changed. Use the
[native runner instructions](../tools/native65816-runtime-tests/README.md).

## Completion criteria and deferred work

The series is complete when all five slices have focused regression coverage,
HELLO's constant address chains and BYTE accesses use direct symbolic forms,
CAT's eligible parser loads/stores use Y without bytewise pointer addition,
both output formats execute correctly at distinct placements, and final
qualification/measurements are recorded.

Deferred: general address CSE, cross-block provenance, pointer-descriptor
constant propagation, register allocation/frame compaction, Y loop residency,
non-unit strides, widened external accesses, general signed index arithmetic,
call-argument scheduling, loader allocation, relocation-proof compression and
guard-free release policy. None is a prerequisite for these slices.
