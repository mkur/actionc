# Native 65816 emission

The [experimental o65 profile](MIR65816_O65_PROFILE.md) retains typed emission
fixups for a separate relocatable output path. Its
[implementation status](MIR65816_O65_IMPLEMENTATION_PLAN.md) is tracked per slice.

The compiler emits freestanding machine code for `wdc-65816-native` under
[`action65816.native.v1`](MIR65816_PHYSICAL_ABI_V1.md).
[Initial Exec acceptance](MIR65816_EXEC_ACCEPTANCE.md) covers the subset below
on the VM's independent 24-bit bus, including context switching and interrupts.

Native integer MUL/DIV/MOD support and its qualification contract are recorded
in the [arithmetic helper plan](MIR65816_ARITHMETIC_HELPERS_PLAN.md).

## Compile an image

The initial driver is `actionc-65816`. It has separate platform options from
the Atari and Amiga drivers. The existing `actionc-emit` command still provides
65816 SemIR/NIR inspection.

Save this layout as `layout.json`. Addresses accept decimal JSON numbers or
quoted hexadecimal strings with a `0x`, `0X` or `$` prefix:

```json
{
  "code_origin": "0x018000",
  "data_origin": "0x120000",
  "stack_overflow": "0x048000",
  "nmi_extra_stack": 0,
  "imports": []
}
```

The same syntax applies to optional `read_only_origin`, `zero_fill_origin`,
`arithmetic_fault`, and each assembly import's `address`. For example, `98304`, `"0x018000"` and
`"$018000"` specify the same address. Hex digits are case-insensitive; bare
`0x018000` is invalid JSON. Addresses must fit in 24 bits. Sizes, stack budgets
and symbol/signature IDs remain decimal JSON numbers. Emitted image JSON
continues to use numeric addresses.

For runtime DIV/MOD, add `"arithmetic_fault": "0x049000"` and provide the raw
`__a816_arithmetic_fault_v1` adapter at that address. It receives A16=1
(DivisionByZero), X16=S and transfers by JML without a push or return. D, I,
DBR=0, native mode and M=X=0 are preserved. Its address must be distinct from
the overflow adapter and outside image/import storage. The dependency is
required only after constant reductions; multiplication alone does not need it.
Images using it serialize as v4. Other images retain v3, and the reader accepts
both versions with their matching dependency contracts.

The example places code from `$018000`, data from `$120000`, and expects the
platform's raw stack-overflow adapter at `$048000`. These are explicit layout
choices, not Atari board reservations. Create the output directory before
compiling:

```sh
mkdir -p build
cargo run --locked --bin actionc-65816 -- \
  --layout layout.json -o build/scalar.a816.json \
  fixtures/runtime/native65816_scalar.act
```

`--no-opt` disables shared NIR optimization. Optimized compilation uses native
loop promotion. Repeated `--module-path` options add module directories.
Compilation validates the image before publishing one self-contained JSON
file by rename. Loaded sources and the layout file cannot be output targets;
a compilation failure leaves an existing output intact.

The library entry points are
[`compiler::native65816`](../src/compiler/native65816.rs),
[`mir65816::emit::materialize`](../src/mir65816/emit/mod.rs), and
[`mir65816::image::link`](../src/mir65816/image.rs).
`Prepared::compile` accepts the explicit link layout. No emulator is a compiler
dependency.

## Experimental o65 output

Save these experimental options as `o65-options.json`:

```json
{"profile":"actionc.o65.experimental.v1","nmi_extra_stack":0,"imports":[]}
```

Compile with:

```sh
cargo run --locked --bin actionc-65816 -- --format o65-experimental \
  --o65-options o65-options.json -o build/scalar.o65 \
  fixtures/runtime/native65816_scalar.act
```

`--layout` is exclusive to JSON output. Both formats accept `--no-opt` and
module paths, and protect all loaded inputs when publishing output. For named
assembly imports, use `--emit-interfaces` to obtain the stable interface IDs,
then add options entries with `symbol`, explicit ASCII `name`, `stack_peak`,
`checks_stack: true`, optional `irq_effect` and `domains` (task=1, IRQ=2, both=3).
Final addresses are supplied to the reference relocator, not these options.

The library APIs are `Prepared::compile_o65`, `native65816::write_o65`, and
`mir65816::o65::{inspect, relocate}`. `relocate` takes serialized bytes and a
`Placement` with section bases, allowed/reserved regions, matching NMI allowance
and named `Provider` contracts/addresses/extents. It returns private loaded
regions, BSS and ABI/maps through `RelocatedImage` accessors. It does not write
guest memory, allocate task contexts or implement an Exec816 application loader.
See the [profile](MIR65816_O65_PROFILE.md) for the admitted subset and rejections.

The [o65 qualification](abi/action65816-o65-qualification.json) executes raw and
optimized files at two independent text/data/BSS placements, including imports,
multi-bank code, stack failures and preempted tasks. The complete 44-test native
suite passes in debug and release. JSON transport v3, physical ABI v1 and
generated stack checks are unchanged.

## Instruction state boundary

Native selection uses the private `TrackedEmitter65816` in
[tracked.rs](../src/mir65816/emit/tracked.rs). The admitted forms live in
[selected.rs](../src/mir65816/emit/selected.rs); one exhaustive dispatch derives
[physical effects](../src/mir65816/emit/effects.rs) from the pre-instruction
environment and invokes the existing encoding/state update. The mutable encoder
and `State65816` are private to
the facade; selection can inspect finalized bytes and metadata but cannot write
raw instructions or attach caller-supplied effects. Linking retains its existing
ability to patch finalized `Code` buffers.

Physical effects describe A/X/Y bit lanes, independent N/Z/C/V reads and writes,
protected environment changes, control flow and ordered memory accesses. Their
precision is independent of forward value facts. Operand encoding size does not
determine memory width: a `ByteOp` stack/DP operand can access a word, while LDX
uses index width. X8 narrowing defines zero high X/Y bytes; A8 operations retain
the hidden accumulator byte unless the instruction explicitly uses it.

Instruction memory ranges remain relative to instruction-entry S, active D or a
typed symbol/address. Analysis normalizes these into invocation-entry-relative
stack bytes and current-domain DP bytes with verified ownership and conservative
aliasing; see [physical homes](MIR65816_HOME_ANALYSIS.md).
Read/modify/write reads precede writes. Indirect addressing
reads its three DP pointer bytes before the data access; indirect stores and
callee clobbers are possible writes, which cannot kill a reaching definition.
Unknown/absolute/indirect source accesses remain protected barriers.

Production calls carry a summary constructed from the verified native call plan:
logical stack arguments are read before conservative callee memory effects,
declared result lanes include ABI zero extension, and other A/X/Y lanes, flags
and all 64 scratch bytes are call-clobbered. A discarded result still has its
declared ABI effects. Logical argument extent is distinct from padded outgoing
reservation. Direct JSL and indirect RTL entry retain distinct transfer and
return-stack phases; the indirect RTL is a call, not a routine return. Unannotated
probe calls retain all register/flag inputs and unknown memory effects.

Call construction checks the complete outgoing and transfer reservation before
changing S, then defines every argument byte and zeroes alignment/tail padding.
Direct calls with exact-width captured/numeric one-to-four-byte operands and
exact-width symbolic byte-fixup operands can construct the complete area
downward using native PHA chunks. Symbolic bytes retain their target, addend
and byte selector; adjacent symbolic bytes are never combined into a word. A complete-call width plan
includes padding, post-guard width permission and final A16 restoration; it must
beat reservation/stores in encoded bytes. Every source byte is checked before
emission at the conservative full outgoing delta, which also bounds each
smaller incremental delta. Sources remain above the fresh outgoing area.
Other calls keep full reservation followed by stores. Source evaluation occurs
before either private construction strategy; the completed ABI layout is identical.

Selected `ArgumentPush` encodes PHA with its ordinary width-sensitive physical
effects. It increases outgoing depth without entering the indirect-transfer
push phase. Replay and selected-CFG validation check that phase separately;
native direct calls retain their checked outgoing extent and require exactly
that depth at transfer. Home effects use the actual S before each instruction,
including the descending writes of word PHA. Partial construction remains
interruptible with the existing stack/DP context contract.
The verified stack argument homes,
not their aggregate extent, identify payload: holes within that extent remain
zero and are written exactly once. No-argument calls retain their one-byte zero
area. Nonempty padding is initialized in A8; an unpadded reservation omits that
setup. The first payload width is still stated explicitly after the guard join.
Source order and extension, indirect target capture, transfer and cleanup
are unchanged. All bytes are initialized before transfer.
Caller home accesses retain the outgoing S delta and cannot overlap the fresh
outgoing area. Context restoration may resume partially constructed arguments;
no new helper, persistent scratch or interrupt-masking assumption is introduced.

Call payload selection preflights captured temp/parameter homes and numeric
constants against their declared widths, the complete outgoing extent and the
prospective S delta. Native copies use A16 pairs. Complete three-byte private
homes and numeric constants may use two words at offsets zero and one, remaining
in A16. Only the private middle byte repeats; the destination remains within its
own argument slot, disjoint from all source homes, other arguments and padding.
No fourth pointer byte is accessed. The word-plus-A8-tail form remains available
when smaller. All other payload bytes retain exactly one write.
Symbolic and mixed-width operands keep bytewise fixups and extension behavior.
A bounded two-state width choice minimizes encoded argument bytes, including
mode changes, the initial mode permission and the next direct-transfer or indirect-
target preparation width. It preserves declaration order and prefers the byte
path on a tie, then the non-overlapping native path over overlapping words.
Source-memory reads remain separate MIR operations and keep their ordering.

Result homes are checked against the declared ABI lanes before any call
emission. After result-preserving caller cleanup, BYTE capture stores A's low
byte, word capture stores A16, three-byte capture stores A16 plus X's low byte,
and four-byte capture stores A16/X16. Captures write only their owned bytes,
finish in A16 and introduce no DP staging or forwarding permission. Discarded
results retain declared call effects but need no capture stores or A preservation
during outgoing-area cleanup. Used results retain the Y-based preservation of
the complete A/X result. Calls remain
barriers and all guards, allocations and ABI stack costs are unchanged. See the
[native call measurements](benchmarks/65816-native-calls/README.md).

A direct native call immediately followed by Return of its sole-use result
temporary may keep matching native ABI result lanes through outgoing cleanup
and frame teardown: BYTE/word in A, or 24/32-bit values in A/X. Neither release
uses X as scratch. BYTE's high A byte and a pointer's high X byte retain the
zero extension required of the callee. Typed occurrence counting includes all blocks,
address operands and edge/terminator uses. Other consumers, indirect calls,
intervening operations and differing result conventions retain capture/reload.
The complete call preflight still checks the reserved result home and native
contract before emission. Allocation remains unchanged; the omitted capture
publishes no home definition. Call and Return retain separate source spans,
and typed call/return effects and replay still describe the actual instructions.
This is ordinary JSL/cleanup/RTL using the declared callee ABI.
See the [call-result measurements](benchmarks/65816-call-results/README.md).

The state owns width-qualified immutable A/X/Y values, N/Z provenance, C/V,
execution modes and environment, exact private stack-home generations, stack
movement and the existing single-use adjacent-word permission. DP and unknown
writes conservatively invalidate memory relations. Source memory is never cached.
Calls clear value/flag/home relations; I preservation becomes unknown because
import IRQ effects are resolved later by linking. Every label discards value/flag
optimization facts. Internal labels revoke mode-omission permission. Reachable
MIR entries may retain A16 permission only under a checked native A16/X16 body
contract: the ABI/prologue supplies the initial edge, and every CFG predecessor,
including both branch arms and later-emitted backedges, must discharge its
execution/stack obligation before finalization. Unproved/dead entries retain
explicit mode requests. Seeded label environments alone are not proof; a missing,
duplicate or incompatible transfer is rejected. This never retains values,
home relations or forwarding permission across joins, nor omits a needed SEP.

TSC/TCS use bounded stack-address equations. The body anchor, outgoing argument
displacement and transfer pushes are distinct: JSL has a three-byte peak, and
indirect PHK/PER/PHA/RTL has a six-byte peak with return facts applied at resume.
The original guards, overflow A/X/S state, homes, stores and ABI remain unchanged.
The foundation was byte-identical; subsequent checked MIR-entry width omission
removes only redundant REP instructions and shifts code positions accordingly.

The default-off `native65816-state-proof` feature exposes only immutable snapshots
and checked probes through `emit::proof`. Ordinary compilation collects no trace.
The separate `instruction_effects` observer exposes physical effects and encoded
ranges, remapped after branch relaxation, without changing historical snapshots
or executable formats. Production compilation independently retains a selected
action stream and CFG as described below. Existing adjacent temporary forwarding
now consumes these facts through the checked rewrite driver, without broader
eligibility or new optimizations.
Qualification compares known values and simultaneous register/home/NZ relations
against independent VM execution and ca65 encodings, including rebased o65 code.
See the [implementation plan](MIR65816_STATE_TRACKER_IMPLEMENTATION_PLAN.md) and
[design](MIR65816_STATE_TRACKER_DESIGN.md) for the foundation and deferred work.

### Selected actions and CFG

Every production routine retains a private `SelectedRoutine` containing the
allocation snapshot, verified home ownership, typed actions, source attribution
and selected CFG.
Recording is independent of optional traces and does not direct selection.
Instructions occur once, including the indivisible inverse-branch/JML form.
Nested request/end markers preserve the inputs and ownership of compound
facade calls without duplicating their emitted instructions. Requests include
mode changes even when REP/SEP is omitted, body anchors, home registration,
barriers, capture/consume attempts, MIR-entry obligations and X operations.
Recorded inputs and environment observations never grant replay permission.
Boolean request outcomes are recorded at the matching end marker solely to
check the freshly recomputed result, including failed single-use consumption.

The graph implements the shared `DataflowGraph` interface with action sites as
nodes. It includes labels, internal compare/staging paths, both conditional
successors, zero-byte fallthrough, return exits and stack-overflow exits. Calls
are intraprocedural summaries. An indirect RTL resumes at the exact label named
by its PER; labels sharing a PC do not become the same site. Native return and
indirect transfer retain distinct stack and control contracts.

Construction validates label ownership, request nesting, instruction widths,
stack changes, body anchors, reachable environment joins and MIR predecessor
multiplicities/reachability. Unreachable metadata following a terminal transfer
does not create a physical path. Joins may discard environment facts but cannot
invent them. Unsupported or inconsistent boundaries reject construction.

Sites carry an owner token, RoutineId, allocation generation and selection
generation. A site from another compilation or generation is invalid even if
its routine and ordinal match. Byte offsets are a derived map; branch relaxation
remaps only this map and preserves identities and graph edges. MIR source spans
remain separate: a fused compare explicitly records its terminator attribution,
and empty spans retain their zero-byte events.

Before and after layout, typed actions reconcile with every encoded byte,
symbolic/PER fixup, label, MIR span/transfer, conditional dispatch and optional
effect observation. Historical state trace PCs must remain on selected
boundaries. Proof-feature queries expose immutable observations and reject
foreign/stale sites. Immutable snapshots provide checked physical-home liveness,
stored-definition queries, and [register/flag liveness](MIR65816_MACHINE_LIVENESS.md).
These observations do not independently authorize an optimization. The
[checked driver](MIR65816_CHECKED_REWRITES.md) also requires sealed rule-specific
equivalence, exact original actions, declared effects, protected event checks
and fresh replay before atomic publication. Every accepted edit advances the
selection generation and invalidates all prior facts, sites and plans. Each new
immutable snapshot constructs fallible home access facts, then computes home
liveness, stored definitions and machine liveness once when their queries demand
them. Queries validate sites first; an uncomputed result never means safe or dead.
Removed-definition and post-replay undefined-read checks remain mandatory.

### Authoritative typed replay

Selection first records the existing choices through the tracked facade.
[Replay](MIR65816_TYPED_REPLAY.md) then executes those typed inputs through a
fresh facade with the native entry contract. Its immutable input owns a CFG
verified at construction; replay does not rebuild that input graph. Edited and
replayed outputs must pass their own constructor validation. Stored state and old success
answers are never used to seed permissions. Mode omissions, home generations,
single-use captures and X refresh obligations are recomputed. A compound request
regenerates its nested actions, whose inputs, effects, boundaries and outcomes
must match the original recording; its children are not executed a second time.

Label identities and source endpoints are symbolic. Replay derives fresh byte
positions for spans, transfers, fixups and traces, preserving owner/allocation/
generation identity for this exact replay. Rebuilt output passes reconciliation
and the unchanged layout finalizer once before flat-image or o65 serialization.
The direct reference path is available only to proof-feature qualification.
No public ABI, serialization, frame, stack-guard or interrupt-reserve policy
changes. The closed checked driver constructs scratch edits from compiler-owned
verified recordings and publishes only their regenerated output. Malformed
plans return blockers; rollback does not rely on catching facade assertions.

## Supported operations

Adjacent eligible word operations may forward a private stack temporary in A16.
An ordinary direct, nonindexed two-byte Load or native word ADD/SUB establishes
the fact only after its retained private store. The next native ADD/SUB,
materialized/fused word comparison, A16 return, or ordinary direct two-byte
Store may omit that same temporary's LDA. Selection checks TempId, the exact
allocated slot, zero transient stack displacement, known A16 and an unchanged
instruction/label cursor. The producer's full-word N/Z must still match A;
unchanged A alone is insufficient. Comparison operand swaps retain identity.

The [checked adjacent rule](MIR65816_ADJACENT_CHECKED_FORWARDING.md) retains the
actual typed load candidate before omission. The adapter reconstructs all
projected-away loads in one traversal, reindexes symbolic links once and verifies
the original continuation. It retains planning blocker diagnostics without
repeating the local proof. Current-generation rediscovery checks exact consume
inputs and actual load correspondence. The planning projection may guide
existing selection only with equivalent A/N/Z facts; the driver authorizes each
final removal and preserves single-use consumption even for failed attempts.
Rejected final proofs retain the ordinary load and revalidated continuation.

All stores and homes remain. Calls, helpers, labels, edges, stack movement,
other operations, intervening instructions and mode changes invalidate the
fact. Volatile, indirect/indexed, DP, byte and wider transfers retain their
original paths. Complete
operand/extent preflight still runs before a load is omitted. Each omission
removes only two private stack-byte reads; ABI, allocation, guard and interrupt
contracts are unchanged.

A separate frame-word witness permits an ordinary two-byte Store of a word
temporary followed immediately by a Load of the same `AutomaticFrame` object
and byte displacement. The object must be non-addressable, both bytes must fit
its extent and the stack-relative range, and the destination must be a checked
two-byte stack temporary. Selection registers the exact object word before its
retained STA. The tracker records immutable A/N/Z, the home generation, typed
object identity, physical slot and instruction/label cursor. The consumer must
match every fact with zero transient stack displacement. It removes only LDA;
both the object store and destination capture remain. The capture may establish
the existing temporary witness for its next eligible consumer.

Frame and temporary identities are distinct even at an equal physical offset.
No permission survives an intervening instruction, label, call/helper, other MIR
operation, mode transition or S movement. Addressable objects, aliases, volatile
and indexed/indirect accesses retain their loads.
Interrupt qualification checks full register and invocation-frame restoration
at both retained stores in separate task domains. Neither witness caches shared
source memory or reorders memory effects. See [frame forwarding](MIR65816_FRAME_FORWARDING.md).

Incoming parameter words have a separate bounded read witness alongside the
ordinary Temp/Frame witness. Only a checked, actual LDA16 from a canonical,
immutable two-byte `StackArgument` may establish it. Typed routine inspection
rejects address escape, writes, Copy use and noncanonical parameter access, even
if home metadata claims immutability. Final allocated incoming displacement and
both capture bytes are checked before emission. Incoming read facts never admit
writes to argument memory as private homes.

The next Load of the same ParamId and exact home may omit LDA while retaining its
capture STA. The sole permitted extension is one Store of the exact captured
TempId/home to a disjoint non-addressable frame word. It must consume ordinary
temp forwarding and emit exactly one STA. Both transitions require matching
read/capture generations, A16 and full N/Z, zero transient S displacement and
exact instruction/label cursors. Every other operation revokes or stales the
permission. An omitted read does not rearm it; the retained capture still
publishes ordinary temporary forwarding. Calls, helpers, joins and preemption
retain the existing invocation ownership and restoration contracts. ABI, guards,
DP use, homes and stores are unchanged. See the
[implementation plan](MIR65816_PARAMETER_FORWARDING_PLAN.md).

`Code.mir_spans` is nonserialized emission proof metadata keyed by MIR block and
operation index (ops.len() denotes the terminator; a fused comparison includes
its edges). Qualification combines these ranges with verified MIR identities,
allocated homes and actual machine instructions. It does not use the metadata
to execute code or publish it in image/o65 formats.

- BYTE, CARD/INT, ADDRESS/SIZE, data/code pointer storage, and LONGCARD/LONGINT
  retain their physical widths. Direct/typed indirect calls and returns use the v1 scalar ABI.
- Loads, stores and address formation cover automatic objects, incoming
  arguments, globals, absolute addresses, pointer dereferences and indexed
  fields/elements. Pointer arithmetic and constant-stride indexing retain all
  24 bits. Signed narrow displacements are sign-extended; wide displacements
  use their low 24 bits, with pointer movement modulo 2^24. Static initializers, zero-fill, aliases and low/high/bank relocations
  are linked by stable identities.
- Integer addition, subtraction, negation, AND/OR/XOR, all six comparisons and
  integer/pointer casts are emitted. Signed comparison and signed widening use
  the retained typed facts. Arithmetic follows the NIR operation's width;
  notably, Action! unary minus on SIZE currently produces INT. Use a SIZE
  subtraction when the intended operation is modular 24-bit subtraction.
- MUL16/MUL32 use modular shift/add helpers. Unsigned DIV/MOD support 8, 16,
  24 and 32 bits; signed DIV/MOD support 16 and 32 bits. Signed division
  truncates toward zero, remainder follows the dividend, and MIN/-1 wraps.
  Helpers use ordinary checked JSL/RTL calls, a zero-byte frame and D+$00..$13
  scratch at most. The core runs with M=X=0; byte/24-bit argument tails use
  brief A8 copies and results zero unused lanes. The restoring divider retains
  the extra carry bit above its remainder. All instructions pass through the
  tracked emitter and selected-action replay.
- Typed constants select MUL by zero/one/powers of two and unsigned DIV/MOD
  by nonzero powers of two before helper collection, in both raw and optimized
  modes. Signed DIV/MOD retain their helpers. Source loads and calls remain in
  order and execute once, including when a reduced result is zero.
- Logical left/right shifts operate at the typed width, including signed
  integer operands. A count at least the bit width produces zero, following NIR
  semantics. Numeric constant counts use byte moves, zero fill and bounded
  residual A8/A16 shifts; all input bytes needed by the result are captured
  before destination writes. Variable counts retain the checked loop. Both
  strategies use only current-domain scratch. See the
  [constant-shift contract](MIR65816_CONSTANT_SHIFTS.md).
- Indexed address scaling shifts its private 24-bit index only between stride
  bits. The final accumulated pointer is authoritative; the discarded index
  and shift flags have no consumer. Source accesses and pointer carries retain
  their existing order and width.
- Ordinary whole-aggregate copies preserve source-value semantics on overlap.
  Byte loops use full-width pointers and per-domain scratch; they make no calls
  or temporary stack pushes. Local initializers execute on each entry, with
  descriptor cells separate from their invocation-owned backing.
- `USE A816MEMORY` with `--module-path runtime/65816` provides
  `Move(BYTE POINTER destination,source SIZE length)` and
  `Clear(BYTE POINTER destination SIZE length)`. Move handles overlap; both
  operate on ordinary contiguous memory and use invocation storage.
- Branches, loops, direct/mutual recursion and block-parameter transfers are supported.
  Parallel edge copies preserve every source until consumed; cyclic edges save
  sources endangered by earlier assignments before writing destinations.
- Volatile accesses remain ordered byte accesses. A byte operation does not
  touch its neighbor. Wider volatile operations are not claimed to be atomic.

Nonempty edges whose arguments and parameters are all exactly two bytes may use
native A16 LDA/STA. Complete preflight checks stack sources, authoritative mutable
parameter homes, destinations and the target label, including transient S
movement. Staged paths also check capacity and the entire accessed byte range
of each required slot. A single word assignment loads its entire source into A
before storing directly to the destination; it needs no staging reservation.
Direct full-word self-copies omit their stores. Multi-word edges with disjoint word
destinations and no partial source/destination overlap use direct copies when
their dependency graph is acyclic. A stable topological schedule consumes each
source before another assignment overwrites it, preferring the original final
assignment last. Direct self-copies omit their LDA/STA pairs after all logical
operands pass preflight. If the last actually emitted assignment is not the
original final one, a final LDA from its destination restores full A and N/Z.
Thus a last-self or all-self edge, including a single self-copy, retains a final
LDA. Logical edge arity remains unchanged; staged fallbacks retain all stores.
For cyclic whole-word geometry,
selective staging captures each stack source overlapping an earlier destination,
then performs all assignments in original argument order. Captures precede every
destination write and use distinct invocation-owned words; repeated endangered
sources are saved separately. The final assignment preserves full A and N/Z
without a reload. Partial or unproved overlaps retain complete word staging.

Allocation, verification and selection share an explicit strategy/capture plan.
Logical captured move indices are distinct from physical scratch offsets. Pool
slot k holds the kth capture, so captured moves [1, 4] use slots [0, 1]. Each slot
reserves the maximum requested width at that capture ordinal across all edges.
Every logical mapping and accessed scratch range is checked before emission. Mixed-width and legal unsupported nonempty edges
retain bytewise emission and reserve their full widths. Internal labels reset mode
permission; proved MIR entries use the contract above. Word edges restore A16
when needed. No DP traffic, pushes, calls or wider
external memory accesses are introduced. Final frame extent, incoming parameter
displacements, spill bytes and local peak reflect compact staging. Guard logic
and public ABI placement remain unchanged.
Each directly scheduled word removes a staging store/reload pair: two private
stack-byte reads and two writes, two instructions and ten cycles. A necessary
final A/N/Z reload adds two stack-byte reads, one instruction and five cycles.
Word loads read both bytes before the corresponding store. Private edge-copy
access order may change; source-language memory access order remains unchanged.

Nonempty edges containing only captured three-byte Temp/Param arguments may use
overlapping A16 word copies between complete private stack/DP homes. Destinations
must be disjoint, and source/destination overlaps must be exact identities.
After removing identities, a stable schedule consumes each complete source
before another move overwrites it. When only cycles remain, it captures one
destination's old pointer in a private three-byte staging slot and redirects
its pending uses there. The resulting chain consumes that capture before the
slot is reused for another cycle. Both word pieces of a pointer move finish
together; they are never scheduled independently. Authoritative parameter
homes, transient S movement, the target and every accessed byte pass preflight
before emission. Partial overlaps, constants and mixed-width edges retain
their existing fallback.
An edge with any nonidentity moves reserves one invocation-owned two-byte staging
word to save the original full A. Cyclic edges additionally reserve one
three-byte capture slot, shared by all cycles on that edge. Both staging slots
belong to the invocation's fixed frame, so interruption and reentrant calls
cannot overwrite them. After the scheduled copies, it reloads the A-save
word, loads the original final assignment's destination bank byte in A8, and
restores A16. This preserves the bytewise fallback's full A, including hidden B,
and final N/Z even when moves were reordered or the final assignment is an
identity. All-identity edges reserve no staging and omit the copies, but retain
the final bank-byte load and width repair. C/V, X/Y, S, D, DBR and I are preserved. Allocation, final verification
and emission share the same checked copy/staging plan, including rejection of
either staging slot overlapping a live pointer home or the other slot. Compact
staging is reflected in the frame extent, incoming argument displacements,
stack peak and guard amounts. No fourth pointer byte,
new DP reservation, external access, push or call is introduced.

Empty edges validate the target and arity and restore A16 only when local mode
knowledge requires it. They never select A8.
Known A16 needs no mode instruction; A8 or unknown knowledge requires REP #$20.
Internal branch labels still revoke omission permission. This changes no branch decision,
nonempty copy, stack guard, frame, register value or data-memory access.

All MIR edges retain a typed logical transfer identity. A final Goto/Fallthrough
or final true arm may omit JML when the intended successor is the physically
next MIR block, after all required assignments. The pending target must be the
next binding; intervening instructions, other labels and unfinished fallthrough
are rejected. The path remains logically closed and the successor retains its
normal barriers and width contract. Other local transfers retain their logical edges and use the layout encodings
below; calls and external fault transfers remain long. Block order, copy scheduling and staging
reservations are unchanged; no jump threading occurs.

Every selected local conditional, including materialized comparisons, signed
correction, shifts, casts and helper loops, retains a typed predicate and label.
MIR/guard dispatch provenance remains separate from encoding choice. Exact
local JML references retain their label and final encoding. Routine-local
finalization jointly shortens conditionals to their two-byte predicate and
local jumps to BRA (two bytes) or BRL (three bytes). It starts from long forms
and shrinks to a fixed point, checking signed-byte/word displacement in each
candidate's own shortened layout. Out-of-range transfers retain their original
long forms; conditional compounds without short reach remain six bytes.
Calls and external fault transfers keep their original encodings.

Each stack guard retains every check, its order and amount, and its success/fault
register and flag behavior. Its local fault-arm JML becomes BRA; its external
overflow JML remains. No values, widths, copies, frame/home allocation or
source-memory accesses change.

One checked position mapping updates labels, retained absolute fixups, PER sites,
MIR spans, logical transfers, local transfer records, selected instruction ranges,
effect ranges and immutable trace PCs together. Selected actions, CFG and effects
remain unchanged. Labels or unrelated metadata inside removed instruction bytes
are rejected; coincident labels, empty spans and trace event ordering remain
valid. Reconciliation checks actual final encodings and relocation ownership.
Relative operands are finalized constants, validated against their targets and
protected from overlapping relocations. Both writers pack final routine sizes;
placed instruction/next-PC/target addresses must share PBR without low-word wrap.
Existing bank-contained routines and o65's aligned text relocation preserve
local displacement bytes. No image/o65 profile or loader extension is required.
See the [local transfer plan](MIR65816_LOCAL_RELAXATION_PLAN.md).

### Scalar instruction selection

Two-byte integer ADD/SUB may use native sixteen-bit A with one ADC/SBC and
an immediate store to the existing two-byte stack result home. Eligible operands
are two-byte stack temps/parameters and U8/U16 constants; U8 constants are
zero-extended and signed widening remains an explicit Cast. Selection checks
both bytes of every source/destination against the stack-displacement limit,
including transient S movement, before changing code or mode knowledge.
Legal unsupported forms retain bytewise emission; malformed locations remain
errors. This path uses no DP scratch, temporary pushes, or persistent registers.
It preserves the current allocation, call barriers, guards, and ABI. A volatile
load captured in a private temp may feed word arithmetic; the original memory
access itself is neither combined nor widened.

Four-byte LONGCARD/LONGINT ADD/SUB may use two A16 ADC/SBC operations,
low word first, writing each result word directly to its existing stack home.
The first operation establishes carry/borrow with CLC/SEC; its store and the
following high-word load preserve carry into the second operation. Arithmetic
is modulo 2^32 for both types and relies on the decimal-clear ABI. The final
A and N/Z/V describe the high-word operation, not a complete long result;
no whole-temp accumulator identity or persistent arithmetic flags are recorded.

Eligible sources are complete four-byte stack temps/parameters and numeric
U8/U16/U24/U32 constants. Narrow constants zero-extend; signed widening of
captured values remains an explicit Cast. Preflight checks both words of every
source and destination, including the fourth byte after transient S movement,
before changing code or mode knowledge. Each source home must be identical to
or disjoint from the destination; partial overlaps and other legal unsupported
forms retain the bytewise fallback. Malformed homes remain errors even when
another operand is unsupported. Mutable parameters use their authoritative
frame homes. No DP allocation, scratch, X/Y use, pushes, helper call, frame/ABI
change or additional external access is introduced. Original volatile and
aliased captures remain separate and unchanged. Guard policy is unaffected.

Two- and four-byte AND/OR/XOR use the same complete-operand preflight as native
ADD/SUB, with one A16 operation and result store per word. Two-byte operands may
also use already-admitted scalar DP homes; this does not expand DP allocation
eligibility. Four-byte operands retain the stack/immediate restriction and
whole-identity-or-disjoint geometry. Numeric widening, unsupported forms and
malformed-home handling are unchanged. Signedness does not change the bitwise
representation or result. No DP staging, carry setup, helper or extra source
access is needed.

Typed stack-relative AND/EOR and word-immediate ORA participate in tracked
selection, effects and replay. Stack operand encoding remains one byte while
its memory extent follows M. Logical operations read/write A and write N/Z,
preserving C/V, X/Y and environment state. A four-byte result leaves only its
high word in A/N/Z and never establishes a whole-long accumulator identity.
External/volatile captures remain complete and in source order. See the
[pointer/bitwise measurements](benchmarks/65816-pointer-bitwise/README.md).

Nonvolatile two-, three- and four-byte constant stores use A16 word pairs,
with an exact A8 tail for three-byte destinations. Numeric constants, numeric
addresses and NULL are eligible; source-width masking preserves bytewise
truncation and zero extension. Signed widening remains an explicit Cast.
Address preparation is unchanged. Selection then checks the complete resolved
destination extent before emitting any constant-store prefix. Each destination
byte is written once, in ascending order; no destination read, overlapping
word, fourth pointer byte, scratch allocation or additional helper is introduced.

An immediate is loaded once when both 32-bit halves match. The three-byte tail
also reuses A's low byte when it matches the bank byte. Reuse is local to the
operation: STA and destination LDY preserve A, and no ambient accumulator fact
is consumed or published as a value identity. Three-byte stack/DP stores starting
in A8 retain the byte path when a distinct bank byte would make the native
sequence larger. Volatile stores retain their original byte instructions;
symbolic source addresses retain their byte fixups. Symbolic destinations still
use ordinary long-address relocations. All mode requests, loads and stores use
the tracked emitter and its existing replay checks. No ABI, allocation, guard
policy or external memory-access contract changes.

Two-byte comparisons may use one native CMP for equality/inequality (signed or
unsigned) and unsigned ordering, using the same checked word sources. The result
must have an exact one-byte stack home. Materialized results are stored as 0 or 1
in A8. Selection checks the destination and both inputs before changing code, labels or mode
knowledge. Legal unsupported sources/destinations retain
bytewise emission; malformed homes remain errors. CMP flags are consumed within
the operation before loading the Boolean, except for the adjacent branch fusion
described below. Both inputs are read before the result store, allowing
existing dead-input slot reuse. Complete-word reads may increase private stack
read traffic compared with the old high-byte early exit; original volatile or
aliased source accesses remain separate and unchanged. No DP scratch, pushes,
helpers, X/Y use, allocation change or ABI change is introduced.

Signed two-byte ordering uses A16 SEC/SBC over the same checked word sources,
then a typed BVC skips EOR #$8000 when there is no overflow. Corrected N
encodes the signed relation: Lt/Ge subtract left minus right and use BMI/BPL;
Gt/Le swap the captured inputs and use BMI/BPL. Corrected Z is never used for
inclusive ordering: overflowing unequal inputs can correct to zero. SEC makes
incoming C irrelevant; SBC establishes V. Binary arithmetic relies on the
existing decimal-clear ABI. The correction join has conservative value facts,
and the signed path retains the operation barrier, with no new forwarding,
DP allocation, source-memory reordering, I changes or persistent flag facts.
The internal BVC relaxes like other local branches, retaining its distinct
provenance from MIR dispatch.
Word EOR writes A/N/Z while preserving C/V; BMI reads N and BVC reads V.

Exact-width BYTE comparisons use A8 CMP for unsigned relations and signed
Eq/Ne. Greater-than and less-or-equal swap captured operands; they never reorder
source-memory accesses. Exact-width three-byte Eq/Ne normally compares the low
word in A16, then the bank byte in A8 if needed. Null on either side is normalized
to the right. Captured stack temps and parameters use A16 `LDA home; ORA home+1`
against null: Z tests all three bytes, repeating the owned middle byte and never
reading a fourth byte. Other forms keep the low-word/bank short circuit. Both
paths finish in A16. The two-word reduction may cost more cycles for nonzero low
words; it saves code and mode changes. Full preflight checks the three-byte
extent before either word is emitted. ORA writes A/N/Z, preserves C/V, and reads
only its declared stack width. The operation barrier and the sole-Z consumer
make the changed A/N values private to selection. No source-memory access is
duplicated and no DP allocator eligibility is added.

Byte and pointer inputs may be captured stack temps/parameters or representable
literal/null/absolute-address values. Full preflight validates both inputs and
the BYTE result home, including stack delta and the final bank-byte extent,
before emitting or changing state. Mutable parameters use their authoritative
frame homes. Direct symbolic addresses, DP-resident narrow operands, mixed
widths, signed BYTE ordering and pointer ordering retain their prior paths.
New materialized predicates write exactly one canonical
0/1 through two outcome arms. Allocation, ABI return extension, source-memory
ordering, all 64 clobberable DP scratch bytes and every guard remain unchanged.

Four-byte LONGCARD/LONGINT Eq/Ne compares captured low words in A16, then high
words only if the low words match. Signedness does not affect equality and adds
no sign bias. U32 zero on either side is normalized to the right; both word
loads use Z directly without CMP-zero. Both halves are checked independently,
including byte 3 and transient S movement. Exact four-byte stack temps and
authoritative parameter homes, plus U32 constants, are eligible. Narrower
constants/homes, direct symbolic addresses and DP operands retain their previous
paths. Preflight checks both operands and the one-byte
destination before any emission or state change. No half-word temp identity or
persistent forwarding witness is introduced. The existing operation barrier,
source-memory accesses, allocation and guards remain intact. A materialized
result uses the existing two canonical BYTE outcomes; source reads still capture
all four bytes before private-home comparisons can short-circuit.

Signed four-byte `< 0`/`>= 0` and their reversed forms may inspect only the top
byte of a complete captured input. Both four-byte operands and the one-byte
result home are preflighted first. Materialization compares that byte with $80,
loads zero without changing carry, and uses ADC-zero to obtain canonical 0/1;
nonnegative tests invert that bit. A sole-use branch consumes the top byte's N
through BMI/BPL, restoring A16 without changing N before edge dispatch.
The full external capture remains, including volatile reads and reads around
calls. A retained constant-widening temp uses general ordering instead of this
sign-only specialization.
See the [ordering plan](MIR65816_LONG_ORDERING_PLAN.md).

Unsigned four-byte ordering compares captured high words in A16, then low words
only if the high words match. `>` and `<=` swap the private operands before
selection; the resulting C drives BCC/BCS for both materialization and fused
branches. Both complete operands and the BYTE result home use the same preflight
as sign tests. No external capture is shortened or reordered, and no scratch,
frame or DP allocation is added.

Other signed four-byte ordering subtracts the low words with SEC/SBC, then
loads the left high word without changing carry and subtracts the right high
word with the propagated borrow. BVC/EOR-$8000 corrects the final high-word N
for signed overflow. BMI/BPL makes the normalized `<`/`>=` decision; `>`/`<=`
swap captured operands first. This includes signed `<= 0` and `> 0`, which need
both halves. The same full preflight, canonical BYTE outcomes and branch-use
proof apply. No subtraction result is stored and no extra scratch is reserved.

A final eligible byte, word, pointer or long Compare followed immediately by
Branch may consume C/Z flags (or corrected N for signed words/longs) directly when a
routine-wide use proof establishes exactly one use: that Branch condition.
Other block conditions, edge arguments, returns and all
operation inputs (including addresses and indirect calls) disqualify fusion.
No flags cross an intervening operation or block. Both edge trampolines retain
parallel copies, typed JML fixups and A16 successor state, even for equal
targets with different arguments. Calls and source-memory operations stay in
place. Unsupported or nonadjacent pairs use ordinary materialization/branching.
The Boolean's home is still validated and reserved, with unchanged allocation,
storage maps and stack guards, but no 0/1 is written for an eliminated branch-only
value. Each executed fusion removes exactly one Boolean stack write and reload;
word reads and edge-copy traffic are unchanged. No value resides in flags or DP
across a call or another MIR operation. Preemption must preserve live A/P through
the adjacent load, comparison/correction and conditional/JML sequence. A signed
fused pair records exactly one final BMI/BPL dispatch in its MIR span; the
internal overflow branch remains part of the selected CFG and relocation map.
Fused pointer or long inequality can record two conditional dispatches to the
same true edge, one after each
part; both belong to the fused MIR span and use the existing layout/fixup rules.
Pointer/long equality's early low-word mismatch skips the bank/high-word decision. The
[byte/pointer measurements](benchmarks/65816-byte-pointer-comparisons/README.md)
and [long measurements](benchmarks/65816-long-equality/README.md) record
raw/optimized execution, exact access traces and interrupt qualification.

An A16 ABI return may load a U8/U16 immediate or an exact two-byte stack
temp/parameter directly into A16. It reuses the complete-word displacement
checks and explicit-cast rules above, including authoritative mutable-parameter
homes. Preflight failure changes no return-preparation bytes or mode knowledge;
legal unsupported operands retain generic return preparation. Other result
homes retain their defined high-bit guarantees. X is unspecified for A16 results
and is not cleared by this path. Both paths use the same frame teardown and RTL;
nonzero-frame teardown preserves A through Y. Selected preparation uses no DP
scratch, push, helper, or assumption about a preceding operation's register value.

An authoritative `NativeResult(A8ZeroExtended)` return with a typed U8 constant
loads that constant directly with A16 `LDA #$00xx`. The full-width load defines
both A bytes, including hidden B after an A8 predecessor. X is unspecified for
BYTE results and is not initialized. This path uses the same A-preserving frame
teardown and RTL, without DP result scratch, additional storage or memory reads.

An exact BYTE stack temp or parameter uses an A8 load followed by A16
`AND #$00FF`. The existing BYTE classifier preflights typed width and the
one-byte displacement, including stack delta and authoritative mutable-parameter
homes. The read never includes a neighboring byte. This path uses the same
terminal-boundary mode restoration, frame release and RTL; it neither reads nor
writes result scratch and does not initialize X. Unsupported operands/homes
retain generic preparation. Source loads and call/alias/volatile ordering remain
separate MIR operations. See the
[captured BYTE return contract](MIR65816_CAPTURED_BYTE_RETURNS.md).
An authoritative `A16X8ZeroExtended` or `A16X16` return may prepare exact-width
numeric constants/null/addresses with X16/A16 immediates, or load a captured
private temp/parameter directly into A/X. Validate the complete home and stack
delta before emission. Mutable parameters use their current frame home.
The 32-bit path loads the high word into X and then the low word into A. The
24-bit path reads words at home+1 and home, using XBA/AND to zero-extend the bank
byte in X; both reads stay inside the three-byte home. No result-scratch writes
are needed. Source-memory reads and their ordering remain separate operations.
Symbolic addresses and width mismatches retain existing preparation. Shared
frame release preserves both result registers. See the
[native wide return contract](MIR65816_WIDE_RETURNS.md).

MIR65816 owns these target-specific preparations; the public ABI, guards and
SemIR/NIR contracts are unchanged.

MIR65816 owns access-width and addressing selection. Ordinary scalar loads and
stores may use sixteen-bit transfers plus a final byte. Three-byte transfers
between disjoint frame slots (or the same slot), and from a frame slot into
owned direct-page pointer scratch, may instead use overlapping words at offsets
zero and one. This touches no fourth byte; overlapping word transfers are never
used to duplicate external or indirect accesses. Volatile loads and stores keep
their exact ascending byte accesses. The
[repeated-access eligibility audit](MIR65816_REPEATED_ACCESS_ELIGIBILITY.md)
records the missing extent, observability, alias and concurrency proofs;
nonvolatile external memory is not automatically eligible.

Small indirect field displacements are carried by `[pointer],Y`, including bank
carry. Displacements that cannot accommodate a four-byte scalar within Y use
explicit full-width pointer addition. Address formation and aggregate copies
materialize any deferred displacement before consuming the pointer itself.

An adjacent native `LDY #0; LDA/STA [pointer],Y` may become `LDA/STA [pointer]`
through the closed zero-index rewrite. Both Y lanes must be dead after the
access. Loads establish the same A/N/Z; stores additionally require dead N/Z,
because their original flags came from LDY. The proof retains the complete
indirect access, its width, ordering and barrier, including uncertain writes.
It grants no general permission to remove memory definitions or cross compiler
events, control-flow entries or environment changes. The checked driver
revalidates each generation, replays selection and rebuilds branch layout
before publication. Allocation and volatile access traces are unchanged.

Accumulator-width knowledge is local to emitted instruction sequences and is
discarded at labels. Scalar operations may retain their final width; calls and
MIR control-flow boundaries restore sixteen-bit A. Procedure frame teardown
does not preserve an unused accumulator result. These choices change neither
the public ABI nor NIR memory effects, and do not allocate persistent values in
call-clobbered scratch.

Same-width three-byte casts from captured temps or parameter homes use the
existing private overlapping-word transfer when source and destination are
identical or disjoint. Both complete homes are checked before emission,
including the current stack delta and the owned DP extent. The selector keeps
the semantic cast and allocated result, retains the operation barrier, and
selects A16 without an intervening A8 excursion. When both checked stack homes
are identical, the cast emits no transfer or mode change; tracked state reflects
only the retained operation barrier. Partial overlaps, constants, symbolic
values and width changes retain their prior cast paths. No external access is
repeated and no fourth byte is touched.

`AddressOf` with a captured indirect base, no index and displacement zero uses
the same checked private transfer when its three-byte result home is disjoint
from the complete base home. Positive displacements through 65,535 instead use
A16 low-word addition followed by A8 bank addition with carry. Both homes are
fully preflighted before emission; the low-word store and mode change retain
carry, and the bank-byte result wraps modulo 24 bits. Address formation never
dereferences the pointer or stages it through DP. Symbolic/object bases, indexed
addresses, larger offsets and overlapping homes retain the general path. These
forms change neither address meaning nor the closed-operation allocation
contract, and make no atomic-update claim.

Typed three-byte Add/Sub by numeric one uses the same captured-home preflight
and native low-word arithmetic, followed by A8 bank carry/borrow. Addition also
admits one on the left. Complete identical homes are safe because the low-word
store cannot touch the bank source; partial overlaps retain the bytewise path.
Other constants, noncaptured operands and other widths keep their existing
selection. The result wraps modulo 24 bits. This is value computation, with no
dereference, new scratch reservation, external access reordering or change to
whole-operation interference. Normal tracked arithmetic effects invalidate the
accumulator and flags; calls and control-flow boundaries restore A16 as usual.

By-value aggregate interfaces, REAL, foreign code,
unresolved runtime/builtin calls and source terminal exits have explicit
diagnostics. Volatile aggregate copies are rejected: use a deliberate scalar
byte-access protocol for such hardware. Freestanding terminal faults are supplied
through the platform assembly interface.

Small-model emission is unsupported; its existing lowering/planning policy is
preserved. Source ORG/SET origins, fixed routine placement and top-level
executable statements require a separate startup/platform contract.

### Declaration-address limitation

The shared declaration-address resolver still uses 16-bit addresses. A bare
declaration such as `VOLATILE BYTE io=$F00000` can reach NIR as initialized data
instead of a hardware alias. The new driver rejects wide numeric bare
initializers in global and local declarations. Use bracketed initialized data
or explicit 24-bit pointer casts and accesses for banked memory. Bank-zero
absolute aliases are exercised by the volatile execution test. Generalizing
the shared declaration-address resolver remains outside the advertised initial
subset; banked MMIO uses explicit pointers.

## Allocation and stack checks

Temporary locations explicitly distinguish stack and direct-page homes. The
selector consumes a verified pointer-leaf plan when eligible. Its whitelist admits
only a single bounded block of ordinary three-byte pointer loads/stores and a
void return, with no indexes or calls. These operations may touch only their
allocated homes and addressed memory. The default sequence uses A/Y/flags and
closed def/use intervals, with three deterministic ABI slots (D+0, D+3, D+6).

If that assignment exceeds three slots, one bounded exception can avoid the
whole-routine stack fallback: a three-byte indirect load may reuse its dying
base's complete DP home. The verifier checks the exact defining load, last use,
widths, identities, slot ownership, all other live ranges and frame accounting.
Selection rechecks that exact operation before emitting any instruction. Calls,
volatile accesses, indexes, joins, non-pointer values and function results remain
outside the whitelist; unproved pressure still rejects the entire candidate.

The exceptional sequence captures the low word in X16, then the bank byte in
A8, before writing any destination byte. It writes the private bank byte first,
then transfers X to A16 and writes the low word. External accesses retain their
original low-word-plus-bank extent and order, including bank carry; no byte is
repeated. The closed leaf has no X-resident value, call or result, and selection
rejects a live loop-X contract or nonzero stack delta. Typed physical effects
account for TAX/TXA and both accumulator widths. Flags and A/X are scratch at
this boundary; all continuing MIR values reside in their checked homes.

The smaller existing sequence wins whenever closed intervals already fit.
This exception does not change generic whole-operation interference, stack/CFG
allocation or the repeated-external-access gate. Pointer-leaf homes remain memory
locations, and no bank-zero reservation is added.

Other routines use invocation-owned stack temporaries with CFG-aware lifetime
reuse. Backward fixed-point liveness includes indirect address bases, indexes,
call targets/arguments/results, returns, edge arguments and block parameters.
Inputs, outputs and values live across an operation interfere for its entire
instruction sequence, including dead outputs that selection still writes, with
one stack-only exception for a dying three-byte identity-cast input and result.
Block
parameters, even unused ones, interfere with each other and successor live-ins.
Required parallel-edge staging slots remain separate from all temporary homes
and frame objects. Empty/direct word edges contribute no staging; selective word
edges capture only endangered sources. A directly scheduled three-byte edge
reserves one two-byte A-save word unless all moves are identities, which need no staging.
Full word/byte fallbacks save every
argument. Each shared slot has the maximum actual width needed at that capture
ordinal, with multi-byte slots aligned evenly.
The allocator plans from a frame containing all private homes: immutable incoming
arguments lie above it, and adding staging only moves them farther from any
destination. The final allocation independently rechecks every plan, required
capacity, physical overlap, incoming last-byte bound and frame accounting. Cycles cannot destroy successor live-ins.

After the initial verified stack allocation, one deterministic affinity pass may
move non-parameter word temporaries onto compatible edge destination homes.
All block-parameter homes stay fixed. Each directly scheduled word edge proposes
its compatible source changes simultaneously; conflicting repeated-source
proposals reject the transaction. The unchanged whole-routine closed-operation
verifier must accept every resulting home and exact frame/staging accounting.
Only transactions reducing copy cost without increasing any edge's bytes or
cycles are accepted. Profitability includes final A/N/Z repair. Rejected trials
leave the original allocation intact. Frame compaction and relaxed arithmetic
interference are separate work. See the
[coalescing plan](MIR65816_EDGE_COALESCING_PLAN.md).

A subsequent bounded pointer-cast affinity pass can place a dying captured
three-byte cast input and its bit-preserving result in the same stack home.
The liveness exception omits only that operation's pair when the input is absent
from the live-after set; interference established elsewhere is never removed.
The stack verifier independently checks every third-party interference and
requires cast homes to be completely identical or disjoint. Partial overlaps
remain illegal. All edge argument and block-parameter locations remain fixed.
Trials must increase the total number of identity transfers and retain exact
frame, stack-peak and staging accounting. Source captures, volatile accesses,
calls and DP reservations remain unchanged. No frame compaction or DP cast
coalescing is included. See the
[pointer/bitwise measurements](benchmarks/65816-pointer-bitwise/README.md).

A bounded scalar loop may keep one unsigned word header parameter mirrored in
X16. An immutable typed plan requires a call-free scalar-DP routine, one simple
loop, a sole-use immediate unsigned comparison, one `p + 1` update, and `p` as
the final assignment on both incoming word-copy edges. The parameter and update
retain separate interfering memory homes. Every store remains authoritative.
TAX follows each completed incoming schedule; CPX immediate consumes the mirror
in the fused branch. The bounded `p + 1` update uses INX/TXA when the rest of
the body has no internal comparison dispatch; otherwise TXA can replace only
the input load. INX keeps the distinct result store and all edge copies. A checked `<= K`
normalization uses `K+1` and rejects `$FFFF`. Unsupported candidates retain the
ordinary selector before any bytes are emitted.

The tracker carries only the declared X/home relation across checked CFG joins;
ordinary A/Y/flag/home witnesses still stop at labels. A store overlapping the
home invalidates the relation until the final TAX. INX also invalidates it
immediately, before memory changes: X then holds the update result. Only the
checked following TXA can consume that result; no pending relation may cross
a label or authorize CPX, another INX or a load of the old parameter. The
whole-routine scalar whitelist proves C/V are not MIR outputs of this ADD and
that admitted flag consumers establish their inputs. Actual INX effects still
preserve C/V; TXA supplies the result with word N/Z. Every emitted instruction
must preserve the reservation or fail its proof. Region exit releases it; no
binding crosses a call, helper, unknown write, D/S change or index narrowing.
CPX updates C/N/Z using index width and leaves A/X/Y/V intact. Its flags need
only preserve the fused branch truth; TXA and TAX preserve the selected input
and completed edge A/N/Z contracts. Stack guards, home maps and public ABI are
unchanged. See the [bounded X plan](MIR65816_LOOP_X_RESIDENCY_PLAN.md).

Only MIR value temporaries share storage. Frame objects, addressed locals and
mutable parameters retain their dedicated homes. No temporary address escapes,
and no alias-sensitive load forwarding or memory reordering is performed.
Values live across calls and helpers remain on the invocation's stack, outside
call-clobbered registers and DP scratch. Allocation is deterministic (descending
width, then interference count, then ID; first available aligned byte range),
and is rechecked against liveness, byte extents, frame objects, staging slots
and final accounting before selection. It need not find the minimum frame.
Both raw and optimized emission use allocation; `--no-opt` controls NIR passes.
See the [measurements and scope](MIR65816_TEMPORARY_ALLOCATION.md).

The allocated even fixed frame must fit 254 bytes. Incoming offsets are
recomputed after allocation. Every emitted stack-relative byte access is
checked against `1..255`, including accesses to argument values after reserving
outgoing space. No displacement is truncated.

At entry, emitted code checks the frame reservation against the current
domain's stack floor and ceiling. Before a call it checks `O + 3` (direct) or `O + 6` (indirect), then
constructs O bytes using either reservation/stores or checked argument pushes
under the contract above. Argument pushes replace the outgoing reservation;
they add no temporary stack peak beyond O and the declared transfer.
The source memory helpers use ordinary checked calls. Indirect calls capture the callable before PHK/PER and the
stack-synthesized RTL transfer; decrementing the target PC does not borrow
from its bank. Same-bank PER continuation/range checks run after placement.
Caller cleanup and frame release preserve A/X through the specified Y-based
sequence. Byte results zero A's unused high byte; 24-bit results zero X's high
byte.

`AllocatedFrame` and image routine metadata report the final fixed frame,
spill bytes and exact **local** reservation/transfer peak. This excludes the
callee's own checked reservations and the platform's interrupt headroom. It is
not a whole-task bound, particularly with recursion. The earlier abstract MIR
plan remains explicitly unallocated.

The loader/platform must establish native mode, M=X=0, decimal clear, DBR=0,
an aligned per-domain direct page, even entry S and valid v1 arguments/return
bytes. It must reserve task headroom `26 + nmi_extra_stack` or IRQ headroom
`13 + nmi_extra_stack` when initializing the domain's floor. Emitted checks
preserve I. Failure transfers by JML to the configured nonreturning
`__a816_stack_overflow_v1` adapter with required bytes in A, unchanged S in X,
and S unchanged. The platform assembles the separate
[context bridge](MIR65816_CONTEXT_INTERFACE.md) for task entry, IRQ, COP and NMI;
reset/startup and board-specific vector installation remain platform work.

## Images, placement and assembly

`actionc-65816-image`, version 3, contains initialized segments, separate
zero-fill regions, exports, data symbols, assembly imports and the platform
stack contract. The ABI and target identities are checked when loading JSON.
The platform loads declared regions and calls the exported program entry.
External address/alias declarations do not allocate or clear memory.
Version 1 and 2 images must be recompiled; the physical ABI remains v1.
Temporary maps contain `id`, `size` and a tagged `home`: either
`{"kind":"stack","displacement":N}` or `{"kind":"direct_page","offset":N}`.
Stack homes are checked against the allocated frame; DP pointer homes must
occupy one of the three owned ABI slots and cannot coexist with calls. DP
values are not stack spills. Multiple temporary IDs can share stack bytes;
their individual widths remain exact and spill bytes count physical extent,
not the sum of temporary widths. Lifetime and scratch-clobber proofs are checked
against typed MIR before selection, not inferred from the final map.

Optional `read_only_origin` and `zero_fill_origin` layout fields independently
place immutable data/templates and wholly zero-filled writable objects. When
omitted, they share `data_origin`. An initialized object with a zero-filled
tail stays contiguous. `ImageEnd` uses the highest allocated end. The platform
reserves bank-zero stacks/domains separately and checks cross-allocation overlap.

Each emitted routine stays within one code bank, leaving the bank's last byte
unused. The linker advances to the next bank when necessary and rejects a
routine larger than that placement strategy permits. JSL calls and JML edges
are relocated with full addresses. No continuation relies on PC bank wrapping.
Out-of-range relocations, unresolved symbols, cyclic/out-of-bounds aliases and
overlapping image/import regions are rejected.

Declare assembly services with ordinary `PUBLIC EXTERNAL` Action! interfaces
in a module. Discover referenced interface identities and argument layouts:

```sh
cargo run --locked --bin actionc-65816 -- --emit-interfaces program.act
```

Add an entry to `imports` for each required service:

```text
symbol          runtime interface ID from --emit-interfaces
signature       structural signature ID from --emit-interfaces
abi             "action65816.native.v1"
address, size   actual assembled code range
stack_peak      assembly's local reservation peak below its entry S
checks_stack    true: assembly performs its own required reservation checks
irq_effect      "preserve" (default), "save_disable" or "restore"
```

ABI/signature mismatches and missing bindings fail linking. Imports are
ordinary returning v1 routines, with conservative call effects and the current
execution domain. Their stack declarations are platform obligations, not
proofs obtained by disassembling their bytes. The IRQ primitives are explicit exceptions to ordinary I preservation:
`save_disable` requires zero arguments and a BYTE result; `restore` requires a
single BYTE argument and no result. Incompatible signatures fail linking. All
source calls remain conservative memory barriers; these declarations do not
relax aliasing or optimizer ordering. See the context interface for stack costs.

The compiler exports argument offsets/widths/alignment and body displacements,
result width, code address, signature identity, allocated frame objects and
temporaries, outgoing call/transfer costs, and the local stack peak.
`whole_task_stack_bound` is null. Image verification checks map extents and cost
consistency; it is not a verifier of arbitrary replacement machine bytes. Names
are display metadata. The independent assembly fixture hand-packs the published mixed
example and consumes only exported code addresses; it does not derive expected
offsets or results from compiler layout helpers.

## Disassembly and executable evidence

```sh
python3 tools/disassemble65816.py build/scalar.a816.json > build/scalar.asm
```

This disassembler reads emitted routine bytes, tracks their explicit M/X width
changes and rejects unknown/truncated encodings. Imported assembly remains
external; retain its ca65 listing and symbols.

[`tools/native65816-runtime-tests`](../tools/native65816-runtime-tests/README.md)
loads serialized compiler images into the pinned native VM with the qualified
status-timing patch. Handwritten callers/callees are assembled with ca65/ld65.
Memory regions, instruction/cycle budgets, guards and expected values are
explicit. Both raw and optimized NIR are exercised.

The [acceptance result](MIR65816_EXEC_ACCEPTANCE.md) records all 24 native tests,
the independent CPU corrections, G1–G6 evidence and remaining platform limits.
The [implementation plan](MIR65816_IMPLEMENTATION_PLAN.md) records the separately
committed slices. New operations, helpers, ABI changes or wider nesting policies
require corresponding execution qualification.

## Scalar DP word locations

Image v3 and experimental o65 profile v1 retain their existing tagged temporary
locations. In addition to the three size-three pointer slots, validators admit
size-two scalar homes at even offsets $20..$3E, only in routines without calls.
Invalid widths, out-of-pool/odd offsets and mixed pointer/scalar allocation
families are rejected. Maps describe geometry; the allocator separately proves
closed-operation liveness and conservative selector effects before emission.
Older validators require an update to consume these scalar maps.

Native word instructions use literal D-relative operands. o65 allocates no
application scratch in its zero segment and adds no relocation for these
operands. Public arguments/results, stack-guard algorithms, interrupt reserves
and physical ABI v1 are unchanged. Scalar frame shrinkage changes reservation
and incoming-argument operands, and zero-frame routines use the existing short
return. Exec816 adoption and its pinned compiler remain a separate task.

### Read-only physical home analysis

The selected snapshot retains native entry/ownership facts for canonical
invocation-entry-relative stack bytes and current-domain DP bytes. Backward
may-liveness composes ordered accesses; unresolved aliases and calls remain
conservative. Queries validate owner, generation and reachability. These facts
do not change selection or grant rewrite permission. See the
[home analysis contract](MIR65816_HOME_ANALYSIS.md).

Stored-definition queries identify each physical byte/write site, attribute
ordered reads and preserve undefined paths and may-write uncertainty. Their
outside-window proof is restricted to private homes and checked straight-line
windows; replacements still require independent validation of local reads and
machine effects. See the [stored-definition contract](MIR65816_HOME_DEFINITIONS.md).

### Read-only machine liveness

The same selected snapshot computes backward A/X/Y lane and independent N/Z/C/V
liveness, using central effects and native result boundaries. Queries validate
site ownership and reachability. Environment operations, X reservations and
forward witnesses remain separate protected obligations; deadness does not
authorize their removal. See [machine liveness](MIR65816_MACHINE_LIVENESS.md).

### Symbolic address materialization

Direct symbolic AddressOf writes three independently relocated immediate bytes
to its checked stack home without staging through pointer scratch. It retains
the stable data target and complete addend on each byte fixup, and checks the
entire destination before emission. Direct symbolic places preserve their
existing checked-addend behavior. Indirect symbolic values currently admit
only zero displacement; nonzero modular arithmetic needs an extent proof.
Captured pointers, unsupported homes and indexed forms retain their existing
selection. No pointed-to byte is read, and no fourth destination byte is touched.
See the [address-selection plan](MIR65816_ADDRESS_SELECTION_PLAN.md).

Before selection, a per-routine plan follows same-block AddressOf chains to
allocated data identities. Constant stride-one indices and displacements fold
only to interior object offsets, where every valid placement is nonwrapping.
One-past, alias/absolute geometry, loads, calls and cross-block provenance keep
their established paths. The plan counts every MIR operand occurrence, including
terminators and edge arguments, before omitting a pure producer whose consumers
all use symbolic replacements. Prepared MIR and frame allocation stay intact;
omitted operations retain empty source spans and normal replay bookkeeping.

The same plan selects A8 long loads/stores for ordinary BYTE accesses at proven
interior symbol offsets. Each replacement retains exactly one target-byte access
and the original value capture/order. Store sources are byte immediates or
complete captured stack homes. Volatile accesses keep their existing selection;
contents are never inferred from an initializer or reused across calls.

Ordinary stride-one BYTE loads/stores with zero residual displacement may use a
captured unsigned 16-bit index in Y16. Eligibility reads the MIR temp's integer
type, not just its width. The base is a complete stack-held pointer or proven
symbolic address, materialized in existing PTR scratch before loading the
complete index in A16 and transferring it to Y. The single A8 long-indirect
indexed access preserves 24-bit carry/wrap. Capturing a load result does not
alter Y. Stores admit only an immediate BYTE or a complete captured stack byte;
loading that source after TAY cannot change Y or the prepared PTR. The original
source captures retain their order and the target receives exactly one write.
Signed/wide/scaled indices, volatile accesses and unsupported homes retain the
fallback. No allocator whitelist or physical ABI changes are implied.

Bounded three-byte source bindings are defined in
[MIR65816_POINTER_FORWARDING.md](MIR65816_POINTER_FORWARDING.md). They omit only
proved private captures, retain allocated writable homes, and substitute checked
authoritative source reads at explicit consumer sites. They do not manufacture
home definitions or reuse the incoming/frame word A/N/Z witness.

### Bounded BYTE indexes

The address selector may zero-extend a verified unsigned BYTE index into A16
and use Y for a stride-one BYTE load/store. The index is read at its exact
one-byte width; hidden B is cleared explicitly. The complete runtime offset
must fit 16 bits for all 256 index values. The constant displacement is added
to Y, while the captured or checked symbolic base retains its full 24 bits.
Bank carry and bus wrap occur through native long-indirect indexed addressing.

Index and payload homes are preflighted before emission. Nonvolatile accesses
retain their external access count/order, and unsupported widths, signed indexes,
partial homes or offset overflow retain generic lowering. Captured pointer
bases use the current source resolver, including bounded pointer bindings;
they never silently reload an omitted capture's former home.

Power-of-two strides and 1–4-byte payloads use the same proof, extended to
`max_index * stride + displacement + payload_bytes - 1 <= 65535`. BYTE scaling
uses A16 accumulator shifts; payloads use the existing nonvolatile transfer
policy: full words followed by an exact odd byte, with ascending `[pointer],Y`
accesses and checked Y increments. No fourth byte of a three-byte payload is
touched. Narrow literal stores retain zero extension, including NULL.
The complete payload stays within its verified home and store preparation
cannot overwrite pointer/index scratch. CARD indexes retain only their existing
stride-one BYTE case because the complete wider offset range does not fit Y.

Typed ASL A reads/writes the active accumulator width, writes N/Z/C and preserves
V; typed INY reads the active index width, writes Y and N/Z, and preserves A/X,
C/V and the native environment. Encoding, tracked values, physical effects and
fresh selected replay share these forms. Neither requires a raw opcode path.

Other positive constant strides use A16 binary shift/add, retaining the original
zero-extended index in the existing INDEX scratch word. Each prefix coefficient
is at most the admitted stride, so the complete offset bound also proves every
intermediate. Each ADC establishes carry independently. No multiplication helper,
new reservation, alias permission or live value across a call is introduced.
A conservative cost comparison includes index extension, displacement, mode and
base-preparation differences; candidates that are not smaller keep generic
lowering. Zero strides and offsets exceeding the complete Y range are refused.

### Native unsigned integer casts

Integer-kind casts from captured unsigned 2/3/4-byte values may copy private
stack homes with A16 pieces and zero extension. Both complete extents and actual
source bindings are checked before emission. Destinations use their own writable
homes. Disjoint homes and safe same-start transfers are admitted; partial overlap,
signed sources, pointer-reinterpretation kinds and unsupported locations retain
their existing paths. Private three-byte copies may overlap their own word
pieces at offsets 0 and 1, touching no fourth byte.

Selection compares complete instruction costs, including entry mode requests
and restoration of the byte fallback's A8 exit. It requires a strict saving.
Same-start copies omit unchanged payload bytes, but still write any required
extension zeros. No frame, lifetime, alias or ABI contract is weakened.
