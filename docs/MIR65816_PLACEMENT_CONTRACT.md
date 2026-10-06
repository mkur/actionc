# Native 65816 operation resources and placement

MIR65816 owns operation resource requirements, value placement and transfers.
The immutable [logical routine analysis](MIR65816_LOGICAL_ANALYSIS.md) supplies
definitions, complete uses, widths, representations and CFG facts. Selected
instruction effects remain authoritative for physical reads, writes, clobbers
and execution state. Neither resource descriptions nor placement grant new
memory aliasing or instruction-rewrite permissions.

## Planning and ownership

The routine placement plan constructs operation descriptions before allocating
homes. It owns the existing storage-demand decisions, final allocation, checked
pointer and scalar read bindings, accumulator intervals, redirected local loads,
deferred address components, adjacent assignments and the bounded X mirror.
Selection consumes these admission results instead of reconstructing competing
home-demand or residence plans. Existing selectors implement their qualified
forms; block-local mixed residence extends their checked input/home choices.

The plan borrows one immutable MIR routine and data table. Verification
reconstructs admission and value/resource obligations from these inputs and the
final allocation. It checks the complete logical operand census, including
unreachable occurrences, pointer bases, indices, indirect targets, block
parameters and distinct parallel edges. Invalid plans fail compilation; a
profitability refusal still selects the existing conservative strategy.

Each temporary has one capture owner and an explicit read location for every
logical occurrence. A canonical allocated home is separate from a borrowed
input, register interval, deferred computation or redirected destination.
Accumulator intervals name their producer and consumer and the complete A8,
A16, A16/X8 or A16/X16 payload lanes. A three-byte deferred address expression
does not promise a live A/X value at its omitted source operation.

Borrowed pointer aliases retain separate definitions and use sets. Their exact
authoritative input homes come from the existing complete, closed read-binding
proofs; numeric slot coincidence cannot authorize a binding. Loop writes with
unknown logical storage versions do not become memory-equivalence proofs.
The existing bounded borrowing rules separately reject intervening writes,
escapes, partial accesses, volatility and call crossings. No field read is
reused because a pointer identity survived.

## Resource windows

An operation window includes address preparation and its complete access or
computation. Its description bounds register reads/writes/clobbers, flag
dependencies, protected state changes, scratch bytes and additional stack use.
Routine-local dense window IDs form a bijection over operation points. Complete
rows retain explicit empty masks, a shared immutable resource description and
the compound-access owner. Verification reconstructs the canonical tables and
rejects holes, duplicate IDs and invalid references. Instruction checking reads
the immutable row once per source window and still derives actual typed effects
for every instruction.
Register bounds are deliberately conservative whole-operation bounds, including
the hidden accumulator byte and index high lanes; they are not smaller live
ranges or grants to retain unrelated registers through an operation.

Unindexed nonvolatile scalar/record accesses, basic scalar arithmetic,
comparisons, casts and address formation have qualified descriptions. Indexed,
aggregate, volatile, other arithmetic and call forms are explicit resource
barriers. These barriers preserve their current selectors and ABI checks and
cannot justify extending a residence interval. Helpers and terminal forwarding
remain opaque under their dedicated independently recomputed contracts.

Ordinary selector workspaces occupy the existing domain scratch from `$80`
through `$9E`; scalar and mixed pointer residence occupies `$A0..$BF`. Pointer leaves use their
existing three-byte slots inside the same scratch reservation. The shared
workspace constants define both selection operands and resource bounds. A
description never initializes scratch. Calls retain the native ABI clobber of
all compiler scratch, registers and flags; invocation homes must preserve any
capture used afterward.

Physical scratch overlap uses complete byte ranges, not temp names or numeric
stack/DP offset equality. A resource window cannot write a protected live DP
value. Declared output and simultaneous edge destinations are checked against
the existing closed-operation interference, pointer identity/reload and
overlap-safe edge-copy proofs. The pointer reload exception still requires
complete capture before overwriting its source base.

External scalar/record reads and writes have their exact declared byte extent.
An admitted adjacent assignment has one compound access owner spanning its
load and store: the current selector may emit both accesses in the store span.
The verifier counts authoritative typed memory effects across that complete
window. This check does not claim an address-equivalence or alias proof; typed
address selection and existing execution oracles retain those obligations.

## Boundaries and transfers

The plan records every logical edge ordinal, simultaneous value binding, full
source/destination width and memory home, plus allocated transfer staging.
Existing word, pointer and mixed copy schedules retain their allocation checks;
the new plan does not substitute sequential assignment for parallel copies.

Block labels bind only after the complete plan verifies. Selected predecessor
counts and reachable entries must match the logical CFG, including parallel
edges and late backedges. Every source operation starts in the native domain,
with decimal clear, word indices and the allocated body stack anchor. Block
entry also requires word accumulator mode. The existing tracked facade checks
actual edge and return state, stack balance and all X refresh obligations.
The X mirror contract must agree exactly with the common plan and every checked
predecessor; its canonical home remains authoritative.

Pointer-stage requests must use the exact input identity and complete home
admitted at that source site. Selected calls cannot conceal additional stack
use: typed state observations must stay within both the operation's outgoing
and transfer allowance and the allocated routine peak.

## Selection, replay and artifacts

After selection, a sealed immutable contract checks actual typed effects,
resource interference, source-window coverage, external access extents and
boundary obligations. It binds to the same routine and final allocation.
Replay regenerates actions and effects through a fresh tracked emitter and
rechecks the placement contract. Final instruction rewrites and layout retain
the contract and recheck it during reconciliation. The contract is shared
immutably across selected generations of the same compilation and allocation.
Another instance cannot reuse it even if its numeric routine ID and homes match.
Mutable input changes require a new logical analysis and plan.

These checks complement the existing physical home, machine-state and rewrite
proofs. They do not introduce a new raw-byte analyzer or trust selector-provided
effect masks. The instruction definition that drives emission and replay also
supplies the checked resource observations.

Image v3 retains its tagged stack/DP home representation. A DP-only interval has
its complete, actual DP home in the map; a stack-backed cache retains the stack
home in the map. Internal read locations are never serialized as fabricated
homes. Frames and local peaks follow actual stack demand; the ABI scratch
reservation does not grow. Transport validation checks complete pool geometry,
while compilation proves lifetimes and resource ownership. A routine can contain
calls outside a DP-only value's complete lifetime. The opt-in proof interface
exposes checked counts of complete DP homes and stack-backed residence intervals.

## Mixed block-local consumers

The common plan admits complete two-byte scalar and three-byte captured pointer
values in ordinary mixed blocks. Definitions and every use come from the typed
operand census. A DP-only value has one operation definition and all reads in
one closed interval within the same reachable block. Unreachable definitions
cannot establish residence. Block parameters, terminator operands,
edge values, indexes, unsupported consumers and resource barriers retain stack
homes. No residence crosses a block boundary or a call. Existing closed scalar
and pointer-leaf allocations retain their dedicated admissions.
The existing fused top-bit selector also retains its mask region and required
stack captures; mixed residence must not change that region's ownership.

Whole operation input/output extents coexist, including the final bank byte.
Aligned intervals are colored deterministically into the existing 32-byte
residence pool. Pressure refuses only the affected interval. Verifiers rebuild
these intervals and exact locations from immutable MIR; missing or forged
intervals, sizes, transfers and scratch geometry are errors.

Trial frame allocation includes actual edge-copy staging and incoming argument
geometry. If the mixed choices increase fixed extent or local stack peak, the
routine retains its ordinary allocation. Removing homes must not conceal new
parallel-copy cycles or a larger reservation.

A captured pointer with later reads beyond its local interval keeps its
invocation home. A profitable prefix receives a complete private stack-to-DP
capture after its producer. Selected transfer requests name the exact temp,
source home, destination and producer point, and must appear exactly once.
Replay reconstructs the complete transfer through the authoritative typed
instruction boundary. Internal reads use the DP copy only within that interval;
barriers and later blocks read the invocation home. The cache does not extend
memory-equivalence facts through writes: every source field read/write remains
at its original operation with its original extent and order.

The size-first admission counts changes of prepared address identity rather than
repeated uses of an already prepared base. A backed pointer needs at least three
proved preparations to cover its private copy and mode cost. Complete DP pointers
avoid preparation from a stack capture. Native DP words retain equal-size
instructions; narrow scalar field captures may use the existing checked adjacent
accumulator producer/consumer contract. BYTE widening clears hidden B explicitly.
Unsupported wide arithmetic/casts and indexed forms retain their existing
strategies. Shared NIR promotion legality and profitability policy are unchanged;
these consumers use already available typed values.
